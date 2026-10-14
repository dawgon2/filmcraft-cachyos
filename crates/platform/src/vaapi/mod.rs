//! Hardware decoding through VA-API (Linux: Intel, AMD and other Mesa / vendor drivers).
//!
//! VA-API decoding is stateless: the application parses the stream and keeps the decoded picture
//! buffer, and the GPU decodes one picture's slice data at a time into a surface. [`h264`] does the
//! host side with the software decoder's own parsers and DPB (so the two decide alike), [`va`]
//! drives libva (loaded at run time; no libva means no hardware decoder, never a failure to start),
//! and [`VaDecoder`] puts them together as a [`VideoDecoder`]. H.264 8-bit 4:2:0 progressive
//! (Constrained Baseline / Main / High) and HEVC 4:2:0 (Main, Main 10, Main Still Picture, and
//! range-extension-profile streams of those formats), the latter through [`hevc`] the same way;
//! fork U0.2.3 adds HEVC Main 4:2:2 10 on the zero-copy accelerator. Everything else stays in
//! software.
//!
//! The FFI is in [`ffi`] (declarations) and `va` (calls); the rest of this module is safe code.

pub mod ffi;
pub mod gray;
pub mod h264;
pub mod hevc;

#[cfg(test)]
mod abi_tests;
#[cfg(test)]
mod tests;

#[cfg(target_os = "linux")]
pub use linux::{NAME_H264_ZERO_COPY, NAME_HEVC_422_ZERO_COPY, NAME_HEVC_ZERO_COPY, VaDecoder, annexb_hevc_info, decode_annexb_with, decode_annexb_zero_copy};

use filmcraft_frame::VideoFrame;

/// The hardware a front end ([`h264::Front`], [`hevc::Front`]) drives: a fixed set of surfaces,
/// picture decoding into one of them, and reading one back or filling it.
pub trait Accel: Send {
    /// The VA surface ids of the decoder's surfaces; front ends refer to them by index.
    fn surfaces(&self) -> &[ffi::VASurfaceID];
    /// Decode one H.264 picture (its slices: parameters and the NAL unit, header byte and
    /// emulation prevention included) into surface `target` (an index into [`Accel::surfaces`]).
    fn decode_h264(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferH264,
        iq: &ffi::VAIQMatrixBufferH264,
        slices: &[(ffi::VASliceParameterBufferH264, Vec<u8>)],
    ) -> std::result::Result<(), String>;
    /// Decode one HEVC picture (its slice segments: parameters and the NAL unit as stored) into
    /// surface `target`; `iq` when scaling lists are enabled.
    fn decode_hevc(
        &mut self,
        target: usize,
        pic: &ffi::VAPictureParameterBufferHEVC,
        iq: Option<&ffi::VAIQMatrixBufferHEVC>,
        slices: &[(ffi::VASliceParameterBufferHEVC, Vec<u8>)],
    ) -> std::result::Result<(), String>;
    /// Decode one HEVC picture of a range-extension profile (Main 4:2:2 10) into surface
    /// `target`: the picture and slice parameters with their range-extension parts, otherwise as
    /// [`Accel::decode_hevc`]. Fork U0.2.3; the default refuses (the read-back decoder makes
    /// 4:2:0 surfaces only).
    fn decode_hevc_rext(
        &mut self,
        _target: usize,
        _pic: &ffi::VAPictureParameterBufferHEVCExtension,
        _iq: Option<&ffi::VAIQMatrixBufferHEVC>,
        _slices: &[(ffi::VASliceParameterBufferHEVCExtension, Vec<u8>)],
    ) -> std::result::Result<(), String> {
        Err("range-extension pictures are not decoded here".into())
    }
    /// The picture in surface `index`, once decoded.
    fn read(&mut self, index: usize) -> std::result::Result<VideoFrame, String>;
    /// The surface the next picture decodes into, among `candidates` (indexes into
    /// [`Accel::surfaces`] the DPB does not hold, ascending); `None` when there is none. The
    /// default takes the first. An accelerator whose surfaces stay in the frames it hands out
    /// (zero-copy display, fork Y1) skips the ones a frame still holds and may add a surface
    /// instead; indexes of existing surfaces never change, so the DPB's references stay valid.
    fn pick(&mut self, candidates: &[usize]) -> Option<usize> {
        candidates.first().copied()
    }
    /// The surface a stand-in for a missing reference is made in ([`Accel::fill`] with `None`),
    /// among `candidates` as for [`Accel::pick`]. The default is [`Accel::pick`]; the zero-copy
    /// accelerator prefers a surface that is still grey from an earlier stand-in, so the grey is
    /// not written again (fork U0.2.3).
    fn pick_gray(&mut self, candidates: &[usize]) -> Option<usize> {
        self.pick(candidates)
    }
    /// Make surface `index` a copy of surface `from`, or mid-gray (every sample half the range)
    /// when `None`: the stand-ins for reference pictures that were never decoded, as the software
    /// decoders make them.
    fn fill(&mut self, index: usize, from: Option<usize>) -> std::result::Result<(), String>;
}

