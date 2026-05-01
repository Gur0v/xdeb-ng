# xdeb-ng

`xdeb-ng` converts Debian `.deb` packages into Void Linux `.xbps` packages. The project name is `xdeb-ng`, but the binary is still named `xdeb`.

> Caution: converted packages can overwrite files on your system if you ignore conflict warnings. Read errors before installing generated packages.

## Why xdeb-ng?

`xdeb-ng` keeps the original `xdeb` workflow while replacing the shell implementation with a Rust binary.

- The binary is still `xdeb`, so familiar commands continue to work.
- `xdeb convert`, `xdeb install`, `xdeb info`, `xdeb clean`, and `xdeb doctor` provide clearer entry points.
- `xdeb install` covers the common wrapper flow: get a `.deb`, convert it, then install the generated `.xbps`.
- `--dry-run`, `--explain-deps`, and `xdeb info` let you inspect a package before building or installing it.
- Dependency resolution is host libc aware. On glibc systems, musl dependencies are skipped. On musl systems, glibc dependencies are skipped.
- Safety checks catch unsafe archive paths, unsafe symlinks, invalid metadata, stale shlibs data, unresolved dependencies when requested, file conflicts, and Debian maintainer scripts.
- It stays pragmatic by using existing Void tools: `xbps-create`, `xbps-rindex`, `xbps-install`, `objdump`, `tar`, `ar`, `curl`, and `xz`.

## Requirements

Install the runtime and build dependencies:

```sh
xbps-install binutils tar curl xbps xz rust cargo
```

Check your environment:

```sh
xdeb doctor
```

## Build

```sh
cargo build --release
```

The binary is created at:

```sh
./target/release/xdeb
```

To install it system-wide:

```sh
sudo install -Dm755 ./target/release/xdeb /usr/local/bin/xdeb
```

Optional: set a persistent package root so conversion artifacts do not clutter the current directory.

```sh
export XDEB_PKGROOT="$HOME/.config/xdeb"
```

Generated packages are written to `${XDEB_PKGROOT}/binpkgs`, or `./binpkgs` if `XDEB_PKGROOT` is unset.

## Quick Start

Convert a local `.deb` into an `.xbps` package:

```sh
xdeb convert -Sedf package.deb
```

Install the generated package manually:

```sh
xbps-install -R ./binpkgs <package-name>
```

Convert and install in one step:

```sh
xdeb install package.deb
```

Install from a URL:

```sh
xdeb install --file https://example.invalid/package.deb
```

Preview without building or installing:

```sh
xdeb convert -Sd --dry-run --explain-deps package.deb
```

Inspect metadata:

```sh
xdeb info package.deb
xdeb info --deps --explain-deps package.deb
xdeb info --json package.deb
```

## Commands

### `xdeb convert`

Convert a `.deb` to `.xbps`.

```sh
xdeb convert -Sedf package.deb
```

The classic flag-only form is also supported:

```sh
xdeb -Sedf package.deb
```

Useful options:

- `-S`: sync the Void shlibs dependency list.
- `-d`: resolve automatic shared library dependencies.
- `-e`: remove empty directories from the package.
- `-f`: accepted for compatibility. Conflict fixes are enabled by default.
- `-F`: disable automatic conflict fixes.
- `-R`: do not register the package in the local repository index.
- `-q`: extract only, do not build.
- `-b`: build from an existing `destdir` without extracting a `.deb`.
- `--dry-run`: resolve and print a preview without building.
- `--explain-deps`: print shared library to XBPS dependency mappings.
- `--missing-deps=warn|error`: choose whether unresolved SONAMEs fail conversion.
- `--file-conflicts=warn|error|ignore|auto`: choose conflict handling behavior.

### `xdeb install`

Convert and install a `.deb` in one command.

```sh
xdeb install package.deb
```

Useful options:

