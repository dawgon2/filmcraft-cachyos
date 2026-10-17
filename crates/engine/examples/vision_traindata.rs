//! Synthetic training frames for the picture-model tune (fork AI-PLAN AI4 item 3, FORK-PLAN §1a.96 (c)).
//!
//! ```text
//! cargo run --release -p filmcraft-engine --example vision_traindata -- \
//!     --out-dir ~/Apps/filmcraft-train/runs/vision-data-v1 [--frames 3000] [--seed 1] [--val 0.1] [--review 40]
//! ```
//!
//! Draws frames whose lighting is known because it is made here: a scene (interior lamp, daylight
//! exterior, sunset, overcast, night street, interview, bright beach / snow) in scene-linear light,
//! then what the camera did to it (exposure in stops, a white-balance error, a saturation change),
//! then how it was recorded: display-referred (Rec. 709-like gamma) or a camera log curve (F-Log2
//! most often, the other [`LogCurve`]s too), as an 8-bit PNG. Each frame gets a question about one
//! topic (exposure, white balance, skin, sky, log, the light) and a target answer: one or two plain
//! sentences of what the frame shows that bears on it, written from the known scene, no advice.
//!
//! **Labels are checked against the scopes:** every frame is measured with
//! [`PictureFacts::measure`] (scene-linear, i.e. after the input transform for log frames) and a
//! frame whose exposure or white-balance words disagree with the measurement is dropped (counted
//! in `data-stats.json`). The prompt is the app's own [`look_prompt`], so training sees what the
//! app sends.
//!
//! Output (in `--out-dir`): `frames/<id>.png`, `train.jsonl` / `val.jsonl` (split by frame), one
//! object per line `{"id", "image", "question", "topic", "prompt", "answer", "scene", "encoding",
//! "measured"}`, `data-stats.json`, and `review.md`: a random sample of frames with their labels
//! for the owner's read (`docs/fork/coach-eval/VISION-READ.md`, OWNER-QUESTIONS Q44). Frames stay
//! local (never in git). Deterministic for a given seed. Offline tooling: not part of the app.

use std::collections::BTreeMap;
use std::error::Error;
use std::io::Write as _;
use std::path::PathBuf;

