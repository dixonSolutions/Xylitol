# Xylitol

Find, download and install Android packages on the Linux desktop — a GTK 4 /
libadwaita app in Rust.

Xylitol is a copy of [Shashlik][shashlik], restarted. Shashlik set out in 2014
to run Android apps on the desktop by carrying its own AOSP-derived runtime, and
stopped in November 2015 when that became untenable. Xylitol keeps the goal and
drops the runtime: it handles finding, choosing, verifying and inspecting
packages itself, and hands the finished file to an Android runtime that is
already on the system. See [NOTICE.md](NOTICE.md) for what was copied and why.

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
- **Install it** through Waydroid or `adb`, including XAPK bundles, which are
  unpacked and installed as a single split transaction.

## Screens

The app has three: **Discover** (search and pick a file), **Library** (what you
have downloaded, and what is in it) and **Runtime** (which Android runtime can
be used right now).

## Install

### Build requirements

- Rust 1.82 or newer
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
xylitol-cli runtime status
xylitol-cli runtime install 'org.videolan.vlc@13070105#armeabi-v7a'
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

## How it is put together

| Crate | Responsibility |
| --- | --- |
| [`xylitol-apk`](crates/xylitol-apk) | Reads APKs and XAPKs: a binary `AndroidManifest.xml` decoder and the metadata on top of it |
| [`xylitol-apkpure`](crates/xylitol-apkpure) | The APKPure client: search, release history, variant listing, resumable verified downloads |
| [`xylitol-core`](crates/xylitol-core) | The library index, XDG paths, and the Waydroid/adb handoff |
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

## What Xylitol does not do

It does not run Android. Reviving Shashlik's runtime would mean maintaining an
Android 4.4-era AOSP tree and its Wayland `gralloc`/`hwcomposer` shims, which is
what killed the original. Waydroid already does that job properly, so Xylitol
drives it rather than competing with it.

## Licence

GPL-3.0-or-later. The copied Shashlik sources under `legacy/` keep their own
LGPL-2.0-or-later terms. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).

[shashlik]: https://github.com/shashlik/old-shashlik
