//! Settings ▸ Playback ▸ Graphics device (LINUX-PLAN L2.6): which GPU draws the interface and the
//! monitors on a computer with more than one (a laptop's integrated and discrete GPUs, a desktop's
//! iGPU next to a card). Auto keeps wgpu's high-performance choice and the drivers' own overrides
//! (`DRI_PRIME`, `MESA_VK_DEVICE_SELECT`, `WGPU_POWER_PREF`); a chosen GPU is matched by its PCI
//! ids, so a driver update that renames it doesn't lose it. Applied when the window's GPU device
//! is made, at start ([`crate::wgpu_configuration_on`]). Exports choose their GPUs themselves.

use eframe::wgpu;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Most GPUs listed (a hostile or odd system can't grow Settings without bound).
const MAX_GPUS: usize = 16;

/// A GPU as Settings lists it; `id` is what `playback.graphicsDevice` stores.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GpuChoice {
    pub id: String,
    pub name: String,
    /// `discrete` | `integrated` | `virtual` | `software` | `other`.
    pub kind: String,
}

/// The preference value for a GPU with these PCI ids (`1002:73df`).
pub fn id(vendor: u32, device: u32) -> String {
    format!("{vendor:04x}:{device:04x}")
}

/// The `(vendor, device)` a preference names; `None`: Auto (empty, or not two hex ids).
pub fn parse(pref: &str) -> Option<(u32, u32)> {
    let (v, d) = pref.trim().split_once(':')?;
    let hex = |s: &str| (!s.is_empty() && s.len() <= 8).then(|| u32::from_str_radix(s, 16).ok()).flatten();
    Some((hex(v)?, hex(d)?))
}

/// Which of `adapters` (their `(vendor, device)` ids) the preference picks: `None` for Auto, or
/// when that GPU isn't there (removed, or another computer's preferences). Two identical cards:
/// the first.
pub fn pick(adapters: &[(u32, u32)], pref: &str) -> Option<usize> {
    let want = parse(pref)?;
    adapters.iter().position(|a| *a == want)
}

/// Auto among `types` when the preferred GPU is missing: a discrete GPU, else an integrated one,
/// else the first that isn't a software renderer, else the first.
pub fn auto_index(types: &[wgpu::DeviceType]) -> Option<usize> {
    use wgpu::DeviceType as T;
    types
        .iter()
        .position(|t| *t == T::DiscreteGpu)
        .or_else(|| types.iter().position(|t| *t == T::IntegratedGpu))
        .or_else(|| types.iter().position(|t| *t != T::Cpu))
        .or_else(|| (!types.is_empty()).then_some(0))
}

fn kind(t: wgpu::DeviceType) -> &'static str {
    match t {
        wgpu::DeviceType::DiscreteGpu => "discrete",
        wgpu::DeviceType::IntegratedGpu => "integrated",
        wgpu::DeviceType::VirtualGpu => "virtual",
        wgpu::DeviceType::Cpu => "software",
        wgpu::DeviceType::Other => "other",
    }
}

/// The backends the window's device is made on: Vulkan on Linux (DMA-BUF import), the primary
/// ones elsewhere.
pub fn backends() -> wgpu::Backends {
    if cfg!(target_os = "linux") { wgpu::Backends::VULKAN } else { wgpu::Backends::PRIMARY }
}

/// The GPUs of `infos`, once each (in the order wgpu lists them), at most [`MAX_GPUS`].
pub fn choices(infos: &[wgpu::AdapterInfo]) -> Vec<GpuChoice> {
    let mut out: Vec<GpuChoice> = Vec::new();
    for i in infos {
        let id = id(i.vendor, i.device);
        if out.len() >= MAX_GPUS || out.iter().any(|c| c.id == id) {
            continue;
        }
        let name: String = i.name.chars().filter(|c| !c.is_control()).take(120).collect();
        out.push(GpuChoice { id, name, kind: kind(i.device_type).to_string() });
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
static LIST: std::sync::OnceLock<Vec<GpuChoice>> = std::sync::OnceLock::new();

/// List the GPUs on a thread of its own (a second GPU waking up or a slow driver must not hold
/// the UI); [`listed`] has them once it's done.
#[cfg(not(target_arch = "wasm32"))]
pub fn warm() {
    if LIST.get().is_none()
        && let Err(e) = std::thread::Builder::new().name("gpu-list".into()).spawn(|| {
            let _ = list();
        })
    {
        log::warn!("could not list the GPUs: {e}");
    }
}

/// The GPUs [`warm`] has listed so far: none while it's still listing (Settings shows Auto and
/// the stored choice; reopening Settings shows the rest).
#[cfg(not(target_arch = "wasm32"))]
pub fn listed() -> Vec<GpuChoice> {
    LIST.get().cloned().unwrap_or_default()
}

/// This computer's GPUs, listed once per run (making an instance and listing the adapters takes
/// tens of milliseconds, more when a sleeping GPU wakes up; call it off the UI thread). A driver
/// that fails while listing: no GPUs (Settings shows Auto only).
#[cfg(not(target_arch = "wasm32"))]
pub fn list() -> Vec<GpuChoice> {
    LIST.get_or_init(|| {
        std::panic::catch_unwind(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: backends(),
                flags: wgpu::InstanceFlags::from_build_config().with_env(),
                backend_options: wgpu::BackendOptions::from_env_or_default(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                display: None,
            });
            let adapters = pollster::block_on(instance.enumerate_adapters(backends()));
            choices(&adapters.iter().map(wgpu::Adapter::get_info).collect::<Vec<_>>())
        })
        .unwrap_or_else(|_| {
            log::warn!("listing the GPUs failed; Settings offers Auto only");
            Vec::new()
        })
    })
    .clone()
}