use filmcraft_color::log::normalized_from_signal;
use filmcraft_color::{LogCurve, Range, WorkingSpace};
use filmcraft_engine::coach_assist::look_prompt;
use filmcraft_render::Image;
use filmcraft_render::picture_facts::PictureFacts;

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f(&mut self) -> f32 {
        ((self.next() >> 40) as f32) / ((1u64 << 24) as f32)
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        let n = v.len();
        if n == 0 { None } else { v.get((self.next() % n as u64) as usize) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scene {
    InteriorLamp,
    Daylight,
    Sunset,
    Overcast,
    NightStreet,
    Interview,
    BrightBeach,
}

const SCENES: [Scene; 7] = [Scene::InteriorLamp, Scene::Daylight, Scene::Sunset, Scene::Overcast, Scene::NightStreet, Scene::Interview, Scene::BrightBeach];

impl Scene {
    fn name(self) -> &'static str {
        match self {
            Scene::InteriorLamp => "interior-lamp",
            Scene::Daylight => "daylight",
            Scene::Sunset => "sunset",
            Scene::Overcast => "overcast",
            Scene::NightStreet => "night-street",
            Scene::Interview => "interview",
            Scene::BrightBeach => "bright-beach",
        }
    }
}

/// What was drawn, in words the answers use.
struct Drawn {
    scene: Scene,
    /// Sentence about the setting and its light sources.
    setting: String,
    /// Where the face is lit from, when there is a face.
    face: Option<&'static str>,
    sky: Option<&'static str>,
}

/// Linear-light canvas.
struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, px: vec![[0.0; 3]; w.saturating_mul(h)] }
    }
    fn each(&mut self, mut f: impl FnMut(f32, f32, &mut [f32; 3])) {
        let (w, h) = (self.w.max(1) as f32, self.h.max(1) as f32);
        for (i, p) in self.px.iter_mut().enumerate() {
            let (x, y) = ((i % self.w.max(1)) as f32 / w, (i / self.w.max(1)) as f32 / h);
            f(x, y, p);
        }
    }
    /// Vertical gradient from `top` to `bottom` between y0 and y1.
    fn gradient(&mut self, y0: f32, y1: f32, top: [f32; 3], bottom: [f32; 3]) {
        self.each(|_, y, p| {
            if y >= y0 && y < y1 {
                let t = ((y - y0) / (y1 - y0).max(1e-6)).clamp(0.0, 1.0);
                *p = [0, 1, 2].map(|c| top[c] + (bottom[c] - top[c]) * t);
            }
        });
    }
    /// Filled ellipse with a soft edge, shaded brighter towards `lit_from_x` (−1 left, 1 right, 0 flat).
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, rgb: [f32; 3], lit_from_x: f32) {
        self.each(|x, y, p| {
            let (dx, dy) = ((x - cx) / rx.max(1e-6), (y - cy) / ry.max(1e-6));
            let d = (dx * dx + dy * dy).sqrt();
            if d < 1.0 {
                let a = ((1.0 - d) * 8.0).clamp(0.0, 1.0);
                let shade = (1.0 + 0.6 * lit_from_x * dx).clamp(0.25, 1.8);
                *p = [0, 1, 2].map(|c| p[c] * (1.0 - a) + rgb[c] * shade * a);
            }
        });
    }
    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, rgb: [f32; 3]) {
        self.each(|x, y, p| {
            if x >= x0 && x < x1 && y >= y0 && y < y1 {
                *p = rgb;
            }
        });
    }
    /// Additive light blob (a practical light or the sun).
    fn glow(&mut self, cx: f32, cy: f32, r: f32, rgb: [f32; 3]) {
        self.each(|x, y, p| {
            let d2 = ((x - cx) * (x - cx) + (y - cy) * (y - cy)) / (r * r).max(1e-9);
            let k = (-d2).exp();
            for c in 0..3 {
                p[c] += rgb[c] * k;
            }
        });
    }
}

fn skin(r: &mut Rng) -> [f32; 3] {
    // light to dark skin reflectances (linear), all near the skin line
    *r.pick(&[[0.50, 0.30, 0.21], [0.42, 0.24, 0.16], [0.30, 0.17, 0.11], [0.18, 0.10, 0.065], [0.10, 0.055, 0.037]]).unwrap_or(&[0.4, 0.24, 0.16])
}

