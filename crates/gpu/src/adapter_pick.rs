//! Which GPU FilmCraft draws and exports on (fork Phase XPS a, LINUX-AUDIT L2.6): one rule for the
//! window, the DMA-BUF interop device, the headless export devices and the export renderer, so a
//! laptop with an integrated and a discrete GPU (the Dell XPS 16: Intel Arc iGPU + RTX 4050) never
//! draws on one GPU and exports on the other by accident.
//!
//! Settings ▸ Playback ▸ Graphics device (`playback.graphicsDevice`) stores one of: `""` (Auto),
//! `"power-saving"`, `"performance"` or a GPU's PCI ids (`"10de:28a1"`, matched by ids so a driver
//! update that renames the GPU doesn't lose it). Auto keeps the drivers' own overrides: with
//! `DRI_PRIME`, `__NV_PRIME_RENDER_OFFLOAD` or `MESA_VK_DEVICE_SELECT` set, the GPU the driver
//! lists first wins (Mesa's device-select layer and NVIDIA's PRIME offload layer put the chosen
//! GPU first); otherwise a discrete GPU, as FilmCraft has always done on desktops.
//!
//! Provisional (XPS, before the laptop): Auto on a hybrid laptop is the discrete GPU, as on a
//! desktop; the laptop runs (docs/fork/XPS-SETUP.md) measure whether Auto should prefer the
//! integrated GPU on battery.

use std::sync::RwLock;

/// What a preference asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuPref {
    /// The drivers' override when one is set, else a discrete GPU.
    #[default]
    Auto,
    /// An integrated GPU first (longer battery life, less heat).
    PowerSaving,
    /// A discrete GPU first.
    Performance,
    /// This GPU (PCI vendor, device); Auto when it isn't there.
    Pci(u32, u32),
}

/// The stored value for [`GpuPref::PowerSaving`].
pub const POWER_SAVING: &str = "power-saving";
/// The stored value for [`GpuPref::Performance`].
pub const PERFORMANCE: &str = "performance";

impl GpuPref {
    /// The preference a stored value names; anything else (empty, hostile, a number) is Auto.
    pub fn parse(s: &str) -> GpuPref {
        let s = s.trim();
        match s {
            POWER_SAVING => GpuPref::PowerSaving,
            PERFORMANCE => GpuPref::Performance,
            _ => parse_pci(s).map_or(GpuPref::Auto, |(v, d)| GpuPref::Pci(v, d)),
        }
    }

    /// The value Settings stores for this preference.
    pub fn stored(self) -> String {
        match self {
            GpuPref::Auto => String::new(),
            GpuPref::PowerSaving => POWER_SAVING.to_string(),
            GpuPref::Performance => PERFORMANCE.to_string(),
            GpuPref::Pci(v, d) => format!("{v:04x}:{d:04x}"),
        }
    }
}

/// The `(vendor, device)` PCI ids a value names (`1002:73df`); `None` unless it is exactly two
/// hex numbers of at most eight digits.
pub fn parse_pci(s: &str) -> Option<(u32, u32)> {
    let (v, d) = s.trim().split_once(':')?;
    let hex = |s: &str| (!s.is_empty() && s.len() <= 8 && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u32::from_str_radix(s, 16).ok()).flatten();
    Some((hex(v)?, hex(d)?))
}

/// An adapter as the rule sees it (from `wgpu::AdapterInfo`, or a fake list in tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterDesc {
    pub vendor: u32,
    pub device: u32,
    pub kind: wgpu::DeviceType,
}

impl From<&wgpu::AdapterInfo> for AdapterDesc {
    fn from(i: &wgpu::AdapterInfo) -> Self {
        AdapterDesc { vendor: i.vendor, device: i.device, kind: i.device_type }
    }
}

/// Whether the environment hands the choice to the driver: `DRI_PRIME`, `__NV_PRIME_RENDER_OFFLOAD`
/// or `MESA_VK_DEVICE_SELECT` set to anything but empty (`WGPU_ADAPTER_NAME` is wgpu's own and is
/// read by wgpu where it applies).
pub fn driver_override_from(get: impl Fn(&str) -> Option<String>) -> bool {
    ["DRI_PRIME", "__NV_PRIME_RENDER_OFFLOAD", "MESA_VK_DEVICE_SELECT"].iter().any(|k| get(k).is_some_and(|v| !v.trim().is_empty()))
}

