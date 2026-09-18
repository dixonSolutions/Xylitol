# Notice

## Relationship to Shashlik

Xylitol began as a copy of [shashlik/old-shashlik][upstream], the "Simulated
Android Environment" written by Dan Leinir Turthra Jensen, Inge Wallin and
others between 2014 and 2015. It is a copy rather than a fork: the upstream
history is not carried over, and no pull requests flow back — that project has
been unmaintained since November 2015.

The original sources are kept verbatim under [`legacy/old-shashlik/`](legacy/old-shashlik/)
for reference and attribution. They are not built, and nothing in `crates/` is
a line-by-line translation of them; the Rust code was written against the same
problem, not against the same source. Where the old code informed the new, it
was in shape rather than in text — `libapk`'s job became
[`xylitol-apk`](crates/xylitol-apk/), and the Qt launcher's job became the
runtime handoff in [`xylitol-core`](crates/xylitol-core/src/runtime.rs).

The legacy tree is licensed under the GNU LGPL, version 2 or later, as stated
in the headers of its own files. Xylitol's own code is licensed under the GNU
GPL, version 3 or later — see [LICENSE](LICENSE) — which those terms permit.

`legacy/old-shashlik/platform.pk8` and `platform.x509.pem` are the AOSP test
signing keys that shipped with the original tree. They are public, well-known
development keys with no secret value, and are kept only so the copy is faithful.

## Cordial

The shim's design is taken from [Cordial](https://github.com/luohoa97/cordial)
by luohoa97, which runs Roblox's official Android build on Linux the same way.
Two ideas in particular are theirs: that every symbol an Android object imports
resolves to exactly one of *implemented here*, *forwarded to the host's libc*,
or *not implemented*; and that the third of those must report failure rather
than fake success, because a stub that returns success sends the app off on an
answer that is not true and it fails later somewhere unrelated.

Cordial is GPL-3.0-or-later, as is Xylitol. No Cordial code was copied — the
Rust here was written against the same problem — but the approach is theirs and
worth saying so plainly.

## APKPure

Xylitol downloads from [APKPure](https://apkpure.com/), which is not affiliated
with this project and does not endorse it. Xylitol uses the site's own search
endpoint and public download URLs, with a per-request backoff that respects the
`Retry-After` header. Packages carry the licences of their own publishers.

[upstream]: https://github.com/shashlik/old-shashlik
