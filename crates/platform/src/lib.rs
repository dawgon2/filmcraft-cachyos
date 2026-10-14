//! OS media integration (layer L5): hardware video decoding and encoding through the operating
//! system's codecs.
//!
//! [`register`] puts the platform's hardware decoder factory in front of FilmCraft's own decoders
//! (`filmcraft_codecs::register_video_decoder`). Today that is VideoToolbox on macOS for H.264
//! (`avcC`) and HEVC (`hvcC`) streams, 8- and 10-bit, 4:2:0 and 4:2:2; Media Foundation / DXVA on
//! Windows; NVDEC ([`nvdec`], NVIDIA's driver) and VA-API on Linux for H.264 and HEVC:
//! upstream's in-process H.264 / HEVC decoder ([`vaapi`]; with the fork's zero-copy output first,
//! [`va_accel`]: frames left on the GPU, HEVC 4:2:2 handed from the decoding GPU to the
//! compositing one; then reading pictures back) and the system ffmpeg run as a child process
//! ([`vaapi_pipe`]) behind it (the fork's own HEVC decoder, `va_hevc`, left the chain in U0.2.3
//! and the code in U0.2.5); on other systems (and on Linux with none of these) registration
//! does nothing and reports [`Availability::Unavailable`]. It also registers a hardware H.264
//! encoder factory (`filmcraft_export::register_encoder`) that only acts when an export asks for
//! it (`ExportSettings::hardware_encoding` = `Auto`), see [`hardware_encode`]; on macOS, on Windows
//! (NVENC, [`nvenc`]) and on Linux (VA-API or libx265, [`hevc_export`]) it also makes the H.265
//! export format available, and on Windows (Main 10) lets HDR sequences export as HDR H.265.
//!
//! Hardware decoding never makes a file undecodable:
//!
//! - the factory declines (falls through to the software decoder) when Settings ▸ Playback ▸
//!   Hardware decoding is Off (`filmcraft_codecs::hw::set_hardware_decoding`), when the stream's
//!   format is one the hardware path does not take, or when the OS cannot create a hardware
//!   session for it (profile, size, no hardware decoder);
//! - a decoder that fails mid-stream switches to the software decoder transparently
//!   ([`HybridDecoder`]) and logs it.
//!
//! Pictures are the software decoder's: same planes (bit-exact on the parity fixtures), colour,
//! pixel aspect, pts and presentation order, so the two are interchangeable.
//!
//! This is the one crate allowed to use `unsafe` (OS FFI), and only in its FFI modules
//! (docs/adr/0001-platform-ffi.md).

#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable))]

// Used by the Windows (and, for `biplanar`, Linux) decoders; compiled everywhere so their tests run
// on every system.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod annexb;
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
mod biplanar;
pub mod cursor;
/// Call before the process exits early (a short CLI command): background hardware probes started
/// by [`register`] finish (bounded) instead of running into the drivers' exit handlers (U0.2.4: a
/// probe inside Intel's driver initialisation at exit corrupted the heap). Nothing to do on
/// systems without such probes.
pub fn quiesce() {
    #[cfg(target_os = "linux")]
    va::quiesce();
}

#[cfg(target_os = "linux")]
pub mod export_lanes;
#[cfg(target_os = "linux")]
pub mod ffmpeg;
#[cfg(target_os = "linux")]
mod gpu_export;
/// One export renderer per GPU (fork 6E.3).
#[cfg(target_os = "linux")]
pub use gpu_export::{frames_on, pci_ids};
#[cfg(target_os = "macos")]
pub mod hardware_encode;
#[cfg(target_os = "linux")]
pub mod hevc_export;
pub mod hybrid;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod kde_appmenu;
#[cfg(target_os = "windows")]
pub mod media_foundation;
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub mod nvdec;
#[cfg(any(target_os = "windows", all(target_os = "linux", target_pointer_width = "64")))]
pub mod nvenc;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
pub mod va;
#[cfg(target_os = "linux")]
pub mod va_accel;
#[cfg(target_os = "linux")]
pub mod va_enc;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod va_sys;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod vaapi;
#[cfg(target_os = "linux")]
pub mod vaapi_pipe;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod videotoolbox;
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub mod videotoolbox_encode;

pub use biplanar::Geometry;
pub use hybrid::HybridDecoder;

