//! The system's light or dark appearance for Settings ▸ Appearance ▸ Appearance Mode ▸ Sync with
//! system, on Linux desktops where winit reports none (Wayland compositors send no theme), and the
//! desktop's accent colour for Settings ▸ Appearance ▸ Use the system accent color and its
//! reduced-motion wish for Settings ▸ Appearance ▸ Animations ▸ Sync with system (L1.4), and its
//! text size for Settings ▸ Appearance ▸ Match the desktop's text size ([`TEXT_KEYS`]).
//!
//! The XDG desktop portal is read once (`org.freedesktop.portal.Settings.Read` of
//! `org.freedesktop.appearance` / `color-scheme`, `accent-color` and `reduced-motion`, and the
//! desktops' own animation keys, [`MOTION_KEYS`]), then its `SettingChanged`
//! signal is followed on a worker thread, which wakes the UI only when a value changes. There is no
//! polling. Without a portal answer for the colour scheme, `gsettings` runs once at start-up, by
//! absolute path. Elsewhere the host returns no service and the UI uses what egui reports.

#[cfg(target_os = "linux")]
const PORTAL_NAMESPACE: &str = "org.freedesktop.appearance";
#[cfg(target_os = "linux")]
const PORTAL_KEY: &str = "color-scheme";
#[cfg(target_os = "linux")]
const ACCENT_KEY: &str = "accent-color";

/// The settings that mean "reduce motion", in the portal's `Read` namespaces: the portal's own
/// key (portal 1.20+), KDE's animation speed (System Settings ▸ Workspace ▸ General Behavior ▸
/// Animation speed ▸ Instant is 0) and GNOME's `enable-animations` (Settings ▸ Accessibility ▸
/// Seeing ▸ Reduce Animation on GNOME 46+, GNOME Tweaks before). When the portal's own key
/// answers it alone decides ([`motion_wanted`]); otherwise any desktop key asking is enough.
#[cfg(target_os = "linux")]
const MOTION_KEYS: [(&str, &str); 3] =
    [(PORTAL_NAMESPACE, "reduced-motion"), ("org.kde.kdeglobals.KDE", "AnimationDurationFactor"), ("org.gnome.desktop.interface", "enable-animations")];

/// The desktop's text size, in the portal's `Read` namespaces: KDE's General font (System Settings
/// ▸ Text & Fonts; a `QFont` string whose second field is the point size, 10 pt by default; the
/// portal has no answer while it is the default) and GNOME's `text-scaling-factor` (Settings ▸
/// Accessibility ▸ Large Text, GNOME Tweaks ▸ Fonts ▸ Scaling Factor). KDE's answer wins when both
/// give one ([`text_factor`]). GNOME's factor counts only on native Wayland: under X11 GNOME's
/// settings daemon already folds it into Xft/DPI, which winit reports as the system scale, so
/// using it again would apply it twice ([`gnome_text_counts`]).
#[cfg(target_os = "linux")]
const TEXT_KEYS: [(&str, &str); 2] = [("org.kde.kdeglobals.General", "font"), ("org.gnome.desktop.interface", "text-scaling-factor")];

/// KDE's default General font size, in points.
#[cfg(target_os = "linux")]
const KDE_DEFAULT_POINTS: f64 = 10.0;

