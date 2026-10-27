#!/bin/sh
# CI compiler wrapper for C++ build scripts.
#
# Cargo does not rebuild librocksdb-sys when only its package opt-level
# changes, so a restored runner cache can keep RocksDB compiled at -O0.
# Unoptimized DB::Open then overflows the Linux thread stack. gcc uses the
# last -O flag, so -O3 is appended only for RocksDB sources. Other crates,
# including aws-lc, keep the -O level on their own command line.
src=
prev=
for arg in "$@"; do
    if [ "$prev" = "-c" ]; then
        src=$arg
    fi
    prev=$arg
done

case $src in
    *rocksdb/* | *librocksdb-sys*)
        exec c++ "$@" -O3
        ;;
    *)
        exec c++ "$@"
        ;;
esac