#[cfg(target_os = "linux")]
mod linux {
    use filmcraft_bitstream::unescape_rbsp;
    use filmcraft_codecs::hw::{NalCodec, NalStreamInfo};
    use filmcraft_codecs::{CodecError, DecodedFrame, Result, VideoDecoder};

    use super::{Accel, ffi, h264, hevc};
    use crate::biplanar::Geometry;
    use crate::va_accel::ZeroCopyAccel;

    /// The name of the zero-copy H.264 decoder (fork Y1; `perf.stats` backend).
    pub const NAME_H264_ZERO_COPY: &str = "VA-API H.264 (in process)";
    /// The name of the zero-copy HEVC 4:2:0 decoder (fork U0.2.2: upstream's front end; the
    /// fork's own decoder, `va_hevc`, was "VA-API HEVC (in process)").
    pub const NAME_HEVC_ZERO_COPY: &str = "VA-API HEVC (in process, upstream front end)";
    /// The name of the HEVC Main 4:2:2 10 decoder (fork U0.2.3: upstream's front end on the
    /// zero-copy accelerator, pictures handed to the compositing GPU; the fork's own decoder was
    /// "VA-API HEVC 4:2:2 (in process)").
    pub const NAME_HEVC_422_ZERO_COPY: &str = "VA-API HEVC 4:2:2 (in process, upstream front end)";

