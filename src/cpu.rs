//! Runtime CPU feature detection and last-level cache sizing.

#[cfg(target_arch = "x86_64")]
#[inline]
pub(crate) fn avx2_available() -> bool {
    #[cfg(test)]
    if FORCE_NO_AVX2.with(|forced| forced.get()) {
        return false;
    }
    static DETECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| std::is_x86_feature_detected!("avx2"))
}

#[cfg(target_arch = "x86_64")]
#[inline]
pub(crate) fn bmi2_available() -> bool {
    #[cfg(test)]
    if FORCE_NO_BMI2.with(|forced| forced.get()) {
        return false;
    }
    static DETECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| std::is_x86_feature_detected!("bmi2"))
}

#[cfg(all(test, target_arch = "x86_64"))]
thread_local! {
    /// Test-only switch used to exercise scalar fallbacks on a CPU that has
    /// AVX2. Thread-local so it cannot affect parallel tests.
    pub(crate) static FORCE_NO_AVX2: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test-only switch used to exercise the no-BMI2 fallback on a CPU that
    /// has BMI2. Thread-local so it cannot affect parallel tests.
    pub(crate) static FORCE_NO_BMI2: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Smallest and largest cache sizes believed from CPUID. A value outside this
/// range is treated as unknown; an unknown LLC disables streaming stores.
#[cfg(target_arch = "x86_64")]
const MIN_CACHE_BYTES: usize = 64 * 1024;
#[cfg(target_arch = "x86_64")]
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Parse one deterministic-cache-parameters entry. Intel leaf 4 and AMD leaf
/// 0x8000_001d share this layout.
#[cfg(target_arch = "x86_64")]
fn parse_cache_leaf(regs: std::arch::x86_64::CpuidResult) -> Option<(u8, usize)> {
    let cache_type = regs.eax & 0x1f;
    if !matches!(cache_type, 1 | 3) {
        return None;
    }
    let level = ((regs.eax >> 5) & 0x7) as u8;
    if level < 2 {
        return None;
    }
    let line = (regs.ebx & 0xfff) as usize + 1;
    let partitions = ((regs.ebx >> 12) & 0x3ff) as usize + 1;
    let ways = ((regs.ebx >> 22) & 0x3ff) as usize + 1;
    let sets = regs.ecx as usize + 1;
    let size = line
        .checked_mul(partitions)?
        .checked_mul(ways)?
        .checked_mul(sets)?;
    if !(MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&size) {
        return None;
    }
    Some((level, size))
}

/// Largest data/unified cache reported by a deterministic-cache leaf.
#[cfg(target_arch = "x86_64")]
#[allow(unused_unsafe)]
fn cache_leaf_size(leaf: u32) -> Option<usize> {
    let mut best: Option<(u8, usize)> = None;
    for index in 0..16 {
        // Leaf/index are documented architectural inputs and unknown leaves
        // return zeros.
        let regs = unsafe { std::arch::x86_64::__cpuid_count(leaf, index) };
        if regs.eax & 0x1f == 0 {
            break;
        }
        if let Some(entry) = parse_cache_leaf(regs)
            && best.is_none_or(|(level, _)| entry.0 > level)
        {
            best = Some(entry);
        }
    }
    best.map(|(_, size)| size)
}

/// Last-level cache size in bytes, if CPUID reports a plausible value.
#[cfg(target_arch = "x86_64")]
#[allow(unused_unsafe)]
pub(crate) fn llc_bytes() -> Option<usize> {
    let max_basic = unsafe { std::arch::x86_64::__cpuid(0) }.eax;
    if max_basic >= 4
        && let Some(size) = cache_leaf_size(4)
    {
        return Some(size);
    }
    let max_extended = unsafe { std::arch::x86_64::__cpuid(0x8000_0000) }.eax;
    if max_extended >= 0x8000_001d
        && let Some(size) = cache_leaf_size(0x8000_001d)
    {
        return Some(size);
    }
    if max_extended >= 0x8000_0006 {
        let regs = unsafe { std::arch::x86_64::__cpuid(0x8000_0006) };
        let l3 = ((regs.ecx >> 18) & 0x3fff) as usize * 512 * 1024;
        if (MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&l3) {
            return Some(l3);
        }
        // Some older AMD parts report only L2 here: bits 31:16 in 256 KiB
        // units. L2 is then the last-level cache for the streaming decision.
        let l2 = ((regs.ecx >> 16) & 0xffff) as usize * 256 * 1024;
        if (MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&l2) {
            return Some(l2);
        }
    }
    None
}

/// Input size past which the input plus encoded output no longer fits in the
/// last-level cache. The encoded output is 4/3 of the input, so the working set
/// is 7/3 of the input; `N >= 3/7 * LLC` is the crossover. In that regime
/// cached stores pay for a read-for-ownership of the output, while streaming
/// stores write it once.
#[cfg(target_arch = "x86_64")]
fn stream_floor_for(llc: Option<usize>) -> usize {
    match llc {
        Some(bytes) if (MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&bytes) => {
            (bytes / 7 * 3).max(MIN_CACHE_BYTES)
        }
        _ => usize::MAX,
    }
}

/// Cached stream floor for the current processor.
#[cfg(target_arch = "x86_64")]
#[inline]
pub(crate) fn stream_floor() -> usize {
    static FLOOR: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *FLOOR.get_or_init(|| stream_floor_for(llc_bytes()))
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub(crate) fn neon_available() -> bool {
    static DETECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| std::arch::is_aarch64_feature_detected!("neon"))
}

#[cfg(all(test, target_arch = "x86_64"))]
mod cache_tests {
    use super::{parse_cache_leaf, stream_floor_for};
    use std::arch::x86_64::CpuidResult;

    fn entry(level: u8, cache_type: u32, line: u32, ways: u32, sets: u32) -> CpuidResult {
        CpuidResult {
            eax: cache_type | (level as u32) << 5,
            ebx: (line - 1) | (ways - 1) << 22,
            ecx: sets - 1,
            edx: 0,
        }
    }

    #[test]
    fn parses_intel_and_amd_l3_shapes() {
        let intel = entry(3, 3, 64, 16, 16 * 1024);
        assert_eq!(parse_cache_leaf(intel), Some((3, 16 * 1024 * 1024)));
        let amd = entry(3, 3, 64, 16, 32 * 1024);
        assert_eq!(parse_cache_leaf(amd), Some((3, 32 * 1024 * 1024)));
        let l2 = entry(2, 3, 64, 8, 1024);
        assert_eq!(parse_cache_leaf(l2), Some((2, 512 * 1024)));
    }

    #[test]
    fn rejects_instruction_and_null_entries() {
        assert_eq!(parse_cache_leaf(entry(3, 0, 64, 16, 1024)), None);
        assert_eq!(parse_cache_leaf(entry(3, 2, 64, 16, 1024)), None);
    }

    #[test]
    fn rejects_implausible_sizes() {
        assert_eq!(parse_cache_leaf(entry(3, 3, 64, 16, 1)), None);
        assert_eq!(parse_cache_leaf(entry(3, 3, 64, 1024, u32::MAX)), None);
    }

    #[test]
    fn detected_cache_is_sane() {
        let llc = super::llc_bytes();
        let floor = super::stream_floor();
        println!("llc_bytes={llc:?} stream_floor={floor}");
        assert!(llc.is_none_or(|bytes| {
            (super::MIN_CACHE_BYTES..=super::MAX_CACHE_BYTES).contains(&bytes)
        }));
        assert!(floor == usize::MAX || floor >= super::MIN_CACHE_BYTES);
    }

    #[test]
    fn floor_is_three_sevenths_of_llc() {
        assert_eq!(stream_floor_for(Some(16 * 1024 * 1024)), 7_190_235);
        assert_eq!(stream_floor_for(Some(512 * 1024)), 224_694);
        assert_eq!(stream_floor_for(Some(64 * 1024)), 65_536);
        assert_eq!(stream_floor_for(None), usize::MAX);
        assert_eq!(stream_floor_for(Some(0)), usize::MAX);
    }
}
