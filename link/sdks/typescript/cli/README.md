# @kalamdb/cli

Install the KalamDB CLI with npm:

```sh
npm install -g @kalamdb/cli
```

Use the global install if you want `kalam` on your shell `PATH`. A local `npm install` only refreshes the package's own `dist/kalam` binary and does not replace some other `kalam` already earlier on `PATH`.

The package bootstraps the native `kalam` binary on first install, then runs `kalam update` to verify checksums and install the correct release build. Version, build-date, and checksum logic live in the Rust CLI — not in this npm wrapper.

If `dist/kalam` (or `dist/kalam.exe`) already exists, postinstall skips bootstrap and delegates directly to `kalam update`.

On Windows, `kalam update` and npm install both rename the running `kalam.exe` aside and put the new binary at the original path before returning. That avoids the "file in use / restart required" failure that happens if a process tries to overwrite an executable it is still running. Uninstall also moves `dist/kalam.exe` out of the package directory first so npm can delete the package while another `kalam` process is still open.

Supported binary targets are `linux-x86_64`, `linux-aarch64`, `macos-aarch64`, and `windows-x86_64`.

Useful commands:

```sh
kalam version
kalam doctor
kalam login --instance prod --url https://db.example.com
kalam whoami
kalam token create --name ci-prod
kalam update
```

Set `KALAM_CLI_VERSION` during install to download a specific release version, or `KALAM_SKIP_DOWNLOAD=1` to skip binary download for packaging workflows. When developing inside the KalamDB monorepo, postinstall also reuses a locally built `target/debug/kalam` (or `target/release/kalam`) before attempting a GitHub release download. Package validation also checks that every supported release asset exists and has a `SHA256SUMS` entry.