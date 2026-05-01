# xdeb-ng

`xdeb-ng` converts Debian `.deb` packages into Void Linux `.xbps` packages.

The project is called `xdeb-ng`, but the binary is still `xdeb`. Old muscle memory still works.

> Caution: converted packages can overwrite files on your system if you ignore conflict warnings. Read the output before installing anything.

## What This Is

`xdeb-ng` keeps the original `xdeb` workflow, but replaces the shell script with a Rust binary.

Same idea. Less mess.

* binary name stays `xdeb`
* clear subcommands: `convert`, `install`, `info`, `clean`, `doctor`
* `xdeb install` handles the common flow end-to-end
* dry runs and inspection are first-class
* dependency resolution respects your libc (glibc vs musl)
* safety checks are stricter and less trusting
* still uses Void tools instead of reinventing everything

It is not trying to be a packaging system. It is a practical conversion tool.

## Why This Exists

Void does not package everything.

Sometimes you just want to take a `.deb`, convert it, inspect it, and install it without writing a full template or cloning `void-packages`.

This does that. Nothing more.

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

Binary:

```text
./target/release/xdeb
```

Install:

```sh
sudo install -Dm755 ./target/release/xdeb /usr/local/bin/xdeb
```

Optional package root:

```sh
export XDEB_PKGROOT="$HOME/.config/xdeb"
```

## Quick Start

Convert:

```sh
xdeb convert -Sedf package.deb
```

Install:

```sh
xbps-install -R ./binpkgs <package-name>
```

One-shot:

```sh
xdeb install package.deb
```

From URL:

```sh
xdeb install --file https://example.invalid/package.deb
```

Preview only:

```sh
xdeb convert -Sd --dry-run --explain-deps package.deb
```

Inspect:

```sh
xdeb info package.deb
```

## Commands

### convert

Convert `.deb` → `.xbps`.

```sh
xdeb convert -Sedf package.deb
```

Classic form still works:

```sh
xdeb -Sedf package.deb
```

Key options:

* `-S` sync shlibs
* `-d` resolve shared libs
* `-e` remove empty dirs
* `-F` disable conflict fixes
* `-R` skip repo registration
* `--dry-run` preview only
* `--explain-deps` show mapping
* `--missing-deps=warn|error`
* `--file-conflicts=warn|error|ignore|auto`

### install

Convert and install in one step:

```sh
xdeb install package.deb
```

### info

Inspect without building:

```sh
xdeb info package.deb
```

### clean

```sh
xdeb clean
```

### doctor

```sh
xdeb doctor
```

## Dependency Resolution

Uses Void’s `common/shlibs` + `objdump`.

```sh
xdeb convert -Sd package.deb
```

Explain:

```sh
xdeb convert -Sd --dry-run --explain-deps package.deb
```

Fail on missing:

```sh
xdeb convert -Sd --missing-deps=error package.deb
```

## File Conflicts

By default, conflicts fail the build.

You can change behavior:

* `error` (default)
* `warn`
* `ignore`
* `auto` (attempt fixes, then fail if needed)

Automatic fixes handle common Debian → Void path differences.

## Safety

`xdeb-ng` rejects or warns about:

* unsafe archive paths (`../`, absolute paths)
* unsafe symlinks
* invalid metadata
* maintainer scripts
* stale shlibs data
* unresolved dependencies (if configured)

If something looks sketchy, it probably is.

## Environment

```sh
export XDEB_PKGROOT="$HOME/.config/xdeb"
export XDEB_COLOR=never
```

`NO_COLOR` is respected.

## Philosophy

* do one job
* make it inspectable
* fail loudly when things look wrong
* do not pretend converted packages are safe

This is a convenience tool, not a guarantee.

## Development

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Upstream

Original `xdeb` idea, rewritten.
