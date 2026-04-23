# Design: Commit-Aware Version Display for TUI and CLI

**Date:** 2026-04-23  
**Scope:** Add a commit-aware build-time version string for user-visible surfaces in `agtop`, show it in the TUI top bar, and align release workflow around `cargo-release`.

---

## Goal

Make it easy to tell exactly which build of `agtop` is running during local development and in release builds.

The version should:

- show a clean release version on tagged builds
- show commit-aware dev versions on local builds between tags
- appear consistently in both the TUI and `agtop --version`
- continue to work with the existing GitHub release workflow triggered by `v*.*.*` tags

## Current State

- The workspace version is manually set in root `Cargo.toml` under `[workspace.package]`
- `clap` exposes `--version` from `CARGO_PKG_VERSION`
- The TUI does not currently show any version string
- GitHub release automation already runs on pushed tags matching `v*.*.*`
- There is no automated release/version management tool configured today

## Desired Version Behavior

Use a single build-time version string for all user-visible version surfaces.

Examples:

- tagged release build: `v0.2.0`
- local build 3 commits after `v0.2.0`: `v0.2.0-3-gabcdef1`
- fallback when no matching tag exists: `0.2.0+gabcdef1`

This lets local builds differ from one commit to another, so a rebuilt binary clearly identifies whether it is newer than the previous one.

## Approaches Considered

### Option A: Use only `CARGO_PKG_VERSION`

- Simple, no build script needed
- Fails the main requirement because two local builds from different commits still show the same version

### Option B: Build-time version from git metadata (Recommended)

- Add a `build.rs` to derive a version string from git at compile time
- Works for local development and release builds
- Keeps the displayed version tied to the exact commit/tag state of the binary

### Option C: Runtime version lookup from git

- Could run git commands when the binary starts
- Wrong fit for distributed binaries and non-git environments
- Adds runtime fragility for something that should be resolved at build time

## Recommended Design

### Version Source

Add `build.rs` to `crates/agtop-cli`.

At build time, it should:

1. run `git describe --tags --dirty --always --match "v*"`
2. normalize the result into a user-facing version string
3. export it via `cargo:rustc-env=AGTOP_VERSION=...`

Rust code then reads the version with:

```rust
env!("AGTOP_VERSION")
```

If git metadata is unavailable, fall back to `env!("CARGO_PKG_VERSION")` or a prefixed equivalent chosen in the build script so builds still succeed outside a full git checkout.

### User-Visible Surfaces

Use the same `AGTOP_VERSION` value in both places:

- TUI top bar badge
- `agtop --version`

This avoids drift between interfaces and makes debugging easier.

## TUI Design

The TUI currently renders a single 1-row top status bar in `render_status`.

Change `render_status` to split the row horizontally:

- left pane: existing status content
- right pane: right-aligned dimmed version string

Example:

```text
 agtop [classic]  refresh#3  [1/2] claude:deadbeef                 v0.2.0-3-gabcdef1
```

Implementation shape:

- keep `render_status` signature unchanged
- use a horizontal `Layout` with a flexible left area and fixed-width right area
- render two `Paragraph` widgets
- use `th::STATUS_BAR` as the base style and dim the version text

## CLI Design

Replace clap's default package-version output with the same build-time version string used by the TUI.

Expected behavior:

```bash
agtop --version
agtop v0.2.0-3-gabcdef1
```

This likely means supplying clap an explicit version string instead of relying on the default `CARGO_PKG_VERSION` behavior.

## Release Workflow Design

Adopt `cargo-release` as the supported release tool.

Expected release flow:

1. run `cargo release patch|minor|major`
2. `cargo-release` bumps the workspace version in `Cargo.toml`
3. it creates a release commit and tag like `v0.2.0`
4. it pushes the tag
5. existing GitHub Actions release workflow runs on the tag and builds release artifacts

This works well with the build-time version design because:

- tagged release builds resolve to the clean release version
- later local builds from subsequent commits resolve to commit-aware dev versions

## Affected Files

| File | Change |
|---|---|
| `crates/agtop-cli/build.rs` | New build script to derive and export `AGTOP_VERSION` |
| `crates/agtop-cli/Cargo.toml` | Register build script and any needed build dependency/config |
| `crates/agtop-cli/src/main.rs` | Use build-time version string for clap |
| `crates/agtop-cli/src/tui/mod.rs` | Render right-aligned version badge in top bar |
| `Cargo.toml` and/or release config files | Configure `cargo-release` if needed |
| `README.md` or release docs | Document the new release workflow if needed |

## Error Handling

- If `git describe` succeeds, use its normalized output
- If it fails because git metadata is unavailable, fall back to the Cargo package version so builds remain portable
- Do not fail the build solely because git is unavailable

## Testing

### Functional checks

- `agtop --version` prints the build-time version string
- TUI top bar renders the same string on the right side in both Classic and Dashboard modes

### Manual verification

1. build on a tagged commit and verify a clean `vX.Y.Z`
2. make an extra commit, rebuild, and verify `vX.Y.Z-N-gHASH`
3. confirm the TUI badge and `--version` output match exactly

### Optional unit coverage

- isolate any version normalization logic into a small helper for testability if needed
- keep tests minimal if the logic remains straightforward

## Out of Scope

- adding version text to `--list` or `--json` output
- changing HTTP user-agent version behavior in `agtop-core`
- redesigning the release GitHub Action beyond what is needed to work cleanly with `cargo-release`
