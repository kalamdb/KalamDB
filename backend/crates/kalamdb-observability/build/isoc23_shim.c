#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

// Shim for older glibc / sysroots that don't provide __isoc23_*.
// Some build environments (e.g., portable Linux builds) may compile against
// newer glibc headers but link against an older libc, resulting in undefined
// references from RocksDB.
//
// We forward to the traditional functions which exist on older glibc.

long __isoc23_strtol(const char *nptr, char **endptr, int base) {
    return strtol(nptr, endptr, base);
}

long long __isoc23_strtoll(const char *nptr, char **endptr, int base) {
    return strtoll(nptr, endptr, base);
}

unsigned long long __isoc23_strtoull(const char *nptr, char **endptr, int base) {
    return strtoull(nptr, endptr, base);
}

int __isoc23_sscanf(const char *str, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int result = vsscanf(str, format, args);
    va_end(args);
    return result;
}