/// The text factor `TEXT_KEYS[source]` reports (1.0 = the desktop's default size), or `None` for
/// anything malformed or outside 0.5 … 4 (a broken setting, not a wish for a huge interface).
/// Plausible factors are passed on as they are; the UI holds them to
/// `filmcraft_engine::settings::TEXT_SCALE_RANGE` (a 6 pt font gives 75 %, a 3 pt one nothing).
#[cfg(target_os = "linux")]
fn text_factor(source: usize, value: zbus::zvariant::OwnedValue) -> Option<f32> {
    use zbus::zvariant::Value;
    let factor = match (source, unwrap_variant(value)?) {
        (0, Value::Str(s)) => s.as_str().split(',').nth(1)?.trim().parse::<f64>().ok()? / KDE_DEFAULT_POINTS,
        (1, Value::F64(f)) => f,
        (1, Value::Str(s)) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (factor.is_finite() && (0.5..=4.0).contains(&factor)).then_some(factor as f32)
}

/// A text factor packed for an `AtomicU32`: 0 = none, else the `f32` bits (never 0: factors are
/// at least 0.5).
#[cfg(target_os = "linux")]
fn pack_text(f: Option<f32>) -> u32 {
    f.map_or(0, f32::to_bits)
}

#[cfg(target_os = "linux")]
fn unpack_text(v: u32) -> Option<f32> {
    (v != 0).then(|| f32::from_bits(v)).filter(|f| f.is_finite())
}

/// Whether FilmCraft's window is on native Wayland (winit picks Wayland when one of these is set),
/// where GNOME's text factor isn't already in the system scale.
#[cfg(target_os = "linux")]
fn gnome_text_counts() -> bool {
    ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"].iter().any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
}

#[cfg(target_os = "linux")]
fn portal_proxy() -> Option<zbus::blocking::Proxy<'static>> {
    let conn = zbus::blocking::connection::Builder::session().ok()?.method_timeout(std::time::Duration::from_secs(2)).build().ok()?;
    zbus::blocking::Proxy::new(&conn, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings").ok()
}

#[cfg(target_os = "linux")]
fn portal_theme(proxy: &zbus::blocking::Proxy<'_>) -> Option<egui::Theme> {
    let value: zbus::zvariant::OwnedValue = proxy.call("Read", &(PORTAL_NAMESPACE, PORTAL_KEY)).ok()?;
    decode(portal_code(value)?)
}

/// The desktop's accent colour as the portal reports it now.
#[cfg(target_os = "linux")]
fn portal_accent_now(proxy: &zbus::blocking::Proxy<'_>) -> Option<[u8; 3]> {
    portal_accent(proxy.call("Read", &(PORTAL_NAMESPACE, ACCENT_KEY)).ok()?)
}

/// The value inside a portal reply. `Read` returns a variant, and some portals nest another
/// variant inside it (KDE's does).
#[cfg(target_os = "linux")]
fn unwrap_variant(value: zbus::zvariant::OwnedValue) -> Option<zbus::zvariant::Value<'static>> {
    let mut value: zbus::zvariant::Value<'static> = value.into();
    for _ in 0..4 {
        match value {
            zbus::zvariant::Value::Value(inner) => value = *inner,
            other => return Some(other),
        }
    }
    None
}

/// Whether the setting `MOTION_KEYS[source]` asks for reduced motion. Unknown or malformed values
/// don't. KDE's portal sends kdeglobals entries as strings, so numbers are also read from text.
#[cfg(target_os = "linux")]
fn motion_reduced(source: usize, value: zbus::zvariant::OwnedValue) -> bool {
    use zbus::zvariant::Value;
    let value = unwrap_variant(value);
    match (source, value) {
        (0, Some(v)) => u32::try_from(v).is_ok_and(|code| code == 1),
        (1, Some(Value::F64(f))) => factor_is_instant(f),
        (1, Some(Value::Str(s))) => s.as_str().trim().parse::<f64>().is_ok_and(factor_is_instant),
        (2, Some(Value::Bool(enabled))) => !enabled,
        (2, Some(Value::Str(s))) => s.as_str().trim() == "false",
        _ => false,
    }
}

/// KDE's `AnimationDurationFactor`: 0 is "Instant" (animations off); NaN is not.
#[cfg(target_os = "linux")]
fn factor_is_instant(f: f64) -> bool {
    (0.0..0.01).contains(&f.abs())
}

