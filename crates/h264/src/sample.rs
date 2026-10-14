//! Picture sample types (fork H2): `u8` for 8-bit pictures, `u16` for High 10. Sample kernels are
//! generic over [`Sample`] and take the largest code `max` = (1 << BitDepth) - 1, so each is
//! compiled once per type and the 8-bit path keeps its speed.

/// A picture sample.
pub trait Sample: Copy + Default + PartialEq + std::fmt::Debug + Send + Sync + 'static {
    /// The sample as an integer.
    fn get(self) -> i32;
    /// A sample from an integer in `0..=max` (callers clip first).
    fn from_i32(v: i32) -> Self;
    /// One output row of chroma bilinear interpolation (8.4.2.2.2): `o[c]` from `r0[c..c + 2]`
    /// and `r1[c..c + 2]` with weights `w` = (w00, w10, w01, w11) summing to 64. Per type so
    /// 8-bit keeps 16-bit arithmetic (twice the vector lanes); the result never exceeds the
    /// inputs' range, so no clipping.
    fn chroma_row(r0: &[Self], r1: &[Self], o: &mut [Self], w: [u32; 4]);
    /// The largest code of a picture whose (1 << BitDepth) - 1 is `max`. A constant for `u8`
    /// (8-bit pictures only), so the 8-bit kernels clip to 255 at compile time.
    fn code_max(max: i32) -> i32;
}

/// The largest 8-bit sample code.
pub const MAX_8BIT: i32 = 255;

impl Sample for u8 {
    #[inline(always)]
    fn get(self) -> i32 {
        i32::from(self)
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u8
    }
    #[inline(always)]
    fn code_max(_max: i32) -> i32 {
        MAX_8BIT
    }
    #[inline(always)]
    fn chroma_row(r0: &[u8], r1: &[u8], o: &mut [u8], w: [u32; 4]) {
        let [w00, w10, w01, w11] = w.map(|x| x as u16);
        for (c, o) in o.iter_mut().enumerate() {
            let v = w00 * r0[c] as u16 + w10 * r0[c + 1] as u16 + w01 * r1[c] as u16 + w11 * r1[c + 1] as u16;
            *o = ((v + 32) >> 6) as u8;
        }
    }
}

impl Sample for u16 {
    #[inline(always)]
    fn get(self) -> i32 {
        i32::from(self)
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u16
    }
    #[inline(always)]
    fn code_max(max: i32) -> i32 {
        max
    }
    #[inline(always)]
    fn chroma_row(r0: &[u16], r1: &[u16], o: &mut [u16], w: [u32; 4]) {
        let [w00, w10, w01, w11] = w;
        for (c, o) in o.iter_mut().enumerate() {
            let v = w00 * r0[c] as u32 + w10 * r0[c + 1] as u32 + w01 * r1[c] as u32 + w11 * r1[c + 1] as u32;
            *o = ((v + 32) >> 6) as u16;
        }
    }
}

/// Clip1 (5-4x): `v` clipped to `0..=max`.
#[inline(always)]
pub fn clip1<S: Sample>(v: i32, max: i32) -> S {
    S::from_i32(v.clamp(0, max))
}