/// What [`register`] made available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// A hardware decoder factory was registered (its name).
    Available(&'static str),
    /// Nothing to register on this system (why).
    Unavailable(&'static str),
}

/// Register the platform's hardware video decoders (call once at startup; repeated calls are
/// harmless). Streams they do not take, and every stream while hardware decoding is Off, keep
/// using FilmCraft's own decoders.
pub fn register() -> Availability {
    #[cfg(all(target_os = "linux", target_pointer_width = "64"))]
    filmcraft_export::register_encoder(nvenc::export::factory);
    #[cfg(target_os = "macos")]
    {
        filmcraft_codecs::register_video_decoder(videotoolbox_factory);
        filmcraft_export::register_encoder(hardware_encode::videotoolbox_encoder_factory);
        filmcraft_export::register_format_probe(filmcraft_export::Format::Hevc, hardware_encode::hevc_available);
        // the probe creates a hardware session (about 0.1 s): answer it now, off the UI thread,
        // before the first draw of the format list asks
        hardware_encode::warm_hevc_probe();
        filmcraft_codecs::hw::set_hw_backend("VideoToolbox");
        Availability::Available("VideoToolbox")
    }
    // Export ▸ H.265: NVENC is FilmCraft's only HEVC encoder on Windows, so the format is available
    // when a small HEVC session opens. That takes about half a second, so it is asked on a thread of
    // its own now rather than by the first draw of the format list.
    #[cfg(target_os = "windows")]
    {
        filmcraft_export::register_format_probe(filmcraft_export::Format::Hevc, nvenc::hevc_available);
        // HDR sequences export as HEVC Main 10 (PQ / HLG) only where NVENC has a 10-bit encoder; elsewhere
        // (and on macOS, whose VideoToolbox path is 8-bit) they are tone-mapped to SDR, as before
        filmcraft_export::register_hdr_probe(filmcraft_export::Format::Hevc, nvenc::hevc_hdr_available);
        nvenc::warm_hevc_probe();
    }
    #[cfg(target_os = "windows")]
    {
        static ENCODERS: std::sync::Once = std::sync::Once::new();
        // Export ▸ Hardware encoding (NVENC H.264): in front of the software encoder, taking an
        // export only when asked for and when NVENC can do it
        ENCODERS.call_once(|| filmcraft_export::register_encoder(nvenc::export::factory));
        filmcraft_codecs::register_video_decoder(media_foundation_factory);
        filmcraft_codecs::hw::set_hw_backend("Media Foundation");
        Availability::Available("Media Foundation")
    }
    #[cfg(target_os = "linux")]
    {
        // H.265 / HEVC export (VA-API or libx265), independent of hardware decoding, with its
        // frames rendered on the GPU
        hevc_export::register();
        gpu_export::register();
        // two-GPU export (fork 6E.3): the second GPU's lane, when there is one
        export_lanes::register();
        // The VA-API decode paths share one factory (`vaapi_factory`): upstream's in-process
        // H.264 / HEVC front ends on the fork's display (frames left on the GPU, or read back),
        // and the ffmpeg pipe (H.264 / HEVC) behind them (U0.2.4: one display layer).
        static ONCE: std::sync::Once = std::sync::Once::new();
        let native = va::Display::get();
        let pipe = ffmpeg::ffmpeg().is_some() && ffmpeg::vaapi_device().is_some();
        ONCE.call_once(|| {
            match &native {
                Ok(d) => log::info!("hardware decoding through in-process VA-API: {} on {}", d.vendor, d.node),
                Err(e) => log::info!("no in-process VA-API hardware decoding: {e}"),
            }
            if pipe {
                log::info!("hardware decoding through VA-API (ffmpeg pipe) available");
            }
        });
        let mut available = if native.is_ok() || pipe {
            filmcraft_codecs::register_video_decoder(vaapi_factory);
            filmcraft_codecs::hw::set_hw_backend("VA-API");
            // 4:2:2 HEVC on a display with Main 4:2:2 10 (fork 6.1b): the clip badges ask this,
            // without waiting for the display search, which starts now in the background
            va_accel::warm_422_probe();
            filmcraft_codecs::hw::set_hevc_422_probe(va_accel::hardware_422_ready);
            Availability::Available("VA-API")
        } else {
            Availability::Unavailable("no VA-API driver (libva and a DRM render node), no ffmpeg with a VA-API device and no NVIDIA driver (NVDEC)")
        };
        // NVIDIA's proprietary driver has no VA-API of its own: NVDEC goes in front, and what it
        // declines still reaches VA-API (a second GPU) and then the software decoders
        #[cfg(target_pointer_width = "64")]
        match nvdec::cuvid::probe() {
            Ok(driver) => {
                static NVDEC: std::sync::Once = std::sync::Once::new();
                NVDEC.call_once(|| log::info!("hardware decoding through NVDEC: {driver}"));
                filmcraft_codecs::register_video_decoder(nvdec_factory);
                filmcraft_codecs::hw::set_hw_backend("NVDEC");
                available = Availability::Available("NVDEC");
            }
            Err(why) => log::info!("no NVDEC hardware decoding: {why}"),
        }
        available
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Availability::Unavailable("no hardware video decoder for this system yet")
    }
}

/// Whether [`register`] has put a hardware decoder factory in front of our decoders.
pub fn registered() -> bool {
    #[cfg(target_os = "macos")]
    {
        filmcraft_codecs::video_decoder_registered(videotoolbox_factory)
    }
    #[cfg(target_os = "windows")]
    {
        filmcraft_codecs::video_decoder_registered(media_foundation_factory)
    }
    #[cfg(target_os = "linux")]
    {
        #[cfg(target_pointer_width = "64")]
        if filmcraft_codecs::video_decoder_registered(nvdec_factory) {
            return true;
        }
        filmcraft_codecs::video_decoder_registered(vaapi_factory)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        false
    }
}

/// Whether this system's hardware decoder takes the stream of `entry` (whatever the Hardware
/// decoding setting says): diagnostics and tests.
pub fn hardware_decoder_for(entry: &filmcraft_isobmff::SampleEntry) -> bool {
    #[cfg(target_os = "macos")]
    {
        filmcraft_codecs::hw::NalStreamInfo::from_entry(entry).and_then(|r| r.ok()).is_some_and(|info| videotoolbox::VtDecoder::new(info).is_ok())
    }
    #[cfg(target_os = "windows")]
    {
        media_foundation::stream_info(entry).is_some_and(|info| media_foundation::MfDecoder::new(info).is_ok())
    }
    #[cfg(target_os = "linux")]
    {
        filmcraft_codecs::hw::NalStreamInfo::from_entry(entry).and_then(|r| r.ok()).is_some_and(|info| {
            #[cfg(target_pointer_width = "64")]
            if nvdec::NvDecoder::new(info.clone()).is_ok() {
                return true;
            }
            vaapi::VaDecoder::zero_copy(info.clone()).is_ok() || vaapi::VaDecoder::new(info.clone()).is_ok() || vaapi_pipe::VaapiDecoder::new(info).is_ok()
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = entry;
        false
    }
}

/// Whether HEVC Main 4:2:2 10 clips decode in hardware on this system (fork Phase 6.1b: VA-API on
/// a display with that profile, Hardware decoding on, no 4:2:2 hardware failure this session),
/// waiting for the display search (diagnostics, tests). False: they decode in software.
pub fn hevc_422_in_hardware() -> bool {
    // (the UI asks the non-blocking `filmcraft_codecs::hw::hevc_422_in_hardware`)
    #[cfg(target_os = "linux")]
    {
        va_accel::hardware_422()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// The VideoToolbox factory: a [`HybridDecoder`] around [`videotoolbox::VtDecoder`] for `avcC` /
/// `hvcC` streams VideoToolbox can decode in hardware, `None` otherwise.
#[cfg(target_os = "macos")]
pub fn videotoolbox_factory(entry: &filmcraft_isobmff::SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn filmcraft_codecs::VideoDecoder>>> {
    if !filmcraft_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    match videotoolbox::VtDecoder::new(info.clone()) {
        Ok(vt) => Some(Ok(Box::new(HybridDecoder::new(Box::new(vt), entry.clone(), info)))),
        Err(why) => {
            log::info!("hardware decoding declined for {} video: {why}", entry.codec.name());
            filmcraft_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// The Media Foundation factory: a [`HybridDecoder`] around [`media_foundation::MfDecoder`] for
/// H.264 / HEVC / VP9 / AV1 streams a Direct3D-aware decoder MFT can decode with DXVA on this
/// system's GPU, `None` otherwise.
#[cfg(target_os = "windows")]
pub fn media_foundation_factory(entry: &filmcraft_isobmff::SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn filmcraft_codecs::VideoDecoder>>> {
    if !filmcraft_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = media_foundation::stream_info(entry)?;
    match media_foundation::MfDecoder::new(info.clone()) {
        Ok(mf) => Some(Ok(Box::new(HybridDecoder::new(Box::new(mf), entry.clone(), info)))),
        Err(why) => {
            log::info!("hardware decoding declined for {} video: {why}", entry.codec.name());
            filmcraft_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// The VA-API factory: a [`HybridDecoder`] around the first of the VA-API decoders that takes the
/// stream, with the next ones as its fallbacks (ADR 0002's chain) before software:
///
/// 1. upstream's in-process decoder with the fork's zero-copy output
///    ([`VaDecoder::zero_copy`](vaapi::VaDecoder::zero_copy): H.264 since Y1, HEVC 4:2:0 since
///    U0.2.2, HEVC Main 4:2:2 10 since U0.2.3 on the display that has it, handed to the
///    compositing GPU or read back; 4:2:0 declines when the surfaces cannot be exported or
///    `FILMCRAFT_NO_GPU_IMPORT` is set). Every HEVC stream VA-API decodes in process goes
///    through upstream's front end since U0.2.3 (the fork's own decoder, `va_hevc`, was
///    removed in U0.2.5);
/// 2. the same front ends with pictures read back ([`vaapi::VaDecoder::new`], H.264 and HEVC
///    4:2:0; `ZeroCopyAccel::new_read_back` since U0.2.4);
/// 3. the ffmpeg pipe ([`vaapi_pipe::VaapiDecoder`], H.264 and HEVC).
///
/// `None` when none takes the stream (other codecs keep the software decoders, and every stream
/// while Hardware decoding is Off).
#[cfg(target_os = "linux")]
pub fn vaapi_factory(entry: &filmcraft_isobmff::SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn filmcraft_codecs::VideoDecoder>>> {
    type Hw = std::result::Result<Box<dyn filmcraft_codecs::VideoDecoder>, String>;
    if !filmcraft_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    let codec = entry.codec.name();
    let zero_copy =
        |info: filmcraft_codecs::hw::NalStreamInfo| -> Hw { vaapi::VaDecoder::zero_copy(info).map(|d| Box::new(d) as Box<dyn filmcraft_codecs::VideoDecoder>) };
    let upstream =
        |info: filmcraft_codecs::hw::NalStreamInfo| -> Hw { vaapi::VaDecoder::new(info).map(|d| Box::new(d) as Box<dyn filmcraft_codecs::VideoDecoder>) };
    let pipe = |info: filmcraft_codecs::hw::NalStreamInfo| -> Hw {
        if ffmpeg::ffmpeg().is_none() {
            return Err("ffmpeg is not installed".into());
        }
        vaapi_pipe::VaapiDecoder::new(info).map(|d| Box::new(d) as Box<dyn filmcraft_codecs::VideoDecoder>)
    };
    // the chain, first taker first; the ones after it become the hybrid's fallbacks
    let mut chain: Vec<Box<dyn FnOnce(filmcraft_codecs::hw::NalStreamInfo) -> Hw + Send>> = vec![Box::new(zero_copy), Box::new(upstream), Box::new(pipe)];
    let mut whys = Vec::new();
    let mut primary: Option<Box<dyn filmcraft_codecs::VideoDecoder>> = None;
    while primary.is_none() && !chain.is_empty() {
        match chain.remove(0)(info.clone()) {
            Ok(d) => primary = Some(d),
            Err(why) => {
                log::info!("VA-API declined {codec} video: {why}");
                whys.push(why);
            }
        }
    }
    match primary {
        Some(d) => {
            let mut h = HybridDecoder::new(d, entry.clone(), info.clone());
            for make in chain {
                let info = info.clone();
                h = h.with_fallback(Box::new(move || make(info)));
            }
            Some(Ok(Box::new(h)))
        }
        None => {
            log::info!("hardware decoding declined for {codec} video: {}", whys.join("; "));
            filmcraft_codecs::hw::note_hw_declined();
            None
        }
    }
}

/// The NVDEC factory: a [`HybridDecoder`] around [`nvdec::NvDecoder`] for H.264 and HEVC streams
/// this system's NVIDIA GPU decodes, `None` otherwise (VA-API and the software decoders keep those).
#[cfg(all(target_os = "linux", target_pointer_width = "64"))]
pub fn nvdec_factory(entry: &filmcraft_isobmff::SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn filmcraft_codecs::VideoDecoder>>> {
    if !filmcraft_codecs::hw::hardware_decoding() {
        return None;
    }
    let info = filmcraft_codecs::hw::NalStreamInfo::from_entry(entry)?.ok()?;
    match nvdec::NvDecoder::new(info.clone()) {
        Ok(nv) => Some(Ok(Box::new(HybridDecoder::new(Box::new(nv), entry.clone(), info)))),
        Err(why) => {
            log::info!("NVDEC declined {} video: {why}", entry.codec.name());
            // with VA-API behind this factory, that one counts the decline (or takes the stream)
            if !filmcraft_codecs::video_decoder_registered(vaapi_factory) {
                filmcraft_codecs::hw::note_hw_declined();
            }
            None
        }
    }
}
