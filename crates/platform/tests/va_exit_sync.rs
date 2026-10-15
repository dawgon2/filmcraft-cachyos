//! U0.2 owed 4 (2026-10-15): once the process exits ([`filmcraft_platform::quiesce`]), surface work
//! outside a picture submit (sync, image read / write, DMA-BUF export, new surfaces) is refused
//! too, so the drivers' exit handlers never run under it. Its own test binary: exiting is
//! process-wide. Skips without a VA-API driver.
#![cfg(target_os = "linux")]

use filmcraft_platform::va::{self, Display, SurfacePool};

// libva's VA_RT_FORMAT_YUV420 and VA_FOURCC_NV12 (va.h)
const YUV420: u32 = 0x1;
const NV12: u32 = 0x3231_564E;

#[test]
fn no_surface_work_starts_after_quiesce() {
    let Ok(d) = Display::get() else {
        eprintln!("SKIPPED: no VA-API on this machine");
        return;
    };
    let pool = SurfacePool::new(&d, YUV420, NV12, 64, 64);
    let Ok(surface) = pool.get() else {
        eprintln!("SKIPPED: the driver made no NV12 surface");
        return;
    };
    assert!(surface.sync().is_ok(), "before exiting");
    filmcraft_platform::quiesce();
    let refused = va::work_refused_while_exiting();
    assert!(surface.sync().is_err(), "a sync started while exiting");
    assert!(surface.read((0, 0, 16, 16), |_, _, _| {}).is_err(), "an image read started while exiting");
    assert!(surface.write(|_, _, _| Ok(())).is_err(), "an image write started while exiting");
    assert!(surface.export().is_err(), "a DMA-BUF export started while exiting");
    assert!(pool.get().is_err(), "a surface was made while exiting");
    assert!(va::work_refused_while_exiting() >= refused + 5, "refusals not counted");
}