/// [`driver_override_from`] on this process's environment.
pub fn driver_override() -> bool {
    driver_override_from(|k| std::env::var(k).ok())
}

/// Which of `adapters` (in the order the driver lists them) `pref` picks. `driver_override`:
/// [`driver_override`]. A named GPU that isn't there, or Auto: the driver's first GPU when it
/// overrides, else as [`GpuPref::Performance`]. Software renderers come last in every case.
/// `None` only for an empty list.
pub fn pick_adapter(pref: GpuPref, adapters: &[AdapterDesc], driver_override: bool) -> Option<usize> {
    use wgpu::DeviceType as T;
    let first = |kind: T| adapters.iter().position(|a| a.kind == kind);
    let hardware = || adapters.iter().position(|a| a.kind != T::Cpu);
    let any = || (!adapters.is_empty()).then_some(0);
    let performance = || first(T::DiscreteGpu).or_else(|| first(T::IntegratedGpu)).or_else(hardware).or_else(any);
    match pref {
        GpuPref::Pci(v, d) => adapters.iter().position(|a| a.vendor == v && a.device == d).or_else(|| pick_adapter(GpuPref::Auto, adapters, driver_override)),
        GpuPref::PowerSaving => first(T::IntegratedGpu).or_else(|| first(T::DiscreteGpu)).or_else(hardware).or_else(any),
        GpuPref::Performance => performance(),
        GpuPref::Auto if driver_override => hardware().or_else(any),
        GpuPref::Auto => performance(),
    }
}

static PREF: RwLock<GpuPref> = RwLock::new(GpuPref::Auto);

/// The process's preference, set at start from Settings (like the window's GPU, a change in
/// Settings applies at the next start: the export renderer keeps the device it made).
pub fn set_preference(pref: GpuPref) {
    *PREF.write().unwrap_or_else(std::sync::PoisonError::into_inner) = pref;
}

