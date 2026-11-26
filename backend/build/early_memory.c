#if defined(__linux__) || defined(__ANDROID__) || defined(__APPLE__)
#if defined(__linux__) || defined(__ANDROID__)
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include <sys/prctl.h>
#ifndef PR_SET_THP_DISABLE
#define PR_SET_THP_DISABLE 41
#endif
#endif

#include <stdlib.h>

/* Runs before mimalloc's process constructor (clang uses priority 101; gcc
 * default is 65535). On Linux, hosts with transparent_hugepage=always collapse
 * mimalloc arenas into 2MiB pages, so idle purge cannot return RSS to the OS.
 * Disable THP on Linux and install reclaim-friendly mimalloc defaults before
 * mi_process_init() on both Linux and macOS unless the operator already set them. */
__attribute__((constructor(100))) static void kalamdb_early_memory_policy(void) {
    (void)setenv("MIMALLOC_ALLOW_THP", "0", 0);
    (void)setenv("MIMALLOC_PURGE_DELAY", "0", 0);
    (void)setenv("MIMALLOC_PURGE_DECOMMITS", "1", 0);
    (void)setenv("MIMALLOC_EAGER_COMMIT", "0", 0);
    (void)setenv("MIMALLOC_ARENA_EAGER_COMMIT", "0", 0);
    (void)setenv("MIMALLOC_ABANDONED_PAGE_PURGE", "1", 0);
#if defined(__linux__) || defined(__ANDROID__)
    (void)prctl(PR_SET_THP_DISABLE, 1L, 0L, 0L, 0L);
#endif
}
#endif
