//! Hardware decoding through VA-API (Linux: Intel, AMD and other Mesa / vendor drivers).
//!
//! VA-API decoding is stateless: the application parses the stream and keeps the decoded picture
//! buffer, and the GPU decodes one picture's slice data at a time into a surface. [`h264`] does the
//! host side with the software decoder's own parsers and DPB (so the two decide alike), [`va`]
//! drives libva (loaded at run time; no libva means no hardware decoder, never a failure to start),
//! and [`VaDecoder`] puts them together as a [`VideoDecoder`]. H.264 8-bit 4:2:0 progressive
//! (Constrained Baseline / Main / High) and HEVC 4:2:0 (Main, Main 10, Main Still Picture), the
//! latter through [`hevc`] the same way; everything else stays in software.
//!
//! The FFI is in [`ffi`] (declarations) and `va` (calls); the rest of this module is safe code.

pub mod ffi;
pub mod gray;
pub mod h264;
pub mod hevc;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod va;

#[cfg(test)]
mod abi_tests;
#[cfg(test)]
mod tests;

#[cfg(target_os = "linux")]
pub use linux::{NAME_H264_ZERO_COPY, VaDecoder};

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

    use super::va::Session;
    use super::{Accel, ffi, h264, hevc};
    use crate::biplanar::Geometry;
    use crate::va_accel::ZeroCopyAccel;

    /// The name of the zero-copy H.264 decoder (fork Y1; `perf.stats` backend).
    pub const NAME_H264_ZERO_COPY: &str = "VA-API H.264 (in process)";

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

    /// The VA-API profiles that decode an HEVC stream of `general_profile_idc` (Main, Main 10,
    /// Main Still Picture) at `bits`, best fit first.
    pub(crate) fn hevc_profiles(profile_idc: u8, bits: u32) -> Option<&'static [ffi::VAProfile]> {
        Some(match (profile_idc, bits) {
            (1 | 3, 8) => &[ffi::VAProfileHEVCMain, ffi::VAProfileHEVCMain10],
            (2, 8) => &[ffi::VAProfileHEVCMain10, ffi::VAProfileHEVCMain],
            (2, 10) => &[ffi::VAProfileHEVCMain10],
            _ => return None,
        })
    }

    enum Front {
        H264(Box<h264::Front<Session>>),
        Hevc(Box<hevc::Front<Session>>),
        /// Fork Y1: H.264 with the pictures left on the GPU ([`ZeroCopyAccel`]).
        H264ZeroCopy(Box<h264::Front<ZeroCopyAccel>>),
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

    /// The common checks: 4:2:0, one bit depth, progressive; the output geometry.
    fn geometry(info: &NalStreamInfo) -> std::result::Result<Geometry, String> {
        let bits = info.bit_depth_luma;
        if info.chroma_format_idc != 1 || info.bit_depth_chroma != bits || info.interlaced {
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
    }

    /// The first parameter set of NAL unit type `ty` among `sets`, without its header (`header`
    /// bytes) and emulation prevention.
    fn parameter_set(sets: &[Vec<u8>], header: usize, ty: impl Fn(u8) -> bool) -> Option<Vec<u8>> {
        sets.iter().find(|n| n.first().is_some_and(|&h| ty(h))).and_then(|n| Some(unescape_rbsp(n.get(header..)?)))
    }

    impl VaDecoder {
        /// A decoder for the stream `info` describes, or why VA-API does not take it on this system.
        pub fn new(info: NalStreamInfo) -> std::result::Result<Self, String> {
            let bits = info.bit_depth_luma;
            let geometry = geometry(&info)?;
            let (front, codec, vendor) = match info.codec {
                NalCodec::H264 => {
                    let H264Setup { profiles, size, mbs, count } = h264_setup(&info)?;
                    let session = Session::new(profiles, size, count, geometry)?;
                    let vendor = session.vendor().to_string();
                    (Front::H264(Box::new(h264::Front::new(session, info.length_size, mbs, &info.parameter_sets).map_err(|e| e.to_string())?)), "H.264", vendor)
                }
                NalCodec::Hevc => {
                    let profiles =
                        hevc_profiles(info.profile_idc, bits).ok_or_else(|| format!("HEVC profile {} at {bits}-bit is not taken", info.profile_idc))?;
                    let rbsp = parameter_set(&info.parameter_sets, 2, |h| (h >> 1) & 0x3f == 33).ok_or("no SPS")?;
                    let sps = filmcraft_hevc::params::Sps::parse(&rbsp).map_err(|e| e.to_string())?;
                    sps.check_supported().map_err(|e| e.to_string())?;
                    if sps.range_extension {
                        return Err("HEVC range extensions are not taken".into());
                    }
                    let count = (sps.max_dec_pic_buffering as usize).saturating_add(2).min(18);
                    let size = (sps.width, sps.height);
                    let session = Session::new(profiles, size, count, geometry)?;
                    let vendor = session.vendor().to_string();
                    (
                        Front::Hevc(Box::new(hevc::Front::new(session, info.length_size, size, bits, &info.parameter_sets).map_err(|e| e.to_string())?)),
                        "HEVC",
                        vendor,
                    )
                }
            };
            let name = format!("VA-API {codec} ({vendor})");
            Ok(Self { front, info, name, fed: 0, fail_after: None })
        }

        /// Fork Y1: a decoder for the H.264 stream `info` describes whose pictures stay on the
        /// GPU ([`ZeroCopyAccel`]), or why not (HEVC has the fork's own decoder, `va_hevc`; the
        /// read-back [`VaDecoder::new`] takes what this declines).
        pub fn zero_copy(info: NalStreamInfo) -> std::result::Result<Self, String> {
            if info.codec != NalCodec::H264 {
                return Err("zero-copy VA-API decodes H.264 only".into());
            }
            let geometry = geometry(&info)?;
            let H264Setup { profiles, size, mbs, count } = h264_setup(&info)?;
            let accel = ZeroCopyAccel::new(profiles, size, count, geometry)?;
            log::info!("VA-API H.264 zero-copy: {} ({}x{})", accel.vendor(), size.0, size.1);
            let front = h264::Front::new(accel, info.length_size, mbs, &info.parameter_sets).map_err(|e| e.to_string())?;
            Ok(Self { front: Front::H264ZeroCopy(Box::new(front)), info, name: NAME_H264_ZERO_COPY.to_string(), fed: 0, fail_after: None })
        }

        /// Test hook: fail every decode call after the first `n` (a GPU lost mid-stream).
        pub fn fail_after(&mut self, n: u64) {
            self.fail_after = Some(n);
        }

        /// The surfaces the decoder has (a zero-copy decoder grows while frames hold them; tests).
        pub fn surface_count(&self) -> usize {
            match &self.front {
                Front::H264(f) => f.accel().surfaces().len(),
                Front::Hevc(f) => f.accel().surfaces().len(),
                Front::H264ZeroCopy(f) => f.accel().surfaces().len(),
            }
        }
    }

    impl VideoDecoder for VaDecoder {
        fn decode(&mut self, sample: &[u8], pts: i64) -> Result<Vec<DecodedFrame>> {
            self.fed += 1;
            if self.fail_after.is_some_and(|n| self.fed > n) {
                return Err(CodecError::Decode("VA-API decoder failed (test hook)".into()));
            }
            match &mut self.front {
                Front::H264(f) => f.decode(sample, pts),
                Front::Hevc(f) => f.decode(sample, pts),
                Front::H264ZeroCopy(f) => f.decode(sample, pts),
            }
        }

        fn flush(&mut self) -> Vec<DecodedFrame> {
            let flushed = match &mut self.front {
                Front::H264(f) => f.flush(),
                Front::Hevc(f) => f.flush(),
                Front::H264ZeroCopy(f) => f.flush(),
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
            match &mut self.front {
                Front::H264(f) => f.reset(),
                Front::Hevc(f) => f.reset(),
                Front::H264ZeroCopy(f) => f.reset(),
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
}