fn draw(c: &mut Canvas, scene: Scene, r: &mut Rng) -> Drawn {
    let side = if r.chance(0.5) { -1.0 } else { 1.0 };
    let side_word = if side < 0.0 { "left" } else { "right" };
    let mut face = None;
    let mut sky = None;
    let face_at = |c: &mut Canvas, r: &mut Rng, light: f32, lit: f32| {
        let s = skin(r).map(|v| v * light);
        let cx = r.range(0.35, 0.65);
        c.ellipse(cx, 0.45, 0.09, 0.16, s, lit);
        c.ellipse(cx, 0.85, 0.2, 0.2, [0.05, 0.05, 0.06].map(|v| v * light), 0.0);
    };
    let setting = match scene {
        Scene::InteriorLamp => {
            let wall = r.range(0.12, 0.25);
            c.rect(0.0, 0.0, 1.0, 1.0, [wall, wall * 0.97, wall * 0.9]);
            c.rect(0.0, 0.75, 1.0, 1.0, [0.06, 0.04, 0.03]);
            let lx = if side < 0.0 { 0.15 } else { 0.85 };
            c.glow(lx, 0.4, 0.08, [3.0, 1.6, 0.6]);
            c.glow(lx, 0.45, 0.35, [0.25, 0.13, 0.04]);
            if r.chance(0.7) {
                face_at(c, r, 1.0, side);
                face = Some(side_word);
            }
            format!("An interior: a warm orange table lamp on the {side_word} lights a plain wall")
        }
        Scene::Daylight => {
            c.gradient(0.0, 0.55, [0.18, 0.32, 0.75], [0.55, 0.65, 0.85]);
            c.gradient(0.55, 1.0, [0.12, 0.2, 0.06], [0.07, 0.1, 0.04]);
            sky = Some("a clear blue sky");
            if r.chance(0.5) {
                face_at(c, r, 1.6, side);
                face = Some(side_word);
            }
            "An exterior in daylight: a clear blue sky over green ground, the sun high".to_string()
        }
        Scene::Sunset => {
            c.gradient(0.0, 0.6, [0.12, 0.05, 0.2], [1.2, 0.5, 0.12]);
            c.rect(0.0, 0.6, 1.0, 1.0, [0.015, 0.012, 0.012]);
            c.glow(r.range(0.2, 0.8), 0.58, 0.05, [4.0, 1.8, 0.4]);
            sky = Some("a sunset sky, purple above and orange at the horizon");
            "A sunset: a low sun at the horizon, the sky purple above and orange below, the ground in dark silhouette".to_string()
        }
        Scene::Overcast => {
            c.gradient(0.0, 0.5, [0.55, 0.57, 0.6], [0.45, 0.47, 0.5]);
            c.gradient(0.5, 1.0, [0.1, 0.11, 0.08], [0.07, 0.07, 0.06]);
            sky = Some("a flat grey overcast sky");
            if r.chance(0.5) {
                face_at(c, r, 0.9, 0.0);
                face = Some("evenly, from the sky");
            }
            "An overcast exterior: a flat grey sky and soft, even light with no shadows".to_string()
        }
        Scene::NightStreet => {
            c.rect(0.0, 0.0, 1.0, 1.0, [0.004, 0.004, 0.006]);
            c.rect(0.0, 0.7, 1.0, 1.0, [0.02, 0.018, 0.016]);
            for _ in 0..3 {
                let x = r.range(0.05, 0.95);
                c.glow(x, r.range(0.15, 0.35), 0.04, [2.5, 1.2, 0.25]);
                c.glow(x, 0.75, 0.15, [0.08, 0.04, 0.01]);
            }
            let neon = r.chance(0.5);
            if neon {
                c.rect(0.6, 0.3, 0.85, 0.36, [2.0, 0.15, 1.2]);
                c.glow(0.72, 0.33, 0.15, [0.15, 0.01, 0.09]);
            }
            if neon {
                "A night street: orange street lamps and a pink neon sign, the rest close to black".to_string()
            } else {
                "A night street: orange street lamps in a dark scene, most of it close to black".to_string()
            }
        }
        Scene::Interview => {
            let g = r.range(0.1, 0.2);
            c.rect(0.0, 0.0, 1.0, 1.0, [g, g, g]);
            face_at(c, r, 1.3, side);
            face = Some(side_word);
            format!("An interview against a plain grey backdrop, the key light from the {side_word}")
        }
        Scene::BrightBeach => {
            c.gradient(0.0, 0.45, [0.4, 0.6, 1.0], [0.8, 0.9, 1.1]);
            c.gradient(0.45, 1.0, [0.95, 0.85, 0.65], [0.85, 0.75, 0.55]);
            sky = Some("a pale blue sky");
            "A bright beach: pale sand and a pale blue sky in strong sunlight".to_string()
        }
    };
    Drawn { scene, setting, face, sky }
}

/// What the camera did: exposure (stops), warm / cool error (−1…1, + warm), tint (+ magenta), saturation.
#[derive(Clone, Copy, Debug)]
struct Camera {
    stops: f32,
    warm: f32,
    tint: f32,
    sat: f32,
}

