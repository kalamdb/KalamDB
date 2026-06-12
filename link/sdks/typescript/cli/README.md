# @kalamdb/cli

Install the KalamDB CLI with npm:

```sh
npm install -g @kalamdb/cli
```

Use the global install if you want `kalam` on your shell `PATH`. A local `npm install` only refreshes the package's own `dist/kalam` binary and does not replace some other `kalam` already earlier on `PATH`.

The package bootstraps the native `kalam` binary on first install, then runs `kalam update` to verify checksums and install the correct release build. Version, build-date, and checksum logic live in the Rust CLI — not in this npm wrapper.

If `dist/kalam` (or `dist/kalam.exe`) already exists, postinstall skips bootstrap and delegates directly to `kalam update`.

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

Set `KALAM_CLI_VERSION` during install to download a specific release version, or `KALAM_SKIP_DOWNLOAD=1` to skip binary download for packaging workflows. Package validation also checks that every supported release asset exists and has a `SHA256SUMS` entry.