    /// The VA-API profiles that decode an H.264 stream of `profile_idc`, best fit first (a Main or
    /// High decoder also decodes what the smaller profiles allow).
    pub(crate) fn h264_profiles(profile_idc: u8) -> Option<&'static [ffi::VAProfile]> {
        Some(match profile_idc {
            66 => &[ffi::VAProfileH264ConstrainedBaseline, ffi::VAProfileH264Main, ffi::VAProfileH264High],
            77 => &[ffi::VAProfileH264Main, ffi::VAProfileH264High],
            100 => &[ffi::VAProfileH264High],
            _ => return None,
        })
    }

    /// The VA-API profiles that decode an HEVC stream of `general_profile_idc` with
    /// `chroma_format_idc` at `bits`, best fit first: 4:2:0 8-bit (Main, Main Still Picture, and
    /// any other profile of that format, range-extension profiles among them) and 10-bit, 4:2:2
    /// 10-bit (Main 4:2:2 10). As the fork's own decoder (`va_hevc`), the format picks the VA
    /// profile, not `general_profile_idc` (fork U0.2.3): what a profile's tools need beyond the
    /// format the parsers refuse (`check_supported`, as in software).
    pub(crate) fn hevc_profiles(profile_idc: u8, chroma_format_idc: u32, bits: u32) -> Option<&'static [ffi::VAProfile]> {
        Some(match (chroma_format_idc, bits) {
            (1, 8) if profile_idc == 2 => &[ffi::VAProfileHEVCMain10, ffi::VAProfileHEVCMain],
            (1, 8) => &[ffi::VAProfileHEVCMain, ffi::VAProfileHEVCMain10],
            (1, 10) => &[ffi::VAProfileHEVCMain10],
            (2, 10) => &[ffi::VAProfileHEVCMain422_10],
            _ => return None,
        })
    }

    enum Front {
        /// Fork Y1: H.264 with the pictures left on the GPU ([`ZeroCopyAccel`]).
        H264ZeroCopy(Box<h264::Front<ZeroCopyAccel>>),
        /// Fork U0.2.2: HEVC 4:2:0 with the pictures left on the GPU, missing references
        /// concealed with grey stand-ins.
        HevcZeroCopy(Box<hevc::Front<ZeroCopyAccel>>),
    }

    /// What an HEVC stream's SPS says the hardware needs: the VA profiles, coded size, chroma
    /// format and surface count (the DPB, the picture being decoded, and one spare).
    struct HevcSetup {
        profiles: &'static [ffi::VAProfile],
        size: (u32, u32),
        chroma_format_idc: u32,
        count: usize,
    }

    fn hevc_setup(info: &NalStreamInfo) -> std::result::Result<HevcSetup, String> {
        let (chroma, bits) = (info.chroma_format_idc, info.bit_depth_luma);
        let profiles = hevc_profiles(info.profile_idc, chroma, bits)
            .ok_or_else(|| format!("HEVC profile {} in chroma format {chroma} at {bits}-bit is not taken", info.profile_idc))?;
        let rbsp = parameter_set(&info.parameter_sets, 2, |h| (h >> 1) & 0x3f == 33).ok_or("no SPS")?;
        let sps = filmcraft_hevc::params::Sps::parse(&rbsp).map_err(|e| e.to_string())?;
        // range-extension tools are refused here as in software (fork U0.2.3: a range-extension
        // profile without them is taken)
        sps.check_supported().map_err(|e| e.to_string())?;
        if (sps.chroma_format_idc, sps.bit_depth_luma, sps.bit_depth_chroma) != (chroma, bits, info.bit_depth_chroma) {
            return Err("the SPS and the stream description disagree on the format".into());
        }
        Ok(HevcSetup {
            profiles,
            size: (sps.width, sps.height),
            chroma_format_idc: chroma,
            count: (sps.max_dec_pic_buffering as usize).saturating_add(2).min(18),
        })
    }

    /// What an H.264 stream's SPS says the hardware needs: the VA profiles, coded size (luma
    /// samples and macroblocks) and surface count (the DPB, the picture being decoded, and one
    /// spare).
    struct H264Setup {
        profiles: &'static [ffi::VAProfile],
        size: (u32, u32),
        mbs: (u32, u32),
        count: usize,
    }

    /// The common checks: 4:2:0 (HEVC: or 4:2:2, fork U0.2.3), one bit depth, progressive; the
    /// output geometry.
    fn geometry(info: &NalStreamInfo) -> std::result::Result<Geometry, String> {
        let bits = info.bit_depth_luma;
        let chroma_ok = info.chroma_format_idc == 1 || (info.chroma_format_idc == 2 && info.codec == NalCodec::Hevc);
        if !chroma_ok || info.bit_depth_chroma != bits || info.interlaced {
            return Err(format!(
                "{} chroma format {} at {bits}/{}-bit is not taken",
                if info.interlaced { "interlaced" } else { "progressive" },
                info.chroma_format_idc,
                info.bit_depth_chroma
            ));
        }
        Ok(Geometry { crop: info.crop, bits, color: info.color, par: info.par })
    }

    fn h264_setup(info: &NalStreamInfo) -> std::result::Result<H264Setup, String> {
        if info.bit_depth_luma != 8 {
            return Err(format!("{}-bit H.264 is not taken", info.bit_depth_luma));
        }
        let profiles = h264_profiles(info.profile_idc).ok_or_else(|| format!("H.264 profile_idc {} is not taken", info.profile_idc))?;
        let rbsp = parameter_set(&info.parameter_sets, 1, |h| h & 0x1f == 7).ok_or("no SPS")?;
        let sps = filmcraft_h264::params::Sps::parse(&rbsp).map_err(|e| e.to_string())?;
        sps.check_supported().map_err(|e| e.to_string())?;
        Ok(H264Setup {
            profiles,
            size: (sps.width(), sps.height()),
            mbs: (sps.pic_width_in_mbs, sps.frame_height_in_mbs()),
            count: sps.max_dpb_frames().saturating_add(2).min(18),
        })
    }

    /// A VA-API decoder for one H.264 or HEVC stream.
    pub struct VaDecoder {
        front: Front,
        info: NalStreamInfo,
        name: String,
        fed: u64,
        fail_after: Option<u64>,
        /// A 4:2:2 picture failed in hardware: no more pictures go to the engine until `reset`
        /// (a seek restarts at a random-access point with an empty DPB). Intel's media engine
        /// hung (GPU reset, ~7 s each) when damaged streams were fed on after a failed picture
        /// (fork 6.1b; ported in U0.2.3).
        stopped: bool,
    }

    /// The HEVC front end over `accel`, missing references concealed with grey stand-ins where
    /// the driver decodes pictures predicted from them; Intel's iHD fails each such picture
    /// ("internal decoding error", U0.2.3 hardware run): there the front end refuses it before
    /// the driver sees it, and the hybrid replays the run in software.
    fn hevc_front(
        accel: ZeroCopyAccel,
        info: &NalStreamInfo,
        size: (u32, u32),
        bits: u32,
        chroma_format_idc: u32,
    ) -> std::result::Result<hevc::Front<ZeroCopyAccel>, String> {
        let mut front = hevc::Front::new(accel, info.length_size, size, bits, &info.parameter_sets).map_err(|e| e.to_string())?;
        front.set_chroma_format(chroma_format_idc);
        let conceal = front.accel().decodes_from_stand_ins();
        front.set_conceal_missing(conceal);
        Ok(front)
    }

    /// The first parameter set of NAL unit type `ty` among `sets`, without its header (`header`
    /// bytes) and emulation prevention.
    fn parameter_set(sets: &[Vec<u8>], header: usize, ty: impl Fn(u8) -> bool) -> Option<Vec<u8>> {
        sets.iter().find(|n| n.first().is_some_and(|&h| ty(h))).and_then(|n| Some(unescape_rbsp(n.get(header..)?)))
    }

    impl VaDecoder {
        /// A decoder for the stream `info` describes whose pictures are read back to memory, or
        /// why VA-API does not take it on this system (U0.2.4: the zero-copy accelerator in its
        /// read-back mode, [`ZeroCopyAccel::new_read_back`]). HEVC 4:2:0 only.
        pub fn new(info: NalStreamInfo) -> std::result::Result<Self, String> {
            let geometry = geometry(&info)?;
            let (front, codec, vendor) = match info.codec {
                NalCodec::H264 => {
                    let H264Setup { profiles, size, mbs, count } = h264_setup(&info)?;
                    let accel = ZeroCopyAccel::new_read_back(profiles, size, count, geometry)?;
                    let vendor = accel.vendor().to_string();
                    (
                        Front::H264ZeroCopy(Box::new(h264::Front::new(accel, info.length_size, mbs, &info.parameter_sets).map_err(|e| e.to_string())?)),
                        "H.264",
                        vendor,
                    )
                }
                NalCodec::Hevc => {
                    let HevcSetup { profiles, size, chroma_format_idc, count } = hevc_setup(&info)?;
                    if chroma_format_idc != 1 {
                        // (4:2:2 read back: the zero-copy accelerator with its hand-off off)
                        return Err("HEVC 4:2:2 is not decoded by the read-back decoder".into());
                    }
                    let accel = ZeroCopyAccel::new_read_back(profiles, size, count, geometry)?;
                    let vendor = accel.vendor().to_string();
                    (Front::HevcZeroCopy(Box::new(hevc_front(accel, &info, size, geometry.bits, 1)?)), "HEVC", vendor)
                }
            };
            let name = format!("VA-API {codec} ({vendor})");
            Ok(Self { front, info, name, fed: 0, fail_after: None, stopped: false })
        }

        /// Fork Y1 / U0.2.2 / U0.2.3: a decoder for the H.264 or HEVC stream `info` describes
        /// whose pictures stay on the GPU ([`ZeroCopyAccel`]), or why not (the read-back
        /// [`VaDecoder::new`] takes what this declines). HEVC pictures whose references are
        /// missing decode against grey stand-ins, as in the fork's decoder and in software. HEVC
        /// Main 4:2:2 10 decodes on the display that has the profile (Intel here, whichever GPU
        /// is the process's device) and its pictures are handed to the compositing GPU as linear
        /// copies, or read back ([`VaDecoder::set_handoff`], `FILMCRAFT_NO_GPU_IMPORT`).
        pub fn zero_copy(info: NalStreamInfo) -> std::result::Result<Self, String> {
            let geometry = geometry(&info)?;
            match info.codec {
                NalCodec::H264 => {
                    let H264Setup { profiles, size, mbs, count } = h264_setup(&info)?;
                    let accel = ZeroCopyAccel::new(profiles, size, count, geometry, 1)?;
                    log::info!("VA-API H.264 zero-copy: {} ({}x{})", accel.vendor(), size.0, size.1);
                    let front = h264::Front::new(accel, info.length_size, mbs, &info.parameter_sets).map_err(|e| e.to_string())?;
                    let name = NAME_H264_ZERO_COPY.to_string();
                    Ok(Self { front: Front::H264ZeroCopy(Box::new(front)), info, name, fed: 0, fail_after: None, stopped: false })
                }
                NalCodec::Hevc => {
                    let HevcSetup { profiles, size, chroma_format_idc, count } = hevc_setup(&info)?;
                    let accel = ZeroCopyAccel::new(profiles, size, count, geometry, chroma_format_idc)?;
                    log::info!(
                        "VA-API HEVC zero-copy (upstream front end): {} on {} ({}x{}, chroma format {chroma_format_idc}, {}-bit)",
                        accel.vendor(),
                        accel.node(),
                        size.0,
                        size.1,
                        geometry.bits
                    );
                    let front = hevc_front(accel, &info, size, geometry.bits, chroma_format_idc)?;
                    let name = if chroma_format_idc == 2 { NAME_HEVC_422_ZERO_COPY } else { NAME_HEVC_ZERO_COPY }.to_string();
                    Ok(Self { front: Front::HevcZeroCopy(Box::new(front)), info, name, fed: 0, fail_after: None, stopped: false })
                }
            }
        }

        /// Whether the front ends take the stream `info` describes, from its parameter sets
        /// alone (no device is opened): `Err` with the reason for what no VA-API front end
        /// takes (since U0.2.3 that is every HEVC stream VA-API decodes in process).
        pub fn front_takes(info: &NalStreamInfo) -> std::result::Result<(), String> {
            geometry(info)?;
            match info.codec {
                NalCodec::H264 => h264_setup(info).map(|_| ()),
                NalCodec::Hevc => hevc_setup(info).map(|_| ()),
            }
        }

        /// Test hook: fail every decode call after the first `n` (a GPU lost mid-stream).
        pub fn fail_after(&mut self, n: u64) {
            self.fail_after = Some(n);
        }

        /// HEVC 4:2:2: hand pictures to the compositing GPU (`on`, the default unless
        /// `FILMCRAFT_NO_GPU_IMPORT` is set) or read them back to memory. Nothing otherwise.
        pub fn set_handoff(&mut self, on: bool) {
            if let Front::HevcZeroCopy(f) = &mut self.front {
                f.accel_mut().set_handoff(on);
            }
        }

        /// Test hook (HEVC 4:2:2): the next hand-off fails as a refused copy or export would;
        /// the stream reads pictures back from then on.
        pub fn fail_next_handoff(&mut self) {
            if let Front::HevcZeroCopy(f) = &mut self.front {
                f.accel_mut().fail_next_handoff();
            }
        }

        /// Whether pictures whose references are missing decode against grey stand-ins (the
        /// zero-copy HEVC decoder where the driver takes them) or are refused (then the hybrid
        /// decoder replays the run in software).
        pub fn conceals_missing(&self) -> bool {
            match &self.front {
                Front::HevcZeroCopy(f) => f.conceals_missing(),
                _ => false,
            }
        }

        /// Stand-ins the zero-copy HEVC decoder made for missing references, and the grey images
        /// it wrote for them (tests: a stand-in reuses a surface still grey from an earlier one).
        pub fn gray_stats(&self) -> (u64, u64) {
            match &self.front {
                Front::HevcZeroCopy(f) => f.accel().gray_stats(),
                _ => (0, 0),
            }
        }

        fn is_422(&self) -> bool {
            self.info.codec == NalCodec::Hevc && self.info.chroma_format_idc == 2
        }

        /// A 4:2:2 stream failed in hardware: the clip badges warn again, saying so
        /// (session-wide; fork 6.1b).
        fn note_failure(&self) {
            if self.is_422() {
                filmcraft_codecs::hw::note_hevc_422_failure();
            }
        }

        /// The surfaces the decoder has (a zero-copy decoder grows while frames hold them; tests).
        pub fn surface_count(&self) -> usize {
            match &self.front {
                Front::H264ZeroCopy(f) => f.accel().surfaces().len(),
                Front::HevcZeroCopy(f) => f.accel().surfaces().len(),
            }
        }
    }

    impl VideoDecoder for VaDecoder {
        fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
            self.fed = self.fed.saturating_add(1);
            if self.fail_after.is_some_and(|n| self.fed > n) {
                self.note_failure();
                return Err(CodecError::Decode("VA-API decoder failed (test hook)".into()));
            }
            if self.stopped {
                return Err(CodecError::Decode("VA-API HEVC 4:2:2: stopped after a failed picture (until the next seek)".into()));
            }
            let r = match &mut self.front {
                Front::H264ZeroCopy(f) => f.decode(sample, pts),
                Front::HevcZeroCopy(f) => f.decode(sample, pts),
            };
            // a picture refused before anything reached the engine (missing references on a
            // decoder that does not conceal them) is not a hardware failure: no stop, no 4:2:2
            // failure badge (U0.2.3 review; the check itself landed in U0.2.3b d)
            if r.as_ref().is_err_and(|e| !matches!(e, CodecError::Unsupported(_))) {
                self.stopped = self.is_422();
                self.note_failure();
            }
            r
        }

        fn flush(&mut self) -> Vec<DecodedFrame> {
            if self.stopped {
                // nothing more goes to the engine: drop the pending picture and the DPB (stays
                // stopped until the next seek)
                self.reset();
                self.stopped = true;
                return Vec::new();
            }
            let flushed = match &mut self.front {
                Front::H264ZeroCopy(f) => f.flush(),
                Front::HevcZeroCopy(f) => f.flush(),
            };
            match flushed {
                Ok(out) => out,
                Err(e) => {
                    log::warn!("{}: {e} while flushing", self.name);
                    Vec::new()
                }
            }
        }

        fn reset(&mut self) {
            self.stopped = false;
            match &mut self.front {
                Front::H264ZeroCopy(f) => f.reset(),
                Front::HevcZeroCopy(f) => f.reset(),
            }
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn is_random_access(&self, sample: &[u8]) -> Option<bool> {
            self.info.is_random_access(sample)
        }

        fn is_disposable(&self, sample: &[u8]) -> bool {
            self.info.is_disposable(sample)
        }
    }

    /// The stream description of an Annex B HEVC stream (its first VPS / SPS / PPS, 4-byte
    /// length prefixes), as an MP4 `hvcC` would give it (tests, diagnostics: fork U0.2.2).
    pub fn annexb_hevc_info(data: &[u8]) -> std::result::Result<NalStreamInfo, String> {
        let nals = filmcraft_bitstream::annexb_nals(data);
        let first = |ty: u8| nals.iter().copied().find(|n| n.first().is_some_and(|&h| (h >> 1) & 0x3f == ty));
        let sets: Vec<&[u8]> = [32, 33, 34].into_iter().filter_map(first).collect();
        // hvcC: version 1, profile / level fields unused here, lengthSizeMinusOne 3, the arrays
        let mut hvcc = vec![1u8];
        hvcc.extend_from_slice(&[0; 20]);
        hvcc.push(3);
        hvcc.push(u8::try_from(sets.len()).map_err(|_| "parameter sets")?);
        for nal in sets {
            let len = u16::try_from(nal.len()).map_err(|_| "parameter set too large")?;
            hvcc.push(nal.first().map_or(0, |h| (h >> 1) & 0x3f));
            hvcc.extend_from_slice(&1u16.to_be_bytes());
            hvcc.extend_from_slice(&len.to_be_bytes());
            hvcc.extend_from_slice(nal);
        }
        NalStreamInfo::from_hvcc(&hvcc).map_err(|e| e.to_string())
    }

    /// Decode a whole Annex B HEVC stream through the zero-copy decoder
    /// ([`VaDecoder::zero_copy`]) and read every picture back, in output order (tests,
    /// diagnostics: fork U0.2.2, the counterpart of `va_hevc::decode_annexb`). Each frame is read
    /// back as it comes out, so its surface is free again for the next picture. 4:2:2 decodes on
    /// the display that has Main 4:2:2 10 (fork U0.2.3).
    pub fn decode_annexb_zero_copy(data: &[u8]) -> std::result::Result<Vec<filmcraft_frame::VideoFrame>, String> {
        decode_annexb_with(data, false)
    }

    /// [`decode_annexb_zero_copy`], with `handoff`: 4:2:2 pictures come out as the decoder hands
    /// them to the compositing GPU (linear copies, `GpuFormat::Y210` GPU frames, fork 6.1b)
    /// instead of read back; an error when one is not handed off. 4:2:0 pictures are read back
    /// either way (the counterpart of `va_hevc::decode_annexb_with`, fork U0.2.3), unless this
    /// thread decodes for another GPU (`hw::decode_hands_off`, U0.2.3b d): then they too must
    /// come out handed off (NV12 / P010 linear copies).
    pub fn decode_annexb_with(data: &[u8], handoff: bool) -> std::result::Result<Vec<filmcraft_frame::VideoFrame>, String> {
        let info = annexb_hevc_info(data)?;
        let packed = info.chroma_format_idc == 2;
        let handed = handoff && (packed || filmcraft_codecs::hw::decode_hands_off());
        let mut dec = VaDecoder::zero_copy(info)?;
        dec.set_handoff(handoff);
        // access units: a VCL NAL unit that starts a picture (first_slice_segment_in_pic_flag)
        // after one that did not ends the access unit before it
        let mut samples: Vec<Vec<u8>> = Vec::new();
        let mut sample = Vec::new();
        let mut has_vcl = false;
        for nal in filmcraft_bitstream::annexb_nals(data) {
            let Some(&h) = nal.first() else { continue };
            let vcl = (h >> 1) & 0x3f < 32;
            let first_slice = nal.get(2).is_some_and(|b| b & 0x80 != 0);
            if vcl && first_slice && has_vcl {
                samples.push(std::mem::take(&mut sample));
                has_vcl = false;
            }
            has_vcl |= vcl;
            let len = u32::try_from(nal.len()).map_err(|_| "NAL unit too large")?;
            sample.extend_from_slice(&len.to_be_bytes());
            sample.extend_from_slice(nal);
        }
        if !sample.is_empty() {
            samples.push(sample);
        }
        let read_back = |d: DecodedFrame| -> std::result::Result<filmcraft_frame::VideoFrame, String> {
            let f = d.frame;
            match &f.data {
                filmcraft_frame::PixelData::Gpu(s) if handed => {
                    if packed != (s.format() == filmcraft_frame::GpuFormat::Y210) {
                        return Err(format!("a handed-off picture of format {:?} (4:2:2: {packed})", s.format()));
                    }
                    Ok(f)
                }
                _ if handed => Err("a picture was not handed off".into()),
                filmcraft_frame::PixelData::Gpu(s) => {
                    let data = s.download(f.width, f.height)?;
                    Ok(filmcraft_frame::VideoFrame { data, ..f })
                }
                _ => Ok(f),
            }
        };
        let mut out = Vec::new();
        for (pts, s) in samples.iter().enumerate() {
            let frames = dec.decode(s, i64::try_from(pts).unwrap_or(i64::MAX)).map_err(|e| e.to_string())?;
            for d in frames {
                out.push(read_back(d)?);
            }
        }
        for d in dec.flush() {
            out.push(read_back(d)?);
        }
        Ok(out)
    }
}
