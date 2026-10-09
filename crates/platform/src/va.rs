//! In-process VA-API (libva) for hardware decoding on Linux (docs/adr/0002-linux-video-interop.md).
//!
//! libva is opened at run time ([`crate::va_sys`], `libloading`), so FilmCraft builds and runs
//! without it: [`Display::get`] then reports why, and callers fall back. Everything here is a safe
//! wrapper with RAII handles: a [`Display`] on the AMD (else first working) render node, decoder
//! [`Config`]s and [`Context`]s, a [`SurfacePool`] whose [`Surface`]s go back to the pool when the
//! last holder drops them, parameter / slice [`Buffer`]s, and downloading a surface to CPU memory.
//! No panic crosses into libva: every call returns a `Result` with libva's error string.

use std::ffi::CStr;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::va_sys as sys;

/// The loaded library (`libva-drm.so.2`, which brings `libva.so.2`).
fn lib() -> Result<&'static sys::Va, String> {
    static LIB: OnceLock<Result<sys::Va, String>> = OnceLock::new();
    LIB.get_or_init(|| {
        let mut last = String::new();
        for name in ["libva-drm.so.2", "libva-drm.so"] {
            // SAFETY: loading libva-drm runs its (and libva's) initialisers, which only set up
            // static state; `--dynamic-link-require-all` makes `new` fail unless every declared
            // function resolves, so no call can reach a missing symbol.
            match unsafe { sys::Va::new(name) } {
                Ok(l) => return Ok(l),
                Err(e) => last = e.to_string(),
            }
        }
        Err(format!("libva is not installed ({last})"))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// libva's description of a status code.
fn status_text(l: &sys::Va, st: sys::VAStatus) -> String {
    // SAFETY: vaErrorStr takes any status value and returns a pointer to a static string (or null).
    let p = unsafe { l.vaErrorStr(st) };
    if p.is_null() {
        return format!("VA status {st}");
    }
    // SAFETY: non-null, points to a NUL-terminated static string owned by libva.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn check(l: &sys::Va, st: sys::VAStatus, what: &str) -> Result<(), String> {
    if st == 0 { Ok(()) } else { Err(format!("{what}: {}", status_text(l, st))) }
}

/// An initialised VA display on a DRM render node.
pub struct Display {
    lib: &'static sys::Va,
    dpy: sys::VADisplay,
    /// The render node, open for as long as the display lives.
    _file: std::fs::File,
    pub node: String,
    pub vendor: String,
    profiles: Vec<(sys::VAProfile, Vec<sys::VAEntrypoint>)>,
}

// SAFETY: a VADisplay may be used from any thread: libva and Mesa's VA frontend serialise access to
// a display internally (per-display mutex). FilmCraft never shares a context between threads
// without its own lock.
unsafe impl Send for Display {}
// SAFETY: as for `Send`; `&Display` only issues thread-safe libva calls.
unsafe impl Sync for Display {}

impl Drop for Display {
    fn drop(&mut self) {
        // SAFETY: `dpy` was initialised by vaInitialize and every object made on it holds an
        // `Arc<Display>`, so none outlives this call.
        unsafe { self.lib.vaTerminate(self.dpy) };
    }
}

impl Display {
    /// The process's display: `FILMCRAFT_VAAPI_DEVICE` when set, else the first render node
    /// (AMD, NVIDIA, other, Intel) that initialises. Opened once.
    pub fn get() -> Result<Arc<Display>, String> {
        static D: OnceLock<Result<Arc<Display>, String>> = OnceLock::new();
        D.get_or_init(|| {
            let lib = lib()?;
            let nodes = match std::env::var("FILMCRAFT_VAAPI_DEVICE").ok().filter(|v| !v.is_empty()) {
                Some(n) => vec![n],
                None => crate::ffmpeg::render_nodes(),
            };
            let mut why = Vec::new();
            for node in nodes {
                match Display::open(lib, &node) {
                    Ok(d) => {
                        log::info!("VA-API: {} on {}", d.vendor, d.node);
                        return Ok(Arc::new(d));
                    }
                    Err(e) => why.push(format!("{node}: {e}")),
                }
            }
            Err(if why.is_empty() { "no DRM render node".to_string() } else { why.join("; ") })
        })
        .clone()
    }

    fn open(lib: &'static sys::Va, node: &str) -> Result<Display, String> {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(node).map_err(|e| e.to_string())?;
        // SAFETY: the fd is a valid open DRM node, kept open by `file` for the display's lifetime.
        let dpy = unsafe { lib.vaGetDisplayDRM(file.as_raw_fd()) };
        if dpy.is_null() {
            return Err("vaGetDisplayDRM failed".into());
        }
        let (mut major, mut minor) = (0, 0);
        // SAFETY: `dpy` is a fresh display; the out-pointers are valid locals.
        let st = unsafe { lib.vaInitialize(dpy, &mut major, &mut minor) };
        if st != 0 {
            let e = status_text(lib, st);
            // SAFETY: terminating a display whose initialisation failed is allowed and frees it.
            unsafe { lib.vaTerminate(dpy) };
            return Err(format!("vaInitialize: {e}"));
        }
        // SAFETY: initialised display; returns a static string or null.
        let vp = unsafe { lib.vaQueryVendorString(dpy) };
        // SAFETY: non-null pointers from vaQueryVendorString are NUL-terminated strings owned by libva.
        let vendor = if vp.is_null() { String::new() } else { unsafe { CStr::from_ptr(vp) }.to_string_lossy().into_owned() };
        let mut d = Display { lib, dpy, _file: file, node: node.to_string(), vendor, profiles: Vec::new() };
        d.profiles = d.query_profiles()?;
        Ok(d)
    }

    fn query_profiles(&self) -> Result<Vec<(sys::VAProfile, Vec<sys::VAEntrypoint>)>, String> {
        let l = self.lib;
        // SAFETY: initialised display.
        let max = unsafe { l.vaMaxNumProfiles(self.dpy) }.clamp(0, 256) as usize;
        let mut profiles = vec![0 as sys::VAProfile; max];
        let mut n = 0;
        // SAFETY: the array holds vaMaxNumProfiles entries, as libva requires.
        check(l, unsafe { l.vaQueryConfigProfiles(self.dpy, profiles.as_mut_ptr(), &mut n) }, "vaQueryConfigProfiles")?;
        profiles.truncate((n.max(0) as usize).min(max));
        // SAFETY: initialised display.
        let max_ep = unsafe { l.vaMaxNumEntrypoints(self.dpy) }.clamp(0, 64) as usize;
        let mut out = Vec::new();
        for p in profiles {
            let mut eps = vec![0 as sys::VAEntrypoint; max_ep];
            let mut m = 0;
            // SAFETY: the array holds vaMaxNumEntrypoints entries.
            if unsafe { l.vaQueryConfigEntrypoints(self.dpy, p, eps.as_mut_ptr(), &mut m) } == 0 {
                eps.truncate((m.max(0) as usize).min(max_ep));
                out.push((p, eps));
            }
        }
        Ok(out)
    }

    /// Whether the driver has `profile` with `entrypoint`.
    pub fn supports(&self, profile: sys::VAProfile, entrypoint: sys::VAEntrypoint) -> bool {
        self.profiles.iter().any(|(p, eps)| *p == profile && eps.contains(&entrypoint))
    }

    fn check(&self, st: sys::VAStatus, what: &str) -> Result<(), String> {
        check(self.lib, st, what)
    }
}

/// A decoder configuration (profile + entrypoint + render-target format).
pub struct Config {
    display: Arc<Display>,
    pub id: sys::VAConfigID,
}

impl Drop for Config {
    fn drop(&mut self) {
        // SAFETY: `id` was created on this display and is destroyed once.
        unsafe { self.display.lib.vaDestroyConfig(self.display.dpy, self.id) };
    }
}

impl Config {
    pub fn new(display: &Arc<Display>, profile: sys::VAProfile, entrypoint: sys::VAEntrypoint, rt_format: u32) -> Result<Config, String> {
        let mut attrib = sys::VAConfigAttrib { type_: sys::VAConfigAttribType_VAConfigAttribRTFormat, value: rt_format };
        let mut id = 0;
        // SAFETY: one valid attribute and a valid out-pointer.
        display.check(unsafe { display.lib.vaCreateConfig(display.dpy, profile, entrypoint, &mut attrib, 1, &mut id) }, "vaCreateConfig")?;
        Ok(Config { display: display.clone(), id })
    }
}

/// A decoding context of a configuration and picture size. `Send` but not `Sync`: a picture's
/// begin / render / end sequence must not interleave with another thread's on the same context.
pub struct Context {
    display: Arc<Display>,
    _config: Arc<Config>,
    pub id: sys::VAContextID,
    pub size: (u32, u32),
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: `id` was created on this display and is destroyed once; every `Buffer` of the
        // context borrows it, so none is left to be destroyed after it.
        unsafe { self.display.lib.vaDestroyContext(self.display.dpy, self.id) };
    }
}

/// A VA parameter struct and the buffer type it fills: the driver reads exactly one whole struct
/// of the type from a buffer of that kind, so only these types can be passed
/// ([`Context::param_buffer`]); sealed.
pub trait ParamBuffer: Copy + sealed::Sealed {
    const KIND: sys::VABufferType;
}

mod sealed {
    pub trait Sealed {}
}

macro_rules! param_buffer {
    ($($t:ty => $kind:expr),* $(,)?) => {$(
        impl sealed::Sealed for $t {}
        impl ParamBuffer for $t {
            const KIND: sys::VABufferType = $kind;
        }
    )*};
}

param_buffer! {
    sys::VAPictureParameterBufferHEVC => sys::VABufferType_VAPictureParameterBufferType,
    sys::VAIQMatrixBufferHEVC => sys::VABufferType_VAIQMatrixBufferType,
    sys::VASliceParameterBufferHEVC => sys::VABufferType_VASliceParameterBufferType,
}

impl Context {
    /// A context without fixed render targets (surfaces from any pool of the same format).
    pub fn new(config: &Arc<Config>, width: u32, height: u32) -> Result<Context, String> {
        let d = &config.display;
        let mut id = 0;
        let (w, h) = (i32::try_from(width).map_err(|_| "width")?, i32::try_from(height).map_err(|_| "height")?);
        // SAFETY: a valid config; no render-target list (null, 0); valid out-pointer.
        d.check(unsafe { d.lib.vaCreateContext(d.dpy, config.id, w, h, sys::VA_PROGRESSIVE as i32, std::ptr::null_mut(), 0, &mut id) }, "vaCreateContext")?;
        Ok(Context { display: d.clone(), _config: config.clone(), id, size: (width, height), _not_sync: std::marker::PhantomData })
    }

    /// A parameter buffer holding a copy of `v`, of the kind its type fills.
    pub fn param_buffer<T: ParamBuffer>(&self, v: &T) -> Result<Buffer<'_>, String> {
        // SAFETY: `v` is a live `T`, readable for `size_of::<T>()` bytes; libva copies them with
        // memcpy (padding included, which C may copy) and does not keep the pointer.
        unsafe { self.create_buffer(T::KIND, (v as *const T).cast(), std::mem::size_of::<T>()) }
    }

    /// A slice-data buffer holding a copy of `data` (the slice NAL unit). The slice parameters
    /// that go with it must lie within it (checked by the caller against `data.len()`).
    pub fn slice_data(&self, data: &[u8]) -> Result<Buffer<'_>, String> {
        // SAFETY: `data` is readable for `data.len()` bytes; libva copies them.
        unsafe { self.create_buffer(sys::VABufferType_VASliceDataBufferType, data.as_ptr().cast(), data.len()) }
    }

    /// # Safety
    /// `data` must be readable for `len` bytes, and hold what the driver expects of `kind`.
    unsafe fn create_buffer(&self, kind: sys::VABufferType, data: *const std::ffi::c_void, len: usize) -> Result<Buffer<'_>, String> {
        let d = &self.display;
        let size = u32::try_from(len).map_err(|_| "buffer too large")?;
        let mut id = 0;
        // SAFETY: (the caller's contract) libva copies `size` bytes from `data` into a new buffer;
        // it does not write through the pointer despite the `*mut` in its signature.
        d.check(unsafe { d.lib.vaCreateBuffer(d.dpy, self.id, kind, size, 1, data.cast_mut(), &mut id) }, "vaCreateBuffer")?;
        Ok(Buffer { context: self, id })
    }

    /// Decode one picture into `target`: begin, render every buffer, end.
    pub fn decode(&self, target: &Surface, buffers: &[Buffer<'_>]) -> Result<(), String> {
        let d = &self.display;
        if buffers.iter().any(|b| !std::ptr::eq(b.context, self)) {
            return Err("a buffer of another context".into());
        }
        // the driver writes a whole context-sized picture into the target: a surface of another
        // display, or a smaller one, would be written out of bounds
        let (tw, th) = target.pool.size;
        if !Arc::ptr_eq(&target.pool.display, d) || tw < self.size.0 || th < self.size.1 {
            return Err(format!("a {tw}x{th} target surface for a {}x{} context", self.size.0, self.size.1));
        }
        // SAFETY: a valid context and surface of this display.
        d.check(unsafe { d.lib.vaBeginPicture(d.dpy, self.id, target.id) }, "vaBeginPicture")?;
        let mut ids: Vec<sys::VABufferID> = buffers.iter().map(|b| b.id).collect();
        let n = i32::try_from(ids.len()).map_err(|_| "too many buffers")?;
        // SAFETY: `ids` holds `n` buffers created on this context; libva reads them during the call.
        let r = d.check(unsafe { d.lib.vaRenderPicture(d.dpy, self.id, ids.as_mut_ptr(), n) }, "vaRenderPicture");
        // SAFETY: ends the picture begun above (also after a render error, so the context stays usable).
        let e = d.check(unsafe { d.lib.vaEndPicture(d.dpy, self.id) }, "vaEndPicture");
        r.and(e)
    }
}

