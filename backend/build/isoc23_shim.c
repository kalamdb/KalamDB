#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

// Shim for toolchains that compile against glibc 2.38+ headers and link an
// older libc that has no __isoc23_* symbols. RocksDB's calls become
// references to those symbols.
//
// The 2.38 headers rewrite strtol, strtoll, strtoull, and vsscanf to the
// __isoc23_* names. Calling those functions from this file recurses until
// the thread hits its stack guard. Bind the pre-2.38 versions instead.

#if defined(__x86_64__) || defined(__i386__)
#define KALAM_GLIBC_VER "GLIBC_2.2.5"
#elif defined(__aarch64__)
#define KALAM_GLIBC_VER "GLIBC_2.17"
#elif defined(__riscv) && __riscv_xlen == 64
#define KALAM_GLIBC_VER "GLIBC_2.27"
#else
#define KALAM_GLIBC_VER "GLIBC_2.2.5"
#endif

#define KALAM_SYMVER(local, symbol, version) \
    __asm__(".symver " #local "," #symbol "@" version)

extern long kalam_strtol(const char *nptr, char **endptr, int base);
extern long long kalam_strtoll(const char *nptr, char **endptr, int base);
extern unsigned long long kalam_strtoull(const char *nptr, char **endptr, int base);
extern int kalam_vsscanf(const char *str, const char *format, va_list ap);

KALAM_SYMVER(kalam_strtol, strtol, KALAM_GLIBC_VER);
KALAM_SYMVER(kalam_strtoll, strtoll, KALAM_GLIBC_VER);
KALAM_SYMVER(kalam_strtoull, strtoull, KALAM_GLIBC_VER);
KALAM_SYMVER(kalam_vsscanf, vsscanf, KALAM_GLIBC_VER);

long __isoc23_strtol(const char *nptr, char **endptr, int base) {
    return kalam_strtol(nptr, endptr, base);
}

long long __isoc23_strtoll(const char *nptr, char **endptr, int base) {
    return kalam_strtoll(nptr, endptr, base);
}

unsigned long long __isoc23_strtoull(const char *nptr, char **endptr, int base) {
    return kalam_strtoull(nptr, endptr, base);
}

int __isoc23_sscanf(const char *str, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int result = kalam_vsscanf(str, format, args);
    va_end(args);
    return result;
}