- `--file`, `-f`: local `.deb` path or remote URL.
- `--options`, `-o`: conversion flags, default is `-Sde`.
- `--deps`: add manual dependencies.
- `--not-deps`: exclude dependencies from automatic resolution.
- `--dry-run`: convert and preview without installing.
- `--explain-deps`: show dependency mappings.
- `--json`: print machine-readable preview output.
- `--keep-workdir`: keep temporary files after install for debugging.
- `--yes`, `-y`: skip install confirmation.
- `--missing-deps=warn|error`: choose whether unresolved SONAMEs fail installation.

### `xdeb info`

Inspect a `.deb` without building a package.

```sh
xdeb info package.deb
xdeb info --deps --explain-deps package.deb
xdeb info --json package.deb
```

### `xdeb clean`

Remove generated work directories.

```sh
xdeb clean
xdeb clean --repo
xdeb clean --all
```

### `xdeb doctor`

Check required tools, host libc, and shlibs cache state.

```sh
xdeb doctor
```

## Dependency Resolution

Automatic dependency resolution uses Void's `common/shlibs` file and `objdump`.

```sh
xdeb convert -Sd package.deb
```

Use `--explain-deps` to see exactly why dependencies were added:

```sh
xdeb convert -Sd --dry-run --explain-deps package.deb
```

Manual dependencies can be added with `--deps`:

```sh
xdeb convert -Sd --deps='oracle-jre>=8' package.deb
```

Dependencies can be excluded with `--not-deps`:

```sh
xdeb convert -Sd --not-deps='some-package' package.deb
```

Unresolved libraries normally produce warnings. To fail instead:

```sh
xdeb convert -Sd --missing-deps=error package.deb
```

## File Conflicts

XBPS can damage a system if a generated package contains files that conflict with existing system files. `xdeb-ng` checks for conflicts on the machine where conversion runs.

Conflict modes:

- `--file-conflicts=error`: fail when conflicts are found. This is the default.
- `--file-conflicts=warn`: warn but allow the package to be built.
- `--file-conflicts=ignore`: skip conflict checks.
- `--file-conflicts=auto`: apply automatic conflict fixes and fail if conflicts remain.

Automatic conflict fixes move common Debian paths into Void-compatible locations, such as `/bin` to `/usr/bin` and `/lib` to `/usr/lib`.

## Safety Checks

`xdeb-ng` rejects or warns about several risky package features:

- Unsafe archive paths such as absolute paths or `../`.
- Unsafe symlinks that point outside the package root.
- Invalid package names and invalid versions.
- Debian maintainer scripts such as `postinst` and `postrm`, since XBPS will not run them automatically.
- Stale `shlibs` cache metadata.
- Unresolved shared library dependencies when `--missing-deps=error` is used.

## Environment Variables

Common variables:

```sh
export XDEB_PKGROOT="$HOME/.config/xdeb"
export XDEB_OPT_DEPS=true
export XDEB_OPT_INSTALL=true
export XDEB_OPT_FIX_CONFLICT=true
export XDEB_OPT_WARN_CONFLICT=true
export XDEB_COLOR=never
```

`NO_COLOR` is also respected.

## Examples

Hydra Launcher dry run:

```sh
xdeb convert -SdR --dry-run --explain-deps hydralauncher_3.9.7_amd64.deb
```

Build without registering in the local repository:

```sh
xdeb convert -SeR package.deb
```

Build from a previously extracted `destdir`:

```sh
xdeb convert -rb
```

Use a custom package name and version:

```sh
xdeb convert --name=my-package --version=1.2.3 package.deb
```

## Development

Run checks locally:

```sh
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

The integration tests generate small `.deb` fixtures and exercise metadata preview, dry runs, invalid metadata, and unsafe symlink rejection.

## Rationale

Void Linux does not package every proprietary, Electron, or Debian-only application. Manually converting these packages can require cloning `void-packages` and writing templates. `xdeb-ng` automates the practical conversion path while keeping the result inspectable before installation.