/// One bit per [`MOTION_KEYS`] source.
#[cfg(target_os = "linux")]
fn motion_bits(bits: u8, source: usize, reduced: bool) -> u8 {
    let Some(bit) = u32::try_from(source).ok().and_then(|s| 1u8.checked_shl(s)) else { return bits };
    if reduced { bits | bit } else { bits & !bit }
}

/// Set in the motion bits once the portal's own `reduced-motion` key has answered.
#[cfg(target_os = "linux")]
const PORTAL_ANSWERED: u8 = 0x80;

/// Whether the motion bits ask for reduced motion: the portal's own key when it answered (a
/// stale desktop key from another backend can't override it), else any desktop key.
#[cfg(target_os = "linux")]
fn motion_wanted(bits: u8) -> bool {
    if bits & PORTAL_ANSWERED != 0 { bits & 1 != 0 } else { bits & 0b110 != 0 }
}

/// The `u32` inside a portal reply.
#[cfg(target_os = "linux")]
fn portal_code(value: zbus::zvariant::OwnedValue) -> Option<u32> {
    u32::try_from(unwrap_variant(value)?).ok()
}

/// The portal's `accent-color`: an sRGB `(ddd)` triple in 0 … 1. A value outside that range means
/// the desktop has no accent colour (the portal specification's "unset").
#[cfg(target_os = "linux")]
fn portal_accent(value: zbus::zvariant::OwnedValue) -> Option<[u8; 3]> {
    let zbus::zvariant::Value::Structure(s) = unwrap_variant(value)? else { return None };
    let [zbus::zvariant::Value::F64(r), zbus::zvariant::Value::F64(g), zbus::zvariant::Value::F64(b)] = s.fields() else { return None };
    let channel = |c: f64| (0.0..=1.0).contains(&c).then(|| (c * 255.0).round() as u8);
    Some([channel(*r)?, channel(*g)?, channel(*b)?])
}