/// The Settings options: Auto first, then each GPU; a chosen GPU that isn't there is kept (so
/// Settings shows what's stored) and marked.
pub fn options(gpus: &[GpuChoice], current: &str) -> Vec<(String, String)> {
    let mut v = vec![(String::new(), tl!("Auto").to_string())];
    v.extend(gpus.iter().map(|g| {
        let kind = match g.kind.as_str() {
            "discrete" => tl!("discrete"),
            "integrated" => tl!("integrated"),
            "virtual" => tl!("virtual"),
            "software" => tl!("software"),
            _ => tl!("other"),
        };
        (g.id.clone(), format!("{} ({kind})", g.name))
    }));
    if !current.is_empty() && !gpus.iter().any(|g| g.id == current) {
        let gpu: String = current.chars().filter(|c| !c.is_control()).take(40).collect();
        v.push((current.to_string(), tlf!("{gpu} (not found: Auto)", gpu).to_string()));
    }
    v
}

/// `graphics.listDevices`: the GPUs, the stored choice and the GPU drawing now. `None`: another id.
pub(crate) fn route(app: &mut crate::FilmcraftApp, id: &str) -> Option<Result<Value, String>> {
    if id != "graphics.listDevices" {
        return None;
    }
    let gpus = app.hooks.graphics_devices.as_mut().map(|f| f()).unwrap_or_default();
    let in_use = app.gpu.as_ref().map(|g| {
        let i = g.render_state.adapter.get_info();
        json!({"id": id_of(&i), "name": i.name})
    });
    Some(Ok(json!({"devices": gpus, "current": app.session.prefs.playback.graphics_device, "inUse": in_use})))
}

fn id_of(i: &wgpu::AdapterInfo) -> String {
    id(i.vendor, i.device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_preference_values_mean_auto() {
        assert_eq!(parse("1002:73df"), Some((0x1002, 0x73df)));
        assert_eq!(parse(" 8086:4680 "), Some((0x8086, 0x4680)));
        for bad in ["", "auto", "1002", ":73df", "1002:", "zz:73df", "1002:73df:1", "123456789:1", "-1:2", "１002:73df"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_chosen_gpu_is_picked_by_its_ids_and_a_missing_one_is_auto() {
        let gpus = [(0x8086, 0x4680), (0x1002, 0x73df)];
        assert_eq!(pick(&gpus, "1002:73df"), Some(1));
        assert_eq!(pick(&gpus, "8086:4680"), Some(0));
        assert_eq!(pick(&gpus, ""), None);
        assert_eq!(pick(&gpus, "10de:2684"), None);
        assert_eq!(pick(&[], "1002:73df"), None);
        use wgpu::DeviceType as T;
        assert_eq!(auto_index(&[T::Cpu, T::IntegratedGpu, T::DiscreteGpu]), Some(2));
        assert_eq!(auto_index(&[T::Cpu, T::IntegratedGpu]), Some(1));
        assert_eq!(auto_index(&[T::Cpu, T::Other]), Some(1));
        assert_eq!(auto_index(&[T::Cpu]), Some(0));
        assert_eq!(auto_index(&[]), None);
    }

    #[test]
    fn settings_options_keep_a_missing_choice_visible() {
        let gpus = vec![GpuChoice { id: "1002:73df".into(), name: "AMD Radeon RX 6700 XT".into(), kind: "discrete".into() }];
        let o = options(&gpus, "");
        assert_eq!(o.len(), 2);
        assert_eq!(o[0].0, "");
        assert_eq!(o[1].0, "1002:73df");
        let o = options(&gpus, "10de:2684");
        assert_eq!(o.len(), 3);
        assert_eq!(o[2].0, "10de:2684");
        assert!(o[2].1.contains("10de:2684"));
    }
}
