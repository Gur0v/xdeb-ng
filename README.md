# xdeb-ng

Convert Debian `.deb` packages into Void Linux `.xbps` packages.

The project is called `xdeb-ng`, but the binary is still `xdeb` — old muscle memory still works.

> **Caution:** Converted packages can overwrite files on your system if you ignore conflict warnings. Read the output before installing anything.

## Overview

`xdeb-ng` keeps the original `xdeb` workflow, but replaces the shell script with a Rust binary. Same idea, less mess.

- Binary name stays `xdeb`
- Clear subcommands: `convert`, `install`, `info`, `clean`, `doctor`
- `xdeb install` handles the common flow end-to-end
- Dry runs and inspection are first-class
- Dependency resolution respects your libc (glibc vs musl)
- Safety checks are stricter and less trusting
- Still uses Void tools instead of reinventing everything

It is not trying to be a packaging system. It is a practical conversion tool.

## Why This Exists

Void does not package everything. Sometimes you just want to take a `.deb`, convert it, inspect it, and install it — without writing a full template or cloning `void-packages`. This does that. Nothing more.

## Requirements

```sh
xbps-install binutils tar curl xbps xz rust cargo
```

Check your setup:

```sh
xdeb doctor
```

## Build

```sh
cargo build --release
```

The binary will be at `./target/release/xdeb`. To install it:

```sh
sudo install -Dm755 ./target/release/xdeb /usr/local/bin/xdeb
```

Optionally, set a custom package root:

```sh
export XDEB_PKGROOT="$HOME/.config/xdeb"
```

## Quick Start

**Convert:**
```sh
xdeb convert -Sedf package.deb
```

**Install after converting:**
```sh
xbps-install -R ./binpkgs <package-name>
```

**One-shot convert and install:**
```sh
xdeb install package.deb
```

**From a URL:**
```sh
xdeb install --file https://example.invalid/package.deb
```

**Bypass strict symlink checks (e.g. for Zoom):**
```sh
xdeb convert -Sd -i package.deb
```

**Preview only:**
```sh
xdeb convert -Sd --dry-run --explain-deps package.deb
```

**Inspect a package:**
```sh
xdeb info package.deb
```

## Commands

### `convert`

Convert a `.deb` into an `.xbps` package.

```sh
xdeb convert -Sedf package.deb
```

The classic form still works:

```sh
xdeb -Sedf package.deb
```

| Flag | Description |
|------|-------------|
| `-S` | Sync shlibs |
| `-d` | Resolve shared libraries |
| `-e` | Remove empty directories |
| `-i` | Bypass strict path verification / suppress safety warnings |
| `-F` | Disable conflict fixes |
| `-R` | Skip repo registration |
| `--dry-run` | Preview only, do not build |
| `--explain-deps` | Show SONAME → XBPS dependency mapping |
| `--missing-deps=warn\|error` | Control behaviour on unresolved deps |
| `--file-conflicts=warn\|error\|ignore\|auto` | Control conflict handling |

### `install`

Convert and install in one step:

```sh
xdeb install package.deb
```

### `info`

Inspect a package without building it:

```sh
xdeb info package.deb
```

### `clean`

```sh
xdeb clean
```

### `doctor`

```sh
xdeb doctor
```

## Dependency Resolution

Uses Void's `common/shlibs` combined with `objdump`.

```sh
# Resolve deps
xdeb convert -Sd package.deb

# Show mapping
xdeb convert -Sd --dry-run --explain-deps package.deb

# Fail on unresolved deps
xdeb convert -Sd --missing-deps=error package.deb
```

## File Conflicts

By default, conflicts fail the build. This can be changed with `--file-conflicts`:

| Mode | Behaviour |
|------|-----------|
| `error` | Fail on conflicts (default) |
| `warn` | Warn but continue |
| `ignore` | Silently ignore conflicts |
| `auto` | Attempt fixes, fail if still needed |

Automatic fixes handle common Debian → Void path differences.

## Safety & Quirks

`xdeb-ng` rejects or warns about:

- Unsafe archive paths (`../`, absolute paths)
- Unsafe symlinks
- Invalid metadata
- Maintainer scripts
- Stale shlibs data
- Unresolved dependencies (if configured)

### The Absolute Symlink Quirk

Some commercial packages (like Zoom) bundle absolute symlinks inside their data archive that point to directories outside the working extraction directory (e.g. `/usr/bin/zoom` → `/opt/zoom/ZoomLauncher`).

Because `xdeb-ng` validates archive structure during extraction, it will flag these as **unsafe symlinks escaping the package root** — even if `--file-conflicts=ignore` is set. To bypass this, pass `-i`:

```sh
xdeb convert -Sd -i zoom_amd64.deb
```

If the symlink is broken after installation, recreate it manually:

```sh
sudo ln -s /opt/zoom/ZoomLauncher /usr/bin/zoom
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `XDEB_PKGROOT` | Override the package root directory |
| `XDEB_COLOR` | Set to `never` to disable colour output |
| `NO_COLOR` | Respected if set |

## Philosophy

- Do one job
- Make it inspectable
- Fail loudly when things look wrong
- Do not pretend converted packages are safe

This is a convenience tool, not a guarantee.

## Development

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```
