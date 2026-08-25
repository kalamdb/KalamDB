//! KalamDB server process shell.

use anyhow::Result;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(unix)]
fn desired_fd_soft_limit(current: libc::rlim_t, hard: libc::rlim_t) -> libc::rlim_t {
    current.max(65_536).min(hard)
}

#[cfg(unix)]
fn raise_fd_limit() {
    use std::mem::MaybeUninit;

    let mut limit = MaybeUninit::<libc::rlimit>::uninit();

    // SAFETY: getrlimit initializes `limit` on success, and the same initialized value is then
    // passed back to setrlimit/getrlimit using the RLIMIT_NOFILE resource.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) != 0 {
            return;
        }

        let mut limit = limit.assume_init();
        let old_soft = limit.rlim_cur;
        limit.rlim_cur = desired_fd_soft_limit(limit.rlim_cur, limit.rlim_max);
        let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
        let _ = libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit);
        if limit.rlim_cur != old_soft {
            eprintln!("📂 Raised open-file limit: {} → {}", old_soft, limit.rlim_cur);
        }
    }
}

fn main() -> Result<()> {
    #[cfg(unix)]
    raise_fd_limit();

    kalamdb_server::process::run_process(std::env::args())
}

#[cfg(test)]
mod tests {
    use std::alloc::Layout;
    #[cfg(feature = "mimalloc")]
    use std::{hint::black_box, time::Instant};

    #[cfg(feature = "mimalloc")]
    use kalamdb_observability::{collect_runtime_metrics, force_allocator_collection};

    #[cfg(unix)]
    #[test]
    fn fd_soft_limit_targets_65536_without_exceeding_hard_limit() {
        assert_eq!(super::desired_fd_soft_limit(1_024, 1_000_000), 65_536);
        assert_eq!(super::desired_fd_soft_limit(1_024, 4_096), 4_096);
        assert_eq!(super::desired_fd_soft_limit(100_000, 1_000_000), 100_000);
    }

    #[test]
    fn allocator_alloc_dealloc_roundtrip() {
        let layout = Layout::array::<u8>(4096).unwrap();

        // SAFETY: the allocation uses `layout`, is checked for null, accessed only within its
        // 4096-byte extent, and deallocated exactly once with the same layout.
        unsafe {
            let ptr = std::alloc::alloc(layout);
            assert!(!ptr.is_null(), "allocation must succeed");
            std::ptr::write_bytes(ptr, 0xAB, 4096);
            assert_eq!(*ptr, 0xAB);
            assert_eq!(*ptr.add(4095), 0xAB);
            std::alloc::dealloc(ptr, layout);
        }
    }

    #[test]
    fn allocator_small_alloc_stress() {
        const COUNT: usize = 10_000;
        const SIZE: usize = 64;
        let layout = Layout::from_size_align(SIZE, 8).unwrap();
        let mut pointers = Vec::with_capacity(COUNT);

        // SAFETY: every successful allocation uses `layout`, accesses stay within SIZE bytes,
        // and every pointer is deallocated exactly once with the matching layout.
        unsafe {
            for index in 0..COUNT {
                let pointer = std::alloc::alloc(layout);
                assert!(!pointer.is_null(), "allocation {index} must succeed");
                std::ptr::write_bytes(pointer, (index & 0xFF) as u8, SIZE);
                pointers.push(pointer);
            }

            for (index, pointer) in pointers.iter().enumerate().rev() {
                assert_eq!(**pointer, (index & 0xFF) as u8);
                std::alloc::dealloc(*pointer, layout);
            }
        }
    }

    #[cfg(feature = "mimalloc")]
    #[test]
    fn mimalloc_is_global_allocator() {
        let name = std::any::type_name_of_val(&super::ALLOC);
        assert!(name.contains("MiMalloc"), "expected MiMalloc global allocator, got: {name}");
    }

    #[cfg(feature = "mimalloc")]
    #[test]
    fn mimalloc_allocator_metrics_recover_after_transient_allocation() {
        let start = Instant::now();
        for _ in 0..16 {
            black_box(collect_runtime_metrics(start));
        }

        force_allocator_collection(true);
        let before = collect_runtime_metrics(start);
        let buffers = (0..64).map(|_| vec![0xAB; 1024 * 1024]).collect::<Vec<_>>();
        black_box(&buffers);

        let during = collect_runtime_metrics(start);
        let memory_delta = during
            .memory_bytes
            .unwrap_or_default()
            .saturating_sub(before.memory_bytes.unwrap_or_default());
        assert!(
            memory_delta >= 32 * 1024 * 1024,
            "expected >=32MB process memory growth, got {memory_delta} bytes"
        );

        drop(buffers);
        force_allocator_collection(true);
        let after = collect_runtime_metrics(start);
        assert!(
            after.memory_bytes.unwrap_or_default()
                <= before.memory_bytes.unwrap_or_default() + 24 * 1024 * 1024,
            "process memory did not recover near baseline"
        );
    }

    #[cfg(feature = "mimalloc")]
    #[test]
    fn mimalloc_runtime_metrics_collection_does_not_monotonically_grow_allocator_state() {
        let start = Instant::now();
        for _ in 0..32 {
            black_box(collect_runtime_metrics(start));
        }

        force_allocator_collection(true);
        let before = collect_runtime_metrics(start);
        for _ in 0..256 {
            black_box(collect_runtime_metrics(start));
        }

        force_allocator_collection(true);
        let after = collect_runtime_metrics(start);
        let allowed_growth = if std::env::var_os("LLVM_PROFILE_FILE").is_some() {
            32 * 1024 * 1024
        } else {
            8 * 1024 * 1024
        };
        assert!(
            after.memory_bytes.unwrap_or_default()
                <= before.memory_bytes.unwrap_or_default() + allowed_growth,
            "runtime metrics collection retained too much process memory"
        );
    }
}
