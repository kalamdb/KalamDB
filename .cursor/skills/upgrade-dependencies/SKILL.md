---
name: upgrade-dependencies
description: >-
  Checks and upgrades KalamDB dependency versions and known vulnerabilities
  across the Rust workspace, TypeScript, Dart, and Python SDKs, the admin UI,
  examples, benchv2, Docker base images, and GitHub Actions. Use when the user
  asks to upgrade packages, bump dependencies, check outdated packages, run
  npm audit, cargo audit, pub outdated, or review dependency CVEs.
---

# Upgrade dependencies

A dependency upgrade changes libraries this repo consumes. It does not change KalamDB's own version. Leave `[workspace.package].version`, `versions.json`, SDK `version` fields, and peer ranges alone unless the user asked for a product release bump.

Default to a report. Edit manifests only when the user asks to apply upgrades.

## Always check

Discover manifests, then walk every row. Checking one package in a language does not cover the others.

```bash
rg --files -g 'Cargo.toml' -g 'Cargo.lock' -g 'package.json' -g 'package-lock.json' -g 'pubspec.yaml' -g 'pubspec.lock' -g 'pyproject.toml' -g 'Dockerfile*' -g 'build.gradle' -g 'dependabot.yml'
```

Ignore `node_modules`, `target`, and `dist`.

| Surface | Manifest | Lock |
|---|---|---|
| Root Rust workspace | `Cargo.toml` `[workspace.dependencies]` | root `Cargo.lock` |
| Workspace members | `backend/`, `cli/`, `pg/`, `link/link-common`, `link/kalam-link-wasm`, `link/kalam-consumer-wasm`, `link/kalam-link-dart`, `link/sdks/rust`, `link/sdks/rust/examples/quickstart`, `link/sdks/rust-e2e`, `link/sdks/python` | root `Cargo.lock` only |
| benchv2 | `benchv2/Cargo.toml`, `benchv2/comparison/Cargo.toml`, `benchv2/pgwire-speed`, `benchv2/pg18`, `benchv2/kalam-vs-pg` | the `Cargo.lock` beside each |
| Standalone Rust examples | `link/sdks/rust/examples/file-attachments`, `live-inbox`, `topic-consumer` | only a lock that already exists |
| TypeScript SDK | `link/sdks/typescript/{client,consumer,orm,react,cli}` | each `package-lock.json` |
| TypeScript apps | `ui/`, `examples/*/` including `examples/chat-with-ai/functions`, `link/sdks/typescript/client/example`, `link/sdks/typescript/react/tests/e2e/app` | each lock next to its `package.json` |
| Dart | `link/sdks/dart/{link,sync,generator}`, `sync/example`, `sync/example/backend`, `generator/example` | each `pubspec.lock` |
| Python | `link/sdks/python/pyproject.toml` | none for pip; native deps are the Rust workspace |
| Android plugin | `link/sdks/dart/link/android/build.gradle` | none |
| iOS and macOS pods | `link/sdks/dart/link/ios/kalam_link.podspec`, `macos/kalam_link.podspec` | none |
| Container bases | `docker/build/Dockerfile`, `docker/build/Dockerfile.prebuilt`, `tools/Dockerfile*`, `pg/docker/Dockerfile*` | none |
| Toolchain pins | root `rust-version`; `RUST_VERSION`, `WASM_PACK_VERSION`, `PGRX_VERSION` in `.github/workflows/*.yml`; `pg/docker/Dockerfile` `ARG` defaults | n/a |
| GitHub Actions | `.github/workflows/*.yml` `uses:` | n/a |
| Dependabot | `.github/dependabot.yml` | n/a |

`benchv2` is excluded from the root workspace. `file-attachments` has its own `[workspace]`. `live-inbox` and `topic-consumer` are standalone and currently unlocked. A `Cargo.lock` inside a root workspace member is unused; do not refresh it.

The npm CLI package is `link/sdks/typescript/cli`. The Rust CLI is the `cli/` workspace member. There is no root `package.json`.

Dependabot's ecosystem field is empty. It is not coverage.

## Pins that stay put