/// A libva buffer of a context, destroyed on drop (before the context: it borrows it, so a
/// destroyed buffer id can never hit a later buffer that reused it).
pub struct Buffer<'a> {
    context: &'a Context,
    id: sys::VABufferID,
}

impl Drop for Buffer<'_> {
    fn drop(&mut self) {
        let d = &self.context.display;
        // SAFETY: `id` was created on this display and is destroyed once (libva does not destroy
        // decode buffers in vaRenderPicture / vaEndPicture).
        unsafe { d.lib.vaDestroyBuffer(d.dpy, self.id) };
    }
}

/// Spare surfaces a pool keeps for the next pictures; surfaces returned beyond this are destroyed
/// (their memory goes back to the driver).
const SPARE_SURFACES: usize = 8;

/// Surfaces of one format and size, recycled: a [`Surface`] returns to the pool when its last
/// holder drops it; the pool keeps a few spare and destroys the rest, and destroys all of them
/// when the last surface and the pool are gone.
pub struct SurfacePool {
    display: Arc<Display>,
    pub rt_format: u32,
    pub fourcc: u32,
    pub size: (u32, u32),
    free: Mutex<Vec<(sys::VASurfaceID, Arc<SurfaceMem>)>>,
    count: std::sync::atomic::AtomicUsize,
}

