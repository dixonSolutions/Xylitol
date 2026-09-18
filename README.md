# Xylitol

Find, download and install Android packages on the Linux desktop — a GTK 4 /
libadwaita app in Rust.

**[Install](#flatpak)** · **[Download page](https://dixonsolutions.github.io/Xylitol/)**

Xylitol is a copy of [Shashlik][shashlik], restarted. Shashlik set out in 2014
to run Android apps on the desktop and stopped in November 2015. Xylitol keeps
the goal: it finds, downloads, verifies and inspects packages, and then runs an
app's native code **in its own process** — no Android runtime, no emulator, no
container, and no instruction translation.

The shim approach is taken from [Cordial][cordial], which runs Roblox's official
Android build on Linux this way. See [NOTICE.md](NOTICE.md) for what was copied
and from where.

## What it does

- **Search APKPure** by app name or package name.
- **List every file a release ships.** An app version is often published as
  several files — one per ABI, sometimes per screen density, sometimes an XAPK
  bundle beside a plain APK. Xylitol shows all of them with their architecture,
  minimum Android version, size and published checksum, and never picks silently.
- **Download with resume and verification.** Interrupted downloads continue
  where they stopped; when APKPure publishes a SHA-1, the file is checked
  against it before it is kept.
- **Read the package.** Xylitol parses `AndroidManifest.xml` itself — no Android
  SDK needed — to show the real package name, version, ABIs, minimum SDK and
  the permissions the app will request.
- **Say whether it can actually run.** Before anything is executed, Xylitol
  reads every native object an app ships and classifies every symbol it imports
  into one of three routes — implemented here, forwarded to the host's libc, or
  not implemented. An app is then a candidate, or it is not, and the reason is
  specific.

## Screens

The app has three: **Discover** (search and pick a file), **Library** (what you
have downloaded, what is in it, and what the shim makes of it) and **Shim**
(what this machine can load).

## Install

### Flatpak

Every commit to `main` is published to a Flatpak remote hosted on GitHub Pages,
so `flatpak update` is all you need after the first install.

```sh
flatpak remote-add --if-not-exists --no-gpg-verify \
    xylitol https://dixonsolutions.github.io/Xylitol/xylitol.flatpakrepo
flatpak install xylitol dev.xylitol.Xylitol
```

```sh
flatpak run dev.xylitol.Xylitol                          # the app
flatpak run --command=xylitol-cli dev.xylitol.Xylitol    # the command line
```

`--no-gpg-verify` is needed because the remote is unsigned by default. The
transport is HTTPS, so this is not the same as no integrity at all, but it does
mean you are trusting GitHub Pages rather than a signature. See
[Signing the remote](#signing-the-remote) to turn signing on for your own fork.

There is also a single-file bundle on the [`continuous`
release](https://github.com/dixonSolutions/Xylitol/releases/tag/continuous) and
at `https://dixonsolutions.github.io/Xylitol/xylitol.flatpak`:

```sh
flatpak install --bundle xylitol.flatpak
```

A bundle does not update itself. Use the remote if you want `flatpak update` to
work.

#### What the sandbox allows

Xylitol asks for network access, a Wayland or X11 socket, and read-only access
to your download folder. That is all: the shim runs the app's code in-process
rather than driving anything on the host, so it needs no way out of the sandbox
— no `--talk-name=org.freedesktop.Flatpak`, no `flatpak-spawn`.

Note what that means for the app's code, though. It runs inside Xylitol's
process, with Xylitol's permissions. The Flatpak sandbox is the boundary; the
shim is not one.

Everything Xylitol downloads goes to its own data directory, which it can always
reach. The download-folder permission is only so that `xylitol-cli inspect`,
`shim` and `library add` can read an APK you obtained some other way; without it
they fail with "No such file or directory" on a path that plainly exists.


### Build requirements

- Rust 1.92 or newer (what the `gtk4` 0.11 bindings require)
- GTK 4.10+ and libadwaita 1.4+ development files

```sh
# Fedora / Bluefin (inside a toolbox or distrobox)
sudo dnf install gtk4-devel libadwaita-devel

# Debian / Ubuntu
sudo apt install libgtk-4-dev libadwaita-1-dev

# Arch
sudo pacman -S gtk4 libadwaita
```

On an atomic host without a container, Homebrew works too — see
[`build-aux/README.md`](build-aux/README.md), which also covers the one
`pkg-config` gap Homebrew leaves.

### Build

```sh
cargo build --release
./target/release/xylitol          # the app
./target/release/xylitol-cli      # the same thing, headless
```

### Flatpak, from source

```sh
flatpak install --user flathub org.gnome.Platform//49 org.gnome.Sdk//49 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08
flatpak-builder --user --install --force-clean \
    build/flatpak build-aux/flatpak/dev.xylitol.Xylitol.yml
```

The GNOME 49 runtime is required rather than preferred: its
`rust-stable` extension carries rustc 1.98, and the `gtk4` 0.11 bindings need
at least 1.92. The 48 runtime ships 1.89 and cannot build this.

The manifest resolves crates from the network during the build rather than
carrying a generated `cargo-sources.json`. That is one less file to regenerate
on every dependency bump, and it is fine for a self-hosted remote — but Flathub
forbids it, so a submission there would need the offline sources adding.

### Signing the remote

The release pipeline signs the repository when a `FLATPAK_GPG_PRIVATE_KEY`
secret is present, and publishes unsigned when it is not. To turn it on in your
own fork:

```sh
gpg --quick-generate-key "Xylitol Releases <you@example.com>" default default never
gpg --export-secret-keys --armor <KEY_ID> | base64 -w0 | \
    gh secret set FLATPAK_GPG_PRIVATE_KEY
```

The next push to `main` will sign the repo and embed the public key in
`xylitol.flatpakrepo`, after which `--no-gpg-verify` is no longer needed. Anyone
who added the remote while it was unsigned should remove and re-add it.

## Using the command line

Everything the app does is scriptable, which is also how the APKPure side is
tested.

```sh
# Find an app
xylitol-cli search vlc

# See its release history
xylitol-cli versions org.videolan.vlc

# See every file the latest release ships — this is the choice
xylitol-cli variants org.videolan.vlc
```

```
3 downloadable file(s) for org.videolan.vlc

   1. 3.7.1          13070106   APK      45.8 MB  arm64-v8a · APK
      Android 5.0+ · Jul 1, 2026 · checksum published
   2. 3.7.1          13070105   APK      44.9 MB  armeabi-v7a · APK
      Android 4.2+ · Jul 1, 2026 · checksum published
   3. 3.7.0          13070009   XAPK    101.4 MB  universal · XAPK
      Android 4.2+ · May 4, 2026 · checksum published
```

Then pick one, by position or by property:

```sh
xylitol-cli download org.videolan.vlc --pick 2
xylitol-cli download org.videolan.vlc --arch arm64-v8a
xylitol-cli download org.videolan.vlc --version 3.7.0 --kind xapk
xylitol-cli download org.videolan.vlc --dry-run      # show, do not fetch
```

Run interactively with nothing but the package name and Xylitol lists the
candidates and asks. Run it in a script and an ambiguous request is an error
unless you pass `--yes`, so a cron job can never quietly fetch the wrong ABI.

Inspect anything you have, whether Xylitol downloaded it or not:

```sh
xylitol-cli inspect ~/Downloads/something.apk
xylitol-cli library list
```

Ask whether an app can run, and why not:

```sh
xylitol-cli shim                             # what this machine can load
xylitol-cli shim ~/Downloads/something.apk   # what this app needs
xylitol-cli shim ~/Downloads/something.apk --unimplemented
```

```
com.termux 0.118.3 (1002)
  host abi   x86_64
  app abis   arm64-v8a, armeabi-v7a, x86, x86_64
  links      libc.so, libdl.so, libm.so, libstdc++.so

  native objects loadable here:
    lib/x86_64/libtermux.so                         0 shim   33 host    0 stub
      5 Java_* natives

  symbols    36 total — 0 shim, 36 host, 0 stub

  verdict    native code exists but only as a JNI library (6 Java_* entry
             point(s)); something has to run the app's Java before any of it
             is called
```

A library key is `package@versionCode#what-makes-it-distinct`, where the last
part is the ABI, `bundle`, a split name, or `base`. Two files of the same
version therefore never collide.

Add `--json` to any command for machine-readable output.

## Where things go

| What | Where |
| --- | --- |
| Downloaded packages | `$XDG_DATA_HOME/xylitol/packages` |
| Library index | `$XDG_STATE_HOME/xylitol/library.json` |

Override with `XYLITOL_DOWNLOAD_DIR` and `XYLITOL_STATE_DIR`. Set `XYLITOL_LOG`
(`error`, `warn`, `info`, `debug`) for logging.

Under Flatpak these land in `~/.var/app/dev.xylitol.Xylitol/`.

## How it is put together

| Crate | Responsibility |
| --- | --- |
| [`xylitol-apk`](crates/xylitol-apk) | Reads APKs and XAPKs: a binary `AndroidManifest.xml` decoder and the metadata on top of it |
| [`xylitol-apkpure`](crates/xylitol-apkpure) | The APKPure client: search, release history, variant listing, resumable verified downloads |
| [`xylitol-shim`](crates/xylitol-shim) | The shim: reads native objects, routes every symbol, implements bionic's deltas |
| [`xylitol-core`](crates/xylitol-core) | The library index, XDG paths, and the front-ends' view of the shim |
| [`xylitol-cli`](crates/xylitol-cli) | The command line, including the selection rules |
| [`xylitol-gui`](crates/xylitol-gui) | The libadwaita app |

APKPure publishes a JSON endpoint for search but nothing official for version
history, so releases and variants are read from the site's markup. That is the
fragile part, so it is isolated in
[`xylitol-apkpure::parse`](crates/xylitol-apkpure/src/parse.rs) and tested
against [saved copies of real pages](crates/xylitol-apkpure/tests/fixtures) —
if APKPure changes its markup, a test fails instead of the UI silently emptying.

Download URLs are the stable, unsigned form
`https://d.apkpure.com/b/APK/<package>?versionCode=<code>`, which APKPure
redirects to a short-lived signed CDN link. Recording one now and fetching it
later therefore works.

## Testing

```sh
cargo test --workspace          # offline: parsers, selection rules, library
cargo test --workspace -- --ignored   # also hits the live site
```

The live tests are `#[ignore]`d by default. APKPure rate-limits by address and
answers `429` with a `Retry-After` measured in tens of minutes, so running them
in a loop is a good way to lock yourself out for an hour. The client honours
`Retry-After` and backs off; the tests do not retry for you.

## Releases

`main` is the release branch. Every commit to it runs
[`release.yml`](.github/workflows/release.yml), which builds the Flatpak,
publishes the ostree repository to GitHub Pages, and refreshes a rolling
`continuous` GitHub Release with the bundle attached.

The Pages site is rebuilt from scratch each time rather than accumulating in a
branch, which keeps binary blobs out of git history. The trade-off is that
clients re-fetch the app rather than applying a delta.

There is no tagged, versioned release yet. When there is, it will be cut by tag
and the `continuous` build will stay where it is.

## How the shim works

An Android `.so` is an ordinary ELF shared object. The format is not the problem;
what it links against is. It names `libc.so`, `libandroid.so`, `liblog.so` — and
those are bionic and the Android framework, not anything on a Linux desktop.

So every symbol an app imports has to resolve one of three ways, and Xylitol
reports the split rather than asking to be trusted:

| route | meaning |
| --- | --- |
| **shim** | Xylitol implements it, because bionic does something glibc does not |
| **host** | forwarded to the host's own libc, because the two agree |
| **stub** | not implemented, and it **reports failure** rather than faking success |

That last row is a rule, not an accident, and it is [Cordial's][cordial]. A stub
that returns success sends the app off on an answer that is not true, and it
fails later somewhere with no relationship to the cause.

## What Xylitol cannot do yet

**It cannot run an app written in Java or Kotlin**, which is most of them. Such
an app keeps its logic in DEX bytecode and calls the `android.*` framework
classes; running one needs a bytecode interpreter and those classes, and Xylitol
has neither. The apps this approach reaches first are the ones whose logic is
already native — NativeActivity and GameActivity games.

**It cannot run code for another CPU.** The shim maps an app's objects into its
own address space, so an arm64 app needs an arm64 machine. Translating would
mean an emulator, which is the thing this exists to avoid.

`xylitol-cli shim <file>` tells you which of these an app falls under, before
anything is executed.

## Licence

GPL-3.0-or-later. The copied Shashlik sources under `legacy/` keep their own
LGPL-2.0-or-later terms. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).

[shashlik]: https://github.com/shashlik/old-shashlik
[cordial]: https://github.com/luohoa97/cordial