/// The preference [`set_preference`] last set (Auto before that).
pub fn preference() -> GpuPref {
    *PREF.read().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The adapter of `instance` the process's preference picks, among `backends`.
#[cfg(not(target_arch = "wasm32"))]
pub fn choose(instance: &wgpu::Instance, backends: wgpu::Backends) -> Option<wgpu::Adapter> {
    let adapters = pollster::block_on(instance.enumerate_adapters(backends));
    let descs: Vec<AdapterDesc> = adapters.iter().map(|a| AdapterDesc::from(&a.get_info())).collect();
    let i = pick_adapter(preference(), &descs, driver_override())?;
    adapters.into_iter().nth(i)
}

/// The PCI vendor of the GPU [`choose`] picks on all backends (what exports render on), listed
/// once per run; `None` without a GPU. Encoders use it to prefer the same GPU's video engine.
#[cfg(not(target_arch = "wasm32"))]
pub fn chosen_vendor() -> Option<u32> {
    static VENDOR: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *VENDOR.get_or_init(|| {
        std::panic::catch_unwind(|| {
            if wgpu::Instance::enabled_backend_features().is_empty() {
                return None;
            }
            let instance = wgpu::Instance::default();
            choose(&instance, wgpu::Backends::PRIMARY).map(|a| a.get_info().vendor)
        })
        .unwrap_or(None)
    })
}

/// On the web the browser picks the GPU: `None` (the caller asks wgpu for its default).
#[cfg(target_arch = "wasm32")]
pub fn choose(_instance: &wgpu::Instance, _backends: wgpu::Backends) -> Option<wgpu::Adapter> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::DeviceType as T;

    const INTEL_ARC: AdapterDesc = AdapterDesc { vendor: 0x8086, device: 0x7d55, kind: T::IntegratedGpu };
    const RTX_4050: AdapterDesc = AdapterDesc { vendor: 0x10de, device: 0x28a1, kind: T::DiscreteGpu };
    const LLVMPIPE: AdapterDesc = AdapterDesc { vendor: 0x10005, device: 0, kind: T::Cpu };

    #[test]
    fn stored_values_round_trip_and_hostile_ones_are_auto() {
        for p in [GpuPref::Auto, GpuPref::PowerSaving, GpuPref::Performance, GpuPref::Pci(0x10de, 0x28a1)] {
            assert_eq!(GpuPref::parse(&p.stored()), p);
        }
        assert_eq!(GpuPref::parse(" 8086:7d55 "), GpuPref::Pci(0x8086, 0x7d55));
        for bad in ["", "auto", "Power-Saving", "1002", ":73df", "1002:", "zz:73df", "1002:73df:1", "123456789:1", "-1:2", "+1:2", "１002:73df"] {
            assert_eq!(GpuPref::parse(bad), GpuPref::Auto, "{bad}");
        }
    }

    /// The XPS 16 as Vulkan lists it in both orders (NVIDIA's ICD may come first or second), with
    /// Mesa's software renderer as well.
    #[test]
    fn xps16_layout_every_preference() {
        let layouts: [&[AdapterDesc]; 2] = [&[INTEL_ARC, RTX_4050, LLVMPIPE], &[LLVMPIPE, RTX_4050, INTEL_ARC]];
        for l in layouts {
            let at = |want: AdapterDesc| l.iter().position(|a| *a == want);
            assert_eq!(pick_adapter(GpuPref::Auto, l, false), at(RTX_4050));
            assert_eq!(pick_adapter(GpuPref::Performance, l, false), at(RTX_4050));
            assert_eq!(pick_adapter(GpuPref::PowerSaving, l, false), at(INTEL_ARC));
            assert_eq!(pick_adapter(GpuPref::Pci(0x8086, 0x7d55), l, false), at(INTEL_ARC));
            // a GPU from another computer's preferences: Auto
            assert_eq!(pick_adapter(GpuPref::Pci(0x1002, 0x73df), l, false), at(RTX_4050));
            // explicit choices beat the driver override; Auto follows the driver's first GPU
            assert_eq!(pick_adapter(GpuPref::PowerSaving, l, true), at(INTEL_ARC));
            assert_eq!(pick_adapter(GpuPref::Auto, l, true), l.iter().position(|a| a.kind != T::Cpu));
        }
    }

    #[test]
    fn single_and_odd_layouts() {
        // this desktop: RX 6700 XT + UHD 770
        let amd = AdapterDesc { vendor: 0x1002, device: 0x73df, kind: T::DiscreteGpu };
        let uhd = AdapterDesc { vendor: 0x8086, device: 0x4680, kind: T::IntegratedGpu };
        assert_eq!(pick_adapter(GpuPref::Auto, &[uhd, amd], false), Some(1));
        assert_eq!(pick_adapter(GpuPref::PowerSaving, &[uhd, amd], false), Some(0));
        // only one GPU: every preference takes it
        for p in [GpuPref::Auto, GpuPref::PowerSaving, GpuPref::Performance, GpuPref::Pci(1, 2)] {
            assert_eq!(pick_adapter(p, &[INTEL_ARC], false), Some(0));
            assert_eq!(pick_adapter(p, &[LLVMPIPE, RTX_4050], true), Some(1));
            assert_eq!(pick_adapter(p, &[LLVMPIPE], false), Some(0));
            assert_eq!(pick_adapter(p, &[], false), None);
        }
        let other = AdapterDesc { vendor: 7, device: 7, kind: T::Other };
        assert_eq!(pick_adapter(GpuPref::PowerSaving, &[LLVMPIPE, other], false), Some(1));
        // two identical cards: the first
        assert_eq!(pick_adapter(GpuPref::Pci(0x10de, 0x28a1), &[RTX_4050, RTX_4050], false), Some(0));
    }

    #[test]
    fn driver_overrides_are_read_from_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert!(!driver_override_from(env(&[])));
        assert!(!driver_override_from(env(&[("DRI_PRIME", ""), ("MESA_VK_DEVICE_SELECT", "  ")])));
        assert!(driver_override_from(env(&[("DRI_PRIME", "1")])));
        assert!(driver_override_from(env(&[("__NV_PRIME_RENDER_OFFLOAD", "1")])));
        assert!(driver_override_from(env(&[("MESA_VK_DEVICE_SELECT", "8086:7d55")])));
    }

    #[test]
    fn the_process_preference_is_shared() {
        set_preference(GpuPref::PowerSaving);
        assert_eq!(preference(), GpuPref::PowerSaving);
        set_preference(GpuPref::Auto);
        assert_eq!(preference(), GpuPref::Auto);
    }
}