fn apply_camera(c: &mut Canvas, cam: Camera, r: &mut Rng) {
    let gain = 2f32.powf(cam.stops);
    let (wr, wb) = (1.0 + 0.45 * cam.warm, 1.0 - 0.45 * cam.warm);
    let g = 1.0 - 0.25 * cam.tint;
    let mut seed = r.next();
    for p in &mut c.px {
        let v = [p[0] * gain * wr, p[1] * gain * g, p[2] * gain * wb];
        let y = 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        // light noise, deterministic
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let n = 1.0 + ((seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.04;
        *p = [0, 1, 2].map(|i| ((y + (v[i] - y) * cam.sat) * n).max(0.0));
    }
}

#[derive(Clone, Copy, Debug)]
enum Encoding {
    Display,
    Log(LogCurve),
}

impl Encoding {
    fn name(self) -> String {
        match self {
            Encoding::Display => "display".to_string(),
            Encoding::Log(l) => l.label().to_string(),
        }
    }
    fn encode(self, v: f32) -> f32 {
        match self {
            // Rec. 709-like: gamma 1 / 2.4 with a hard clip at white
            Encoding::Display => v.clamp(0.0, 1.0).powf(1.0 / 2.4),
            Encoding::Log(l) => normalized_from_signal(l.encode(f64::from(v.max(0.0))), Range::Full, l.domain()).clamp(0.0, 1.0) as f32,
        }
    }
}

fn exposure_words(cam: Camera, enc: Encoding, f: &PictureFacts) -> Option<&'static str> {
    // the label from what the camera did, kept only when the scopes agree
    if cam.stops <= -1.2 {
        (f.mids_stops < -0.8).then_some("underexposed: the picture is dark overall")
    } else if cam.stops >= 1.2 {
        let clipped = matches!(enc, Encoding::Display) && f.clipped > 0.02;
        (f.mids_stops > 0.6 || clipped).then_some(if clipped { "overexposed: bright, with clipped white highlights" } else { "overexposed: bright overall" })
    } else {
        None
    }
}

fn cast_words(cam: Camera, f: &PictureFacts) -> Option<String> {
    let warm = if cam.warm >= 0.25 {
        Some("a warm orange cast")
    } else if cam.warm <= -0.25 {
        Some("a cool blue cast")
    } else {
        None
    };
    let tint = if cam.tint >= 0.3 {
        Some("a magenta tint")
    } else if cam.tint <= -0.3 {
        Some("a green tint")
    } else {
        None
    };
    // the measured cast must point the same way (unless too few areas are neutral to measure);
    // a frame labelled neutral must not measure strongly cast (a lamp-lit wall, a sunset)
    let m = f.cast_words();
    if f.neutral_share >= 0.02 {
        let disagrees = match warm {
            Some(w) if w.contains("warm") => !m.contains("warm"),
            Some(_) => !m.contains("cool"),
            None => m.contains("very "),
        };
        if disagrees {
            return None;
        }
    }
    match (warm, tint) {
        (None, None) => Some(String::new()),
        (Some(w), None) => Some(w.to_string()),
        (None, Some(t)) => Some(t.to_string()),
        (Some(w), Some(t)) => Some(format!("{w} and {t}")),
    }
}

struct Args {
    out_dir: PathBuf,
    frames: usize,
    seed: u64,
    val: f32,
    review: usize,
}

fn parse_args() -> Result<Args, Box<dyn Error>> {
    let mut a = Args { out_dir: PathBuf::new(), frames: 3000, seed: 1, val: 0.1, review: 40 };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let v = it.next().ok_or_else(|| format!("{k} needs a value"))?;
        match k.as_str() {
            "--out-dir" => a.out_dir = PathBuf::from(v),
            "--frames" => a.frames = v.parse::<usize>()?.min(100_000),
            "--seed" => a.seed = v.parse()?,
            "--val" => a.val = v.parse::<f32>()?.clamp(0.0, 0.5),
            "--review" => a.review = v.parse::<usize>()?.min(200),
            _ => return Err(format!("unknown option {k}").into()),
        }
    }
    if a.out_dir.as_os_str().is_empty() {
        return Err("--out-dir is required".into());
    }
    Ok(a)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args()?;
    let frames_dir = args.out_dir.join("frames");
    std::fs::create_dir_all(&frames_dir)?;
    let mut r = Rng(args.seed ^ 0x7669_7369_6f6e_3031);
    let mut train = std::io::BufWriter::new(std::fs::File::create(args.out_dir.join("train.jsonl"))?);
    let mut val = std::io::BufWriter::new(std::fs::File::create(args.out_dir.join("val.jsonl"))?);
    let mut dropped: BTreeMap<&str, usize> = BTreeMap::new();
    let mut topics: BTreeMap<String, usize> = BTreeMap::new();
    let mut encodings: BTreeMap<String, usize> = BTreeMap::new();
    let mut kept = Vec::new();
    let (mut n_train, mut n_val) = (0usize, 0usize);
    let sizes = [(512usize, 288usize), (512, 384), (448, 448), (384, 512)];
    for i in 0..args.frames {
        let scene = *r.pick(&SCENES).unwrap_or(&Scene::Daylight);
        let (w, h) = *r.pick(&sizes).unwrap_or(&(512, 288));
        let mut c = Canvas::new(w, h);
        let drawn = draw(&mut c, scene, &mut r);
        let cam = Camera {
            stops: if r.chance(0.5) { r.range(-0.6, 0.6) } else { *r.pick(&[-2.5f32, -1.8, -1.3, 1.3, 1.8, 2.3]).unwrap_or(&0.0) + r.range(-0.2, 0.2) },
            warm: if r.chance(0.5) { r.range(-0.12, 0.12) } else { r.range(-0.8, 0.8) },
            tint: if r.chance(0.7) { r.range(-0.15, 0.15) } else { r.range(-0.8, 0.8) },
            sat: if r.chance(0.6) { r.range(0.85, 1.15) } else { *r.pick(&[0.35f32, 0.5, 1.5, 1.8]).unwrap_or(&1.0) },
        };
        // where the light's own colour is the picture (a sunset, sodium lamps) a white-balance
        // error doesn't read as a cast, so those scenes only get small ones
        let cam = if matches!(scene, Scene::Sunset | Scene::NightStreet) {
            Camera { warm: cam.warm.clamp(-0.12, 0.12), tint: cam.tint.clamp(-0.15, 0.15), ..cam }
        } else {
            cam
        };
        apply_camera(&mut c, cam, &mut r);
        let enc = if r.chance(0.65) {
            Encoding::Display
        } else if r.chance(0.45) {
            Encoding::Log(LogCurve::FLog2)
        } else {
            Encoding::Log(*r.pick(&LogCurve::ALL).unwrap_or(&LogCurve::FLog))
        };
        // measured in scene-linear light (for log: after the input transform)
        let img = Image { w, h, px: c.px.iter().flat_map(|p| [p[0], p[1], p[2], 1.0]).collect() };
        let Some(facts) = PictureFacts::measure(&img, WorkingSpace::Rec709) else {
            *dropped.entry("unmeasurable").or_default() += 1;
            continue;
        };
        let expo = exposure_words(cam, enc, &facts);
        if expo.is_none() && cam.stops.abs() >= 1.2 {
            *dropped.entry("exposure label disagrees with the scopes").or_default() += 1;
            continue;
        }
        let Some(cast) = cast_words(cam, &facts) else {
            *dropped.entry("cast label disagrees with the scopes").or_default() += 1;
            continue;
        };
        // the topics this frame can be asked about
        let mut options: Vec<(&str, &str)> =
            vec![("light", "what light is in this shot?"), ("exposure", "is this shot exposed right?"), ("white-balance", "is the white balance off?")];
        if drawn.face.is_some() {
            options.push(("skin", "do the skin tones look right?"));
        }
        if drawn.sky.is_some() {
            options.push(("sky", "how does the sky look?"));
        }
        if matches!(enc, Encoding::Log(_)) {
            options.push(("log", "why does my footage look so flat and grey?"));
            options.push(("log", "why does my footage look so flat and grey?"));
        }
        let Some(&(topic, question)) = r.pick(&options) else { continue };
        // (which log curve can't be seen in the pixels, so the label never names one)
        let log_line = matches!(enc, Encoding::Log(_))
            .then(|| "It looks flat and washed out, like log footage as the camera recorded it, before any input transform".to_string());
        let sat_line = if cam.sat <= 0.6 {
            Some("the colours are muted")
        } else if cam.sat >= 1.4 {
            Some("the colours are very saturated")
        } else {
            None
        };
        let second = match topic {
            "exposure" => Some(expo.map_or("The exposure looks about right".to_string(), |e| format!("It is {e}"))),
            "white-balance" => Some(if cast.is_empty() {
                "The neutral areas look neutral, no clear colour cast".to_string()
            } else {
                format!("The neutral areas show {cast}")
            }),
            "skin" => drawn.face.map(|f| {
                let lit = if f.starts_with("evenly") { format!("lit {f}") } else { format!("lit from the {f}") };
                if cast.is_empty() {
                    format!("There is one face, {lit}, the skin close to natural")
                } else {
                    format!("There is one face, {lit}; the skin takes on {cast}")
                }
            }),
            "sky" => drawn.sky.map(|s| format!("The sky is {s}")),
            "log" => log_line.clone(),
            _ => expo.map(|e| format!("It is {e}")).or(sat_line.map(|s| format!("Also, {s}"))),
        };
        let mut answer = format!("{}.", drawn.setting);
        if let Some(s) = second {
            answer.push(' ');
            answer.push_str(&s);
            answer.push('.');
        }
        // what else is clearly wrong is said too, briefly (a label must not call a blue-cast sunset
        // simply orange)
        let mut also = Vec::new();
        if !matches!(topic, "white-balance" | "skin") && !cast.is_empty() {
            also.push(format!("there is {cast} over everything"));
        }
        if !matches!(topic, "exposure" | "light")
            && let Some(e) = expo
        {
            also.push(format!("it is {e}"));
        }
        if !also.is_empty() {
            answer.push_str(&format!(" Also, {}.", also.join(", and ")));
        }
        // a log frame is always said to be log (it is the first thing that matters)
        if topic != "log"
            && let Some(l) = &log_line
        {
            answer.push(' ');
            answer.push_str(l);
            answer.push('.');
        }
        let id = format!("v{:05}-{}", i, scene.name());
        let rel = format!("frames/{id}.png");
        let bytes: Vec<u8> = c.px.iter().flat_map(|p| p.map(|v| (enc.encode(v) * 255.0).round().clamp(0.0, 255.0) as u8)).collect();
        let png = image::RgbImage::from_raw(w as u32, h as u32, bytes).ok_or("frame buffer size")?;
        png.save(args.out_dir.join(&rel))?;
        let row = serde_json::json!({
            "id": id, "image": rel, "question": question, "topic": topic, "prompt": look_prompt(question),
            "answer": answer, "scene": drawn.scene.name(), "encoding": enc.name(), "measured": facts.sheet(),
            "camera": {"stops": cam.stops, "warm": cam.warm, "tint": cam.tint, "sat": cam.sat},
        });
        let line = serde_json::to_string(&row)?;
        if r.f() < args.val {
            writeln!(val, "{line}")?;
            n_val += 1;
        } else {
            writeln!(train, "{line}")?;
            n_train += 1;
        }
        *topics.entry(topic.to_string()).or_default() += 1;
        *encodings.entry(enc.name()).or_default() += 1;
        kept.push(row);
    }
    train.flush()?;
    val.flush()?;
    // the review sample: evenly spaced over what was kept
    let mut md = String::from(
        "# Picture-model training frames: review sample\n\nSynthetic (generated with known lighting). Frames are local, relative to this file's run folder.\n\n",
    );
    let step = (kept.len() / args.review.max(1)).max(1);
    for row in kept.iter().step_by(step).take(args.review) {
        let s = |k: &str| row.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        md.push_str(&format!(
            "## {}\n\n- Frame: `{}` ({}, {})\n- Question: {}\n- Label: {}\n- Measured: {}\n- Label right? [ ] yes  [ ] no, because:\n\n",
            s("id"),
            s("image"),
            s("scene"),
            s("encoding"),
            s("question"),
            s("answer"),
            s("measured").replace('\n', " / ")
        ));
    }
    std::fs::write(args.out_dir.join("review.md"), md)?;
    let stats = serde_json::json!({"train": n_train, "val": n_val, "dropped": dropped, "topics": topics, "encodings": encodings, "seed": args.seed});
    std::fs::write(args.out_dir.join("data-stats.json"), serde_json::to_string_pretty(&stats)?)?;
    println!("{}", serde_json::to_string(&stats)?);
    Ok(())
}
