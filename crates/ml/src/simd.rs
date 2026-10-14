//! The vector instruction set this CPU offers, detected at run time.

/// The widest vector instruction set available at run time: `"avx512"`, `"avx2"`, `"sse4.1"`,
/// `"sse2"` (x86-64), `"neon"` (aarch64) or `"portable"`.
pub fn level() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx512f") {
            return "avx512";
        }
        if std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma") {
            return "avx2";
        }
        if std::arch::is_x86_feature_detected!("sse4.1") {
            return "sse4.1";
        }
        "sse2"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "neon"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "portable"
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_a_known_level() {
        assert!(["avx512", "avx2", "sse4.1", "sse2", "neon", "portable"].contains(&super::level()));
    }
}