/// One surface's memory: a process-unique id (libva reuses surface ids after a surface is
/// destroyed, so ids alone cannot key caches), a token that dies with the memory, and its
/// DMA-BUF export, made once.
pub struct SurfaceMem {
    pub memory_id: u64,
    alive: Arc<()>,
    export: Mutex<Option<Arc<Export>>>,
}

impl SurfaceMem {
    /// Dead once the surface is destroyed (import caches drop their imports of it).
    pub fn liveness(&self) -> std::sync::Weak<()> {
        Arc::downgrade(&self.alive)
    }
}

/// A surface's memory exported as DMA-BUF (the description owns the fds).
pub struct Export {
    pub dmabuf: filmcraft_frame::DmaBuf,
}

/// Whether the rectangle (x, y, w, h) is non-empty and lies inside a surface of `size`.
fn rect_fits((x, y, w, h): (u32, u32, u32, u32), size: (u32, u32)) -> bool {
    w > 0 && h > 0 && x.checked_add(w).is_some_and(|r| r <= size.0) && y.checked_add(h).is_some_and(|b| b <= size.1)
}

impl Drop for SurfacePool {
    fn drop(&mut self) {
        let free = std::mem::take(&mut *self.free.lock().unwrap_or_else(PoisonError::into_inner));
        for (id, _) in free {
            self.destroy(id);
        }
    }
}