- Security floor pins in root `Cargo.toml`: `aws-lc-rs`, `rustls`, `rustls-native-certs`, `quinn-proto`, `rustls-webpki`. Raise them when an advisory requires it. Never lower or delete them.
- `pgrx` is an exact `=` pin. Changing it is a Postgres extension upgrade. Check `PGRX_VERSION` in `.github/workflows/release.yml` and `pg/docker/Dockerfile` in the same change. Refreshing the `postgres:16-bookworm` image is not a Postgres major upgrade.
- `link/sdks/dart/link` pins `build_runner` because a newer release pulls an analyzer that conflicts with `freezed`. Read that comment before touching it. `kalam_sync` has its own pin. `kalam_sync_generator` is on a newer analyzer line. Do not unify those three.
- `benchv2` Tokio stays aligned with the root workspace when its comment says so.
- Peer ranges on `@kalamdb/*`, `kalam_link`, and `kalam_sync` are product compatibility, not third-party upgrades.
- `@kalamdb/cli` version equals `[workspace.package].version`. `@kalamdb/client`, `consumer`, `orm`, and `react` share one version. `link/sdks/python/pyproject.toml` version equals `link/sdks/python/Cargo.toml`, which inherits the workspace. The podspec version is not the Dart package version.
- A third-party version written inside a workspace member is a second pin. Update it with the shared entry. Do not migrate it to `workspace = true` as a side effect.
- Rust, wasm-pack, and pgrx pins move only when the user asked to move the toolchain, and every copy moves together.

## Audit

Rust, from the repo root, then again in each independent Rust directory:

```bash
cargo audit
cargo update --dry-run
```

If `cargo-audit` is missing, say so. Do not install it unless asked. For one crate, add `-p <crate>` to `cargo update`.

Each npm project that has a lockfile, from that directory:

```bash
npm outdated
npm audit
```

Report dev findings too, marked dev. `file:` dependencies follow the SDK, and the app still has its own direct dependencies.

Each Dart package:

```bash
dart pub outdated
```

If `osv-scanner` is installed, run it on each `pubspec.lock`. If it is not, record that the Dart advisory scan was skipped.

Python has no lockfile. Read `[project.optional-dependencies]` in `link/sdks/python/pyproject.toml`. Run `pip-audit` only when a virtualenv for that package exists. Otherwise list the ranges and mark the advisory scan skipped.

For container bases, compare `FROM` and `ARG` image tags with the current patched tag on that same major line. For Actions, list floating refs (`@main`, `@master`, unpinned major) and known action advisories. Do not bump either during a library upgrade unless the user included them.

## Apply

One ecosystem at a time.

Rust shared crate: edit `[workspace.dependencies]`, keep its features, then from the repo root:

```bash
cargo update -p <crate>
```

Repeat in each independent lock that uses the crate. Do not run a bare `cargo update` unless the user asked to refresh the whole lock.

npm: edit the range, then in that directory:

```bash
npm install --package-lock-only --ignore-scripts --no-audit --no-fund
```

Name the package. Do not use `npm audit fix --force`.

Dart: edit the constraint, then `dart pub get` in that package. Do not create a lock for an unlocked Rust example unless asked.

After a root workspace dependency change, run one dev-profile `cargo check --workspace`. After a benchv2-only change, run `cargo check` in that bench directory. Do not pass `--release`. After an npm lock refresh, run `npm ci --ignore-scripts --dry-run` in that package.

If a product version file changed by accident, restore it and run `python3 scripts/versions.py verify`.

## Report

```markdown
## Dependency review

| Surface | Package | Current | Wanted | Advisory | Action |
|---|---|---|---|---|---|
| rust workspace | rustls | 0.23.46 | 0.23.x | CVE-... | raise floor pin |

Skipped: <missing tool or unlocked directory>
Left unchanged: product versions, peer ranges, floors already at the required version.
```

Advisories come first. Split patch/minor updates from major or pin changes. Apply a major bump only when the call site still matches; otherwise leave it in the report.

## Do not

- Stop after the root Rust workspace or the TypeScript SDK.
- Lower or delete a security floor pin.
- Bump KalamDB package versions, `versions.json`, or peer ranges while updating dependencies.
- Add a lockfile, Dependabot config, or audit tool the user did not ask for.
- Commit unless asked.