/// An accent colour packed for an `AtomicU32`: 0 = none, else `0x01rrggbb`.
#[cfg(target_os = "linux")]
fn pack_accent(c: Option<[u8; 3]>) -> u32 {
    c.map_or(0, |[r, g, b]| 0x0100_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
}

#[cfg(target_os = "linux")]
fn unpack_accent(v: u32) -> Option<[u8; 3]> {
    (v & 0x0100_0000 != 0).then(|| {
        let [_, r, g, b] = v.to_be_bytes();
        [r, g, b]
    })
}

/// The portal's `color-scheme`: 0 no preference, 1 prefer dark, 2 prefer light.
#[cfg(target_os = "linux")]
fn decode(code: u32) -> Option<egui::Theme> {
    match code {
        1 => Some(egui::Theme::Dark),
        2 => Some(egui::Theme::Light),
        _ => None,
    }
}

/// One-shot fallback for desktops without the settings portal: runs once, by absolute path.
#[cfg(target_os = "linux")]
fn gtk_theme() -> Option<egui::Theme> {
    let exe = ["/usr/bin/gsettings", "/bin/gsettings", "/usr/local/bin/gsettings"].into_iter().find(|p| std::path::Path::new(p).is_file())?;
    let output = std::process::Command::new(exe).args(["get", "org.gnome.desktop.interface", "color-scheme"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_gtk_scheme(std::str::from_utf8(&output.stdout).ok()?)
}

#[cfg(target_os = "linux")]
fn parse_gtk_scheme(s: &str) -> Option<egui::Theme> {
    match s.trim().trim_matches('\'') {
        "prefer-dark" => Some(egui::Theme::Dark),
        "prefer-light" => Some(egui::Theme::Light),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn code_of(theme: Option<egui::Theme>) -> u8 {
    match theme {
        Some(egui::Theme::Dark) => 1,
        Some(egui::Theme::Light) => 2,
        None => 0,
    }
}

/// Publish initial and subsequent answers alike: the initial Read may finish after the UI's
/// bounded startup wait, so it must also wake an already-created context.
#[cfg(target_os = "linux")]
fn publish(value: &std::sync::atomic::AtomicU8, code: u8, repaint: impl FnOnce()) {
    if value.swap(code, std::sync::atomic::Ordering::Relaxed) != code {
        repaint();
    }
}

/// [`publish`] for a packed accent colour or text factor.
#[cfg(target_os = "linux")]
fn publish_u32(value: &std::sync::atomic::AtomicU32, packed: u32, repaint: impl FnOnce()) {
    if value.swap(packed, std::sync::atomic::Ordering::Relaxed) != packed {
        repaint();
    }
}

/// The host hooks `HostHooks::system_theme` and `HostHooks::system_accent`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Service {
    pub theme: filmcraft_ui_egui::SystemThemeFn,
    pub accent: filmcraft_ui_egui::SystemAccentFn,
    pub reduced_motion: filmcraft_ui_egui::SystemReducedMotionFn,
    pub text_scale: filmcraft_ui_egui::SystemTextScaleFn,
}

/// The host hooks, or `None` where egui's own report is used and there is no accent colour.
pub fn service() -> Option<Service> {
    #[cfg(target_os = "linux")]
    {
        use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
        use std::sync::{Arc, OnceLock};
        let value = Arc::new(AtomicU8::new(0));
        let accent = Arc::new(AtomicU32::new(0));
        let motion = Arc::new(AtomicU8::new(0));
        // one packed factor per TEXT_KEYS source
        let text: Arc<[AtomicU32; 2]> = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
        let wake: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
        let (worker_value, worker_accent, worker_motion, worker_wake) = (Arc::clone(&value), Arc::clone(&accent), Arc::clone(&motion), Arc::clone(&wake));
        let worker_text = Arc::clone(&text);
        let (ready, wait) = std::sync::mpsc::channel();
        let worker = move || {
            let repaint = || {
                if let Some(ctx) = worker_wake.get() {
                    ctx.request_repaint();
                }
            };
            let proxy = portal_proxy();
            // Subscribe before Read so a change during startup cannot be missed.
            let signals = proxy.as_ref().and_then(|p| p.receive_signal("SettingChanged").ok());
            let initial = proxy.as_ref().and_then(portal_theme).or_else(gtk_theme);
            publish(&worker_value, code_of(initial), repaint);
            let initial_accent = proxy.as_ref().and_then(portal_accent_now);
            publish_u32(&worker_accent, pack_accent(initial_accent), repaint);
            let _ = ready.send(());
            // After `ready`: the first frame waits for the theme and accent only.
            let mut motion_now = 0u8;
            if let Some(p) = proxy.as_ref() {
                for (source, (namespace, key)) in MOTION_KEYS.iter().enumerate() {
                    let read: zbus::Result<zbus::zvariant::OwnedValue> = p.call("Read", &(*namespace, *key));
                    let Ok(v) = read else { continue };
                    if source == 0 {
                        motion_now |= PORTAL_ANSWERED;
                    }
                    motion_now = motion_bits(motion_now, source, motion_reduced(source, v));
                }
            }
            publish(&worker_motion, motion_now, repaint);
            if let Some(p) = proxy.as_ref() {
                for (source, ((namespace, key), slot)) in TEXT_KEYS.iter().zip(worker_text.iter()).enumerate() {
                    let read: zbus::Result<zbus::zvariant::OwnedValue> = p.call("Read", &(*namespace, *key));
                    let factor = read.ok().and_then(|v| text_factor(source, v));
                    publish_u32(slot, pack_text(factor), repaint);
                }
            }
            let Some(signals) = signals else { return };
            // Sleeps in the signal iterator: no polling.
            for message in signals {
                let Ok((namespace, key, changed)) = message.body().deserialize::<(String, String, zbus::zvariant::OwnedValue)>() else {
                    continue;
                };
                if let Some(source) = MOTION_KEYS.iter().position(|(n, k)| *n == namespace && *k == key) {
                    if source == 0 {
                        motion_now |= PORTAL_ANSWERED;
                    }
                    motion_now = motion_bits(motion_now, source, motion_reduced(source, changed));
                    publish(&worker_motion, motion_now, repaint);
                    continue;
                }
                if let Some(source) = TEXT_KEYS.iter().position(|(n, k)| *n == namespace && *k == key) {
                    if let Some(slot) = worker_text.get(source) {
                        publish_u32(slot, pack_text(text_factor(source, changed)), repaint);
                    }
                    continue;
                }
                if namespace != PORTAL_NAMESPACE {
                    continue;
                }
                if key == PORTAL_KEY {
                    publish(&worker_value, code_of(portal_code(changed).and_then(decode)), repaint);
                } else if key == ACCENT_KEY {
                    publish_u32(&worker_accent, pack_accent(portal_accent(changed)), repaint);
                }
            }
        };
        let spawned = std::thread::Builder::new().name("appearance-portal".into()).spawn(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(worker)).is_err() {
                log::warn!("the system appearance watcher stopped; Sync with system keeps the last value");
            }
        });
        if spawned.is_err() {
            return None;
        }
        // The first reading, so the first frame already shows the right theme (bounded wait).
        let _ = wait.recv_timeout(std::time::Duration::from_millis(250));
        let (accent_wake, motion_wake, text_wake) = (Arc::clone(&wake), Arc::clone(&wake), Arc::clone(&wake));
        let text_sources = if gnome_text_counts() { TEXT_KEYS.len() } else { 1 };
        Some(Service {
            theme: Box::new(move |ctx: &egui::Context| {
                let _ = wake.set(ctx.clone());
                decode(u32::from(value.load(Ordering::Relaxed)))
            }),
            accent: Box::new(move |ctx: &egui::Context| {
                let _ = accent_wake.set(ctx.clone());
                unpack_accent(accent.load(Ordering::Relaxed))
            }),
            reduced_motion: Box::new(move |ctx: &egui::Context| {
                let _ = motion_wake.set(ctx.clone());
                motion_wanted(motion.load(Ordering::Relaxed))
            }),
            text_scale: Box::new(move |ctx: &egui::Context| {
                let _ = text_wake.set(ctx.clone());
                text.iter().take(text_sources).find_map(|slot| unpack_text(slot.load(Ordering::Relaxed)))
            }),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn initial_and_signal_answers_repaint_only_on_changes() {
        use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
        let repaints = AtomicUsize::new(0);
        let value = AtomicU8::new(0);
        let repaint = || {
            repaints.fetch_add(1, Ordering::Relaxed);
        };
        // A late initial Read takes the same publication path as later signals.
        super::publish(&value, 2, repaint);
        assert_eq!(repaints.load(Ordering::Relaxed), 1);
        super::publish(&value, 2, repaint);
        assert_eq!(repaints.load(Ordering::Relaxed), 1);
        super::publish(&value, 1, repaint);
        assert_eq!(repaints.load(Ordering::Relaxed), 2);
        assert_eq!(value.load(Ordering::Relaxed), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portal_and_gsettings_values_decode() {
        assert_eq!(super::decode(0), None);
        assert_eq!(super::decode(1), Some(egui::Theme::Dark));
        assert_eq!(super::decode(2), Some(egui::Theme::Light));
        assert_eq!(super::decode(9), None);
        let nested = zbus::zvariant::Value::Value(Box::new(zbus::zvariant::Value::Value(Box::new(zbus::zvariant::Value::U32(2)))));
        assert_eq!(super::portal_code(zbus::zvariant::OwnedValue::try_from(nested).unwrap()), Some(2));
        let plain = zbus::zvariant::OwnedValue::from(1u32);
        assert_eq!(super::portal_code(plain), Some(1));
        let wrong = zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::from("dark")).unwrap();
        assert_eq!(super::portal_code(wrong), None);
        assert_eq!(super::parse_gtk_scheme("'prefer-light'\n"), Some(egui::Theme::Light));
        assert_eq!(super::parse_gtk_scheme("'prefer-dark'\n"), Some(egui::Theme::Dark));
        assert_eq!(super::parse_gtk_scheme("'default'\n"), None);
        assert_eq!(super::parse_gtk_scheme(""), None);
        assert_eq!((super::code_of(None), super::code_of(Some(egui::Theme::Dark)), super::code_of(Some(egui::Theme::Light))), (0, 1, 2));
    }

    /// The portal's `accent-color` as KDE sends it (a `(ddd)` in two variants), hostile values
    /// refused without panicking (L1.4).
    #[cfg(target_os = "linux")]
    #[test]
    fn accent_colors_decode_and_pack() {
        use zbus::zvariant::{OwnedValue, StructureBuilder, Value};
        let triple = |r: f64, g: f64, b: f64| Value::Structure(StructureBuilder::new().add_field(r).add_field(g).add_field(b).build().unwrap());
        let kde = Value::Value(Box::new(Value::Value(Box::new(triple(0.239216, 0.682353, 0.913725)))));
        assert_eq!(super::portal_accent(OwnedValue::try_from(kde).unwrap()), Some([61, 174, 233]));
        assert_eq!(super::portal_accent(OwnedValue::try_from(triple(0.0, 1.0, 0.5)).unwrap()), Some([0, 255, 128]));
        // out of range means "unset"; NaN and infinities too
        for (r, g, b) in [(-1.0, 0.5, 0.5), (0.5, 1.5, 0.5), (f64::NAN, 0.5, 0.5), (0.5, 0.5, f64::INFINITY)] {
            assert_eq!(super::portal_accent(OwnedValue::try_from(triple(r, g, b)).unwrap()), None, "{r} {g} {b}");
        }
        // wrong shapes
        let two = Value::Structure(StructureBuilder::new().add_field(0.5).add_field(0.5).build().unwrap());
        assert_eq!(super::portal_accent(OwnedValue::try_from(two).unwrap()), None);
        let ints = Value::Structure(StructureBuilder::new().add_field(1u32).add_field(2u32).add_field(3u32).build().unwrap());
        assert_eq!(super::portal_accent(OwnedValue::try_from(ints).unwrap()), None);
        assert_eq!(super::portal_accent(OwnedValue::from(1u32)), None);
        let deep = (0..8).fold(triple(0.5, 0.5, 0.5), |v, _| Value::Value(Box::new(v)));
        assert_eq!(super::portal_accent(OwnedValue::try_from(deep).unwrap()), None, "nesting is bounded");
        for c in [None, Some([0, 0, 0]), Some([61, 174, 233]), Some([255, 255, 255])] {
            assert_eq!(super::unpack_accent(super::pack_accent(c)), c);
        }
    }

    /// Reduced motion as KDE, GNOME and the portal send it; hostile values never ask for it and
    /// never panic (L1.4).
    #[cfg(target_os = "linux")]
    #[test]
    fn reduced_motion_values_decode() {
        use zbus::zvariant::{OwnedValue, Value};
        let v = |value: Value<'static>| OwnedValue::try_from(Value::Value(Box::new(value))).unwrap();
        assert!(super::motion_reduced(0, v(Value::U32(1))));
        assert!(!super::motion_reduced(0, v(Value::U32(0))));
        assert!(!super::motion_reduced(0, v(Value::U32(7))));
        // KDE: the factor as text (kdeglobals) or a double; 0 is Instant
        assert!(super::motion_reduced(1, v(Value::from("0"))));
        assert!(super::motion_reduced(1, v(Value::F64(0.0))));
        assert!(!super::motion_reduced(1, v(Value::from("0.5"))));
        assert!(!super::motion_reduced(1, v(Value::F64(1.0))));
        for hostile in ["", "NaN", "inf", "-0.5", "zero"] {
            assert!(!super::motion_reduced(1, v(Value::from(hostile))), "{hostile:?}");
        }
        assert!(!super::motion_reduced(1, v(Value::F64(f64::NAN))));
        let kde_nested = OwnedValue::try_from(Value::Value(Box::new(Value::Value(Box::new(Value::from("0")))))).unwrap();
        assert!(super::motion_reduced(1, kde_nested));
        // GNOME: enable-animations false means reduce
        assert!(super::motion_reduced(2, v(Value::Bool(false))));
        assert!(!super::motion_reduced(2, v(Value::Bool(true))));
        assert!(super::motion_reduced(2, v(Value::from("false"))));
        assert!(!super::motion_reduced(2, v(Value::from("true"))));
        // a source that doesn't exist, the wrong type
        assert!(!super::motion_reduced(9, v(Value::U32(1))));
        assert!(!super::motion_reduced(2, v(Value::U32(0))));
        // one bit per source; any source asking is enough, clearing one keeps the others
        let bits = super::motion_bits(super::motion_bits(0, 1, true), 2, true);
        assert_eq!(super::motion_bits(bits, 1, false), 0b100);
        assert_eq!(super::motion_bits(bits, 99, true), bits, "out of range source is ignored");
        assert_eq!(super::motion_bits(0, usize::MAX, true), 0);
        // the portal's own key, once it answered, decides over a stale desktop key
        assert!(super::motion_wanted(0b100));
        assert!(!super::motion_wanted(super::PORTAL_ANSWERED | 0b100));
        assert!(super::motion_wanted(super::PORTAL_ANSWERED | 0b001));
        assert!(!super::motion_wanted(0));
    }

    /// The desktop's text size as KDE (a `QFont` string) and GNOME (a factor) send it; hostile
    /// values give no factor and never panic (L1.4, Q15 b).
    #[cfg(target_os = "linux")]
    #[test]
    fn text_sizes_decode() {
        use zbus::zvariant::{OwnedValue, Value};
        let v = |value: Value<'static>| OwnedValue::try_from(Value::Value(Box::new(value))).unwrap();
        assert_eq!(super::text_factor(0, v(Value::from("Noto Sans,11,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"))), Some(1.1));
        assert_eq!(super::text_factor(0, v(Value::from("Noto Sans,10.5,-1,5,50,0,0,0,0,0"))), Some(1.05));
        assert_eq!(super::text_factor(1, v(Value::F64(1.25))), Some(1.25));
        assert_eq!(super::text_factor(1, v(Value::from("1.5"))), Some(1.5));
        let nested = OwnedValue::try_from(Value::Value(Box::new(Value::Value(Box::new(Value::from("Inter,12")))))).unwrap();
        assert_eq!(super::text_factor(0, nested), Some(1.2));
        // a pixel-sized font (-1 points), garbage, out of range, the wrong type or source
        for hostile in ["Noto Sans,-1,14,5,400", "Noto Sans", "", ",", "Noto Sans,NaN", "Noto Sans,inf", "Noto Sans,400", "Noto Sans,1"] {
            assert_eq!(super::text_factor(0, v(Value::from(hostile))), None, "{hostile:?}");
        }
        for hostile in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.2, 9.0] {
            assert_eq!(super::text_factor(1, v(Value::F64(hostile))), None, "{hostile}");
        }
        assert_eq!(super::text_factor(1, v(Value::U32(1))), None);
        assert_eq!(super::text_factor(0, v(Value::F64(1.0))), None);
        assert_eq!(super::text_factor(9, v(Value::F64(1.0))), None);
        for f in [None, Some(0.5), Some(1.1), Some(4.0)] {
            assert_eq!(super::unpack_text(super::pack_text(f)), f);
        }
        assert_eq!(super::unpack_text(f32::NAN.to_bits()), None);
    }
}
