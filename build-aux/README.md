# Build helpers

## `pkgconfig/` — Homebrew on Linux

Homebrew's `xorgproto` formula installs the X11 protocol headers but no
`pkg-config` files. Brew's own `x11.pc`, `xrender.pc` and `xext.pc` still list
them under `Requires:`, so any `pkg-config` query that reaches Cairo — which
means every `gtk4-sys` build — fails with:

```
Package xproto was not found in the pkg-config search path.
```

`homebrew-env.sh` writes header-only shims into `pkgconfig/` that point at the
`xorgproto` keg, closing the gap without touching the Homebrew prefix. The
generated files are not committed, because they contain your Homebrew prefix.
Source the script and build:

```sh
. build-aux/homebrew-env.sh
cargo build --release
```

Distributions that ship `gtk4-devel`/`libgtk-4-dev` and `libadwaita-devel` need
none of this.
