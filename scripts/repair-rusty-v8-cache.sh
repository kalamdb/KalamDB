#!/usr/bin/env bash
# A prefix restore of the Rust build cache can keep v8's build-script output
# while dropping the prebuilt static library. Rustc then fails with
# "could not find native static library `rusty_v8`". Drop those units so the
# build script downloads the library again.
set -euo pipefail

if [[ ! -d target ]]; then
  exit 0
fi

shopt -s nullglob
for profile in target/* target/*/*; do
  [[ -d "$profile" ]] || continue
  units=("$profile"/.fingerprint/v8-*)
  if [[ ${#units[@]} -eq 0 && ! -d "$profile/build/v8" ]]; then
    continue
  fi
  if [[ -f "$profile/build/gn_out/obj/librusty_v8.a" || -f "$profile/build/gn_out/obj/rusty_v8.lib" ]]; then
    continue
  fi
  echo "rusty_v8 static library missing under $profile; clearing v8 build state"
  rm -rf "$profile/build/v8" "${units[@]}"
done
