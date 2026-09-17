//! Process-wide memory policy: return unused pages to the OS after load drops.
//!
//! mimalloc is kept as the production allocator. Switching to jemalloc would
//! still inflate RSS on Linux hosts with `transparent_hugepage=always`, because
//! the kernel collapses anonymous mappings into 2MiB pages that idle purge
//! cannot split. The server therefore:
//! - disables transparent huge pages for its own address space
//! - prefers mimalloc's immediate decommit/purge defaults
//! - splits leftover huge pages and forces allocator collection when idle

use std::ops::Range;

const MIMALLOC_ENV_DEFAULTS: &[(&str, &str)] = &[
    ("MIMALLOC_ALLOW_THP", "0"),
    ("MIMALLOC_PURGE_DELAY", "0"),
    ("MIMALLOC_PURGE_DECOMMITS", "1"),
    ("MIMALLOC_EAGER_COMMIT", "0"),
    ("MIMALLOC_ARENA_EAGER_COMMIT", "0"),
    ("MIMALLOC_ABANDONED_PAGE_PURGE", "1"),
];

#[cfg_attr(
    not(any(test, target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
const TRANSPARENT_HUGE_PAGE_BYTES: u64 = 2 * 1024 * 1024;

/// Install reclaim-friendly allocator defaults and disable process THP.
///
/// Safe to call more than once. Existing environment values are left in place
/// so Docker/operators can still override mimalloc knobs.
pub fn apply_process_memory_policy() {
    install_mimalloc_env_defaults();
    disable_transparent_huge_pages();
}

/// Return unused heap pages to the OS after a load burst.
pub fn reclaim_idle_process_memory() {
    disable_transparent_huge_pages();
    split_anonymous_transparent_huge_pages();
    crate::force_allocator_collection(true);
    trim_libc_heap();
}

fn install_mimalloc_env_defaults() {
    for &(key, value) in MIMALLOC_ENV_DEFAULTS {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn disable_transparent_huge_pages() {
    // SAFETY: PR_SET_THP_DISABLE with a boolean `1` argument is an mm-wide
    // request on modern kernels and does not dereference pointers. Failure is
    // ignored because older kernels and restricted seccomp profiles may reject it.
    unsafe {
        libc::prctl(libc::PR_SET_THP_DISABLE, 1, 0, 0, 0);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn disable_transparent_huge_pages() {}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn split_anonymous_transparent_huge_pages() {
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
        return;
    };
    for range in anonymous_private_ranges(maps.lines()) {
        let len = range.end.saturating_sub(range.start);
        if len < TRANSPARENT_HUGE_PAGE_BYTES {
            continue;
        }
        let Ok(len) = usize::try_from(len) else {
            continue;
        };
        // SAFETY: `range` comes from this process's maps, is page-aligned, and
        // MADV_NOHUGEPAGE only changes VM flags / splits huge pages. EINVAL is
        // expected if another thread unmapped the range concurrently.
        unsafe {
            libc::madvise(range.start as *mut libc::c_void, len, libc::MADV_NOHUGEPAGE);
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn split_anonymous_transparent_huge_pages() {}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn trim_libc_heap() {
    // SAFETY: malloc_trim(0) is a glibc heap compact; it does not take pointers.
    unsafe {
        libc::malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn trim_libc_heap() {}

#[cfg_attr(
    not(any(test, target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
fn anonymous_private_ranges<'a, I>(lines: I) -> Vec<Range<u64>>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut ranges = Vec::new();
    for line in lines {
        if let Some(range) = parse_anonymous_private_range(line) {
            ranges.push(range);
        }
    }
    ranges
}

#[cfg_attr(
    not(any(test, target_os = "linux", target_os = "android")),
    allow(dead_code)
)]
fn parse_anonymous_private_range(line: &str) -> Option<Range<u64>> {
    let mut parts = line.split_whitespace();
    let addrs = parts.next()?;
    let perms = parts.next()?;
    let _offset = parts.next()?;
    let _dev = parts.next()?;
    let inode = parts.next()?;
    let pathname = parts.next().unwrap_or("");

    if !perms.contains('p') || !perms.contains('w') {
        return None;
    }
    if inode != "0" {
        return None;
    }
    if matches!(pathname, "[stack]" | "[vdso]" | "[vvar]" | "[vsyscall]") {
        return None;
    }

    let (start, end) = addrs.split_once('-')?;
    let start = u64::from_str_radix(start, 16).ok()?;
    let end = u64::from_str_radix(end, 16).ok()?;
    (end > start).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_memory_policy_is_idempotent() {
        apply_process_memory_policy();
        apply_process_memory_policy();
        reclaim_idle_process_memory();
    }

    #[test]
    fn parses_anonymous_private_heap_and_arenas() {
        let maps = "\
70f400000000-70f402000000 rw-p 00000000 00:00 0\n\
55a1b2c00000-55a1b2e21000 rw-p 00000000 00:00 0                          [heap]\n\
7ffc1234a000-7ffc1236b000 rw-p 00000000 00:00 0                          [stack]\n\
7f8b80000000-7f8b80021000 r--p 00000000 08:01 12345                      /lib/libc.so.6\n\
7f8b81000000-7f8b81200000 rw-s 00000000 00:00 0                          /dev/zero\n";
        let ranges = anonymous_private_ranges(maps.lines());
        assert_eq!(
            ranges,
            vec![
                0x70f4_0000_0000..0x70f4_0200_0000,
                0x55a1_b2c0_0000..0x55a1_b2e2_1000
            ]
        );
        assert!(ranges[0].end - ranges[0].start >= TRANSPARENT_HUGE_PAGE_BYTES);
    }

    #[test]
    fn skips_file_backed_and_non_writable_maps() {
        assert!(parse_anonymous_private_range(
            "7f8b80000000-7f8b80021000 r-xp 00000000 08:01 12345 /lib/ld-linux.so"
        )
        .is_none());
        assert!(parse_anonymous_private_range(
            "7f8c00000000-7f8c02000000 rw-p 00000000 08:01 99 /data/000123.sst"
        )
        .is_none());
    }
}