/// Live surfaces in all pools (`perf.stats`).
static LIVE_SURFACES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn live_surfaces() -> usize {
    LIVE_SURFACES.load(std::sync::atomic::Ordering::Relaxed)
}

/// Pools kept for reuse across decoders (most recently used last): a decoder restarted for a clip
/// (or the next clip of the same format) takes surfaces from the spares instead of allocating.
const SHARED_POOLS: usize = 4;

impl SurfacePool {
    /// The process's pool for this format and size on `display` (made on first use; the
    /// [`SHARED_POOLS`] most recently used ones are kept alive with their spare surfaces).
    pub fn shared(display: &Arc<Display>, rt_format: u32, fourcc: u32, width: u32, height: u32) -> Arc<SurfacePool> {
        static POOLS: Mutex<Vec<Arc<SurfacePool>>> = Mutex::new(Vec::new());
        let mut g = POOLS.lock().unwrap_or_else(PoisonError::into_inner);
        let found = g.iter().position(|p| Arc::ptr_eq(&p.display, display) && p.rt_format == rt_format && p.fourcc == fourcc && p.size == (width, height));
        let pool = match found {
            Some(i) => g.remove(i),
            None => SurfacePool::new(display, rt_format, fourcc, width, height),
        };
        g.push(pool.clone());
        if g.len() > SHARED_POOLS {
            g.remove(0);
        }
        pool
    }

    pub fn new(display: &Arc<Display>, rt_format: u32, fourcc: u32, width: u32, height: u32) -> Arc<SurfacePool> {
        Arc::new(SurfacePool { display: display.clone(), rt_format, fourcc, size: (width, height), free: Mutex::new(Vec::new()), count: Default::default() })
    }

    /// The spare surface returned longest ago, or a new one. Oldest first: a surface's last frame
    /// may still be sampled by GPU work already submitted when it comes back, and the decoder
    /// overwrites it on reuse; the longer it rests, the more surely that work has finished.
    pub fn get(self: &Arc<Self>) -> Result<Surface, String> {
        {
            let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
            if !free.is_empty() {
                let (id, mem) = free.remove(0);
                return Ok(Surface { id, mem, pool: self.clone() });
            }
        }
        static MEMORY_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let d = &self.display;
        let mut id = sys::VA_INVALID_SURFACE;
        let mut attr = sys::VASurfaceAttrib {
            type_: sys::VASurfaceAttribType_VASurfaceAttribPixelFormat,
            flags: sys::VA_SURFACE_ATTRIB_SETTABLE,
            value: sys::VAGenericValue {
                type_: sys::VAGenericValueType_VAGenericValueTypeInteger,
                value: sys::_VAGenericValue__bindgen_ty_1 { i: self.fourcc as i32 },
            },
        };
        // SAFETY: one surface into a valid out-pointer, one valid attribute.
        d.check(unsafe { d.lib.vaCreateSurfaces(d.dpy, self.rt_format, self.size.0, self.size.1, &mut id, 1, &mut attr, 1) }, "vaCreateSurfaces")?;
        self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        LIVE_SURFACES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mem =
            Arc::new(SurfaceMem { memory_id: MEMORY_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed), alive: Arc::new(()), export: Mutex::new(None) });
        Ok(Surface { id, mem, pool: self.clone() })
    }

    /// Surfaces alive (in use or spare).
    pub fn count(&self) -> usize {
        self.count.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn destroy(&self, mut id: sys::VASurfaceID) {
        // SAFETY: a surface of this display that no `Surface` refers to any more; destroyed once.
        unsafe { self.display.lib.vaDestroySurfaces(self.display.dpy, &mut id, 1) };
        self.count.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        LIVE_SURFACES.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One surface of a pool; back to the pool on drop.
pub struct Surface {
    pub id: sys::VASurfaceID,
    mem: Arc<SurfaceMem>,
    pool: Arc<SurfacePool>,
}

impl Drop for Surface {
    fn drop(&mut self) {
        let mut free = self.pool.free.lock().unwrap_or_else(PoisonError::into_inner);
        if free.len() < SPARE_SURFACES {
            free.push((self.id, self.mem.clone()));
        } else {
            drop(free);
            // the memory (its liveness token and export fds) goes with the surface
            self.pool.destroy(self.id);
        }
    }
}

impl Surface {
    pub fn pool(&self) -> &Arc<SurfacePool> {
        &self.pool
    }

    /// The surface's memory: unique id, liveness.
    pub fn mem(&self) -> &Arc<SurfaceMem> {
        &self.mem
    }

    /// The surface's memory as DMA-BUF layers (luma, chroma), exported once per surface.
    pub fn export(&self) -> Result<Arc<Export>, String> {
        if let Some(e) = self.mem.export.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            return Ok(e.clone());
        }
        let d = &self.pool.display;
        let mut desc = sys::VADRMPRIMESurfaceDescriptor::default();
        let (mem, flags) = (sys::VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2, sys::VA_EXPORT_SURFACE_READ_ONLY | sys::VA_EXPORT_SURFACE_SEPARATE_LAYERS);
        let out = &mut desc as *mut sys::VADRMPRIMESurfaceDescriptor as *mut std::ffi::c_void;
        // SAFETY: a valid surface; `out` points to the VADRMPRIMESurfaceDescriptor that
        // VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2 fills.
        let st = unsafe { d.lib.vaExportSurfaceHandle(d.dpy, self.id, mem, flags, out) };
        d.check(st, "vaExportSurfaceHandle")?;
        // take ownership of every object fd right away, so none leaks on an error below
        let nobj = (desc.num_objects as usize).min(desc.objects.len());
        let fds: Vec<std::os::fd::OwnedFd> = desc.objects[..nobj]
            .iter()
            .filter(|o| o.fd >= 0)
            // SAFETY: vaExportSurfaceHandle returns new fds owned by the caller.
            .map(|o| unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(o.fd) })
            .collect();
        if fds.len() != nobj || nobj == 0 {
            return Err("the export has no buffer objects".into());
        }
        if desc.num_layers != 2 {
            return Err(format!("{} layers (expected luma and chroma)", desc.num_layers));
        }
        let modifier = desc.objects[0].drm_format_modifier;
        if desc.objects[..nobj].iter().any(|o| o.drm_format_modifier != modifier) {
            return Err("objects with different format modifiers".into());
        }
        let layer = |l: &sys::_VADRMPRIMESurfaceDescriptor__bindgen_ty_2| -> Result<Vec<filmcraft_frame::DmaBufPlane>, String> {
            let n = (l.num_planes as usize).min(4);
            (0..n)
                .map(|p| {
                    let obj = *l.object_index.get(p).ok_or("plane index")? as usize;
                    if obj >= nobj {
                        return Err("object index out of range".into());
                    }
                    Ok(filmcraft_frame::DmaBufPlane { object: obj, offset: l.offset[p], pitch: l.pitch[p] })
                })
                .collect()
        };
        let layers = [layer(&desc.layers[0])?, layer(&desc.layers[1])?];
        let dmabuf = filmcraft_frame::DmaBuf {
            layer_formats: [desc.layers[0].drm_format, desc.layers[1].drm_format],
            modifier,
            layers,
            width: desc.width,
            height: desc.height,
            // the description owns the fds, so they stay open while any copy lives
            objects: fds.into(),
        };
        let e = Arc::new(Export { dmabuf });
        *self.mem.export.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.clone());
        Ok(e)
    }

    /// Wait until the decode into this surface has finished.
    pub fn sync(&self) -> Result<(), String> {
        let d = &self.pool.display;
        // SAFETY: a valid surface of this display.
        d.check(unsafe { d.lib.vaSyncSurface(d.dpy, self.id) }, "vaSyncSurface")
    }

    /// Copy the rectangle (x, y, w, h) of this surface to memory, as the image format `fourcc`
    /// (`P010` / `NV12`): `f(plane index, row bytes)` for every row of the luma plane then the
    /// chroma plane. Syncs first.
    pub fn read(&self, rect: (u32, u32, u32, u32), mut f: impl FnMut(usize, usize, &[u8])) -> Result<(), String> {
        let (x, y, w, h) = rect;
        if !rect_fits(rect, self.pool.size) {
            return Err(format!("rectangle {w}x{h} at ({x}, {y}) outside the {}x{} surface", self.pool.size.0, self.pool.size.1));
        }
        self.sync()?;
        let d = &self.pool.display;
        let l = d.lib;
        let fourcc = self.pool.fourcc;
        // the driver's description of the format
        // SAFETY: initialised display.
        let max = unsafe { l.vaMaxNumImageFormats(d.dpy) }.clamp(0, 256) as usize;
        let mut formats: Vec<sys::VAImageFormat> = vec![Default::default(); max];
        let mut n = 0;
        // SAFETY: the array holds vaMaxNumImageFormats entries.
        d.check(unsafe { l.vaQueryImageFormats(d.dpy, formats.as_mut_ptr(), &mut n) }, "vaQueryImageFormats")?;
        formats.truncate((n.max(0) as usize).min(max));
        let mut format = *formats.iter().find(|f| f.fourcc == fourcc).ok_or("the driver has no image format for the surface")?;
        let int = |v: u32| i32::try_from(v).map_err(|_| format!("{v} does not fit libva's int"));
        let (xi, yi, wi, hi) = (int(x)?, int(y)?, int(w)?, int(h)?);
        let mut image: sys::VAImage = Default::default();
        // SAFETY: a valid format struct and out-pointer.
        d.check(unsafe { l.vaCreateImage(d.dpy, &mut format, wi, hi, &mut image) }, "vaCreateImage")?;
        // a driver could hand back an image smaller than asked: the rows below are bounds-checked
        let result = (|| {
            // SAFETY: the image was created above with the requested size; the rectangle lies in the
            // surface (`rect_fits`).
            d.check(unsafe { l.vaGetImage(d.dpy, self.id, xi, yi, w, h, image.image_id) }, "vaGetImage")?;
            let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            // SAFETY: the image's buffer, mapped for reading into a valid out-pointer.
            d.check(unsafe { l.vaMapBuffer(d.dpy, image.buf, &mut ptr) }, "vaMapBuffer")?;
            if ptr.is_null() {
                return Err("vaMapBuffer returned no memory".to_string());
            }
            // SAFETY: the mapping covers `data_size` bytes until vaUnmapBuffer below.
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, image.data_size as usize) };
            let bpp = if fourcc == sys::VA_FOURCC_P010 { 2 } else { 1 };
            let rows = [(h as usize, w as usize * bpp), (h.div_ceil(2) as usize, w.div_ceil(2) as usize * 2 * bpp)];
            let mut ok = true;
            for (plane, &(nrows, row_bytes)) in rows.iter().enumerate() {
                let (off, pitch) = (image.offsets[plane] as usize, image.pitches[plane] as usize);
                for r in 0..nrows {
                    let start = r.checked_mul(pitch).and_then(|o| o.checked_add(off));
                    match start.and_then(|s| bytes.get(s..s.checked_add(row_bytes)?)) {
                        Some(row) => f(plane, r, row),
                        None => ok = false,
                    }
                }
            }
            // SAFETY: unmaps the mapping made above; `bytes` is not used after this.
            unsafe { l.vaUnmapBuffer(d.dpy, image.buf) };
            if ok { Ok(()) } else { Err("image smaller than its planes".to_string()) }
        })();
        // SAFETY: destroys the image created above, once.
        unsafe { l.vaDestroyImage(d.dpy, image.image_id) };
        result
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, Context, Display, SurfacePool, rect_fits, sys};
    use std::sync::Arc;

    /// Unsafe-code review round 3: a context decodes only into surfaces of its display that hold a
    /// whole picture (the driver would write past a smaller one). Skipped without VA-API.
    #[test]
    fn a_context_refuses_a_smaller_target_surface() {
        let Ok(d) = Display::get() else {
            eprintln!("no VA-API; skipped");
            return;
        };
        let (profile, vld) = (sys::VAProfile_VAProfileHEVCMain, sys::VAEntrypoint_VAEntrypointVLD);
        if !d.supports(profile, vld) {
            eprintln!("no HEVC decode; skipped");
            return;
        }
        let config = Arc::new(Config::new(&d, profile, vld, sys::VA_RT_FORMAT_YUV420).expect("config"));
        let ctx = Context::new(&config, 256, 256).expect("context");
        let small = SurfacePool::new(&d, sys::VA_RT_FORMAT_YUV420, sys::VA_FOURCC_NV12, 128, 256).get().expect("surface");
        let err = ctx.decode(&small, &[]).expect_err("a smaller target is refused");
        assert!(err.contains("128x256"), "{err}");
        let fits = SurfacePool::new(&d, sys::VA_RT_FORMAT_YUV420, sys::VA_FOURCC_NV12, 256, 256).get().expect("surface");
        // passes the check (the driver may still refuse an empty picture)
        if let Err(e) = ctx.decode(&fits, &[]) {
            assert!(!e.contains("target surface"), "{e}");
        }
    }

    #[test]
    fn read_rectangles_must_lie_in_the_surface() {
        assert!(rect_fits((0, 0, 64, 32), (64, 32)));
        assert!(rect_fits((8, 4, 56, 28), (64, 32)));
        assert!(!rect_fits((0, 0, 0, 32), (64, 32)));
        assert!(!rect_fits((1, 0, 64, 32), (64, 32)));
        assert!(!rect_fits((0, 0, 64, 33), (64, 32)));
        assert!(!rect_fits((u32::MAX, 0, 2, 1), (64, 32)));
        assert!(!rect_fits((0, 0, u32::MAX, 1), (64, 32)));
    }
}
