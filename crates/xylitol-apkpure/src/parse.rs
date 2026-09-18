//! HTML scraping for the two APKPure pages Xylitol reads.
//!
//! APKPure has no public API for version history, so these functions parse the
//! website's markup. They are kept separate from the HTTP client so that they
//! can be tested against saved pages without touching the network.

use scraper::{ElementRef, Html, Selector};

use crate::model::{FileKind, Release, Variant};
use crate::Error;

/// Build a selector that is known-good at compile time.
fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector must parse")
}

fn text_of(el: ElementRef<'_>) -> String {
    el.text().collect::<String>().trim().to_string()
}

/// Parse `https://apkpure.com/<slug>/<package>/versions`.
pub fn releases(html: &str, package: &str) -> Result<Vec<Release>, Error> {
    let doc = Html::parse_document(html);
    let item = sel("div.ver_download_link");
    let tag = sel("span.apk-type-tag");
    let when = sel("span.update-on");

    let mut out = Vec::new();
    for el in doc.select(&item) {
        let version_name = el
            .attr("data-dt-version")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string();

        let version_code = el.attr("data-dt-versioncode").and_then(|v| v.parse().ok());

        // A release may be published in both formats, so collect every tag
        // rather than stopping at the first. The apkid path is a fallback for
        // markup that carries no tag list at all.
        let mut kinds: Vec<FileKind> = Vec::new();
        for kind in el
            .select(&tag)
            .filter_map(|t| t.attr("data-tag"))
            .filter_map(FileKind::parse)
        {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
        if kinds.is_empty() {
            kinds.push(
                el.attr("data-dt-apkid")
                    .and_then(|id| id.split('/').nth(1))
                    .and_then(FileKind::parse)
                    .unwrap_or(FileKind::Apk),
            );
        }

        let size_bytes = el.attr("data-dt-filesize").and_then(|v| v.parse().ok());

        let published = el
            .select(&when)
            .next()
            .map(text_of)
            .filter(|s| !s.is_empty());

        let has_variants = el.attr("data-dt-variant") == Some("true")
            || el
                .attr("data-dt-apklist")
                .map(|l| l.contains(','))
                .unwrap_or(false);

        out.push(Release {
            package: package.to_string(),
            version_name,
            version_code,
            kinds,
            size: size_bytes,
            published,
            has_variants,
        });
    }

    if out.is_empty() && !looks_like_app_page(&doc) {
        return Err(Error::NotFound(package.to_string()));
    }
    Ok(out)
}

/// Parse the "All Variants of the App" block on a download page.
pub fn variants(html: &str, package: &str) -> Result<Vec<Variant>, Error> {
    let doc = Html::parse_document(html);
    let apk = sel("div#version-list div.apk");
    let name = sel("div.info-top span.name");
    let code = sel("div.info-top span.code");
    let tag = sel("div.info-top span.tag");
    let time = sel("div.info-bottom span.time");
    let size = sel("div.info-bottom span.size");
    let sdk = sel("div.info-bottom span.sdk");
    let btn = sel("a.download-btn");
    let detail_row = sel("div.variants-desc-dialog div.content p");
    let label = sel("span.label");
    let value = sel("span.value");

    let mut out = Vec::new();
    for el in doc.select(&apk) {
        let Some(href) = el.select(&btn).next().and_then(|a| a.attr("href")) else {
            // A row without a download button is not something we can offer.
            continue;
        };

        let version_name = el.select(&name).next().map(text_of).unwrap_or_default();
        let version_code = el
            .select(&code)
            .next()
            .map(text_of)
            .and_then(|t| t.trim_matches(['(', ')']).parse::<i64>().ok())
            // The versionCode is also in the download URL; prefer whichever parses.
            .or_else(|| {
                href.split_once("versionCode=")
                    .and_then(|(_, v)| v.split('&').next())
                    .and_then(|v| v.parse().ok())
            });
        let Some(version_code) = version_code else {
            continue;
        };

        let kind = el
            .select(&tag)
            .next()
            .and_then(|t| t.attr("data-tag").map(str::to_string).or(Some(text_of(t))))
            .and_then(|t| FileKind::parse(&t))
            .unwrap_or(FileKind::Apk);

        // Read the per-variant detail dialog: Architecture, DPI, SHA-1, ...
        let mut arch = None;
        let mut dpi = None;
        let mut sha1 = None;
        let mut signature = None;
        let mut uploader = None;
        for p in el.select(&detail_row) {
            let (Some(k), Some(v)) = (p.select(&label).next(), p.select(&value).next()) else {
                continue;
            };
            let k = text_of(k).to_ascii_lowercase();
            let v = text_of(v);
            if v.is_empty() {
                continue;
            }
            match k.as_str() {
                "architecture" => arch = Some(v),
                "screen dpi" => dpi = Some(v),
                "file sha1" => sha1 = Some(v.to_ascii_lowercase()),
                "signature" => signature = Some(v.to_ascii_lowercase()),
                "uploaded by" => uploader = Some(v),
                _ => {}
            }
        }

        out.push(Variant {
            package: package.to_string(),
            version_name,
            version_code,
            kind,
            size: el
                .select(&size)
                .next()
                .map(text_of)
                .and_then(|s| parse_size(&s)),
            published: el
                .select(&time)
                .next()
                .map(text_of)
                .filter(|s| !s.is_empty()),
            arch,
            dpi,
            min_android: el
                .select(&sdk)
                .next()
                .map(text_of)
                .filter(|s| !s.is_empty()),
            sha1,
            signature,
            uploader,
            download_url: absolute(href),
        });
    }

    if out.is_empty() && !looks_like_app_page(&doc) {
        return Err(Error::NotFound(package.to_string()));
    }
    Ok(out)
}

/// APKPure serves a soft-404 app page rather than an HTTP error for unknown
/// packages, so distinguish "no versions listed" from "no such app".
fn looks_like_app_page(doc: &Html) -> bool {
    doc.select(&sel("body"))
        .next()
        .and_then(|b| b.attr("data-pkg"))
        .is_some()
        || doc.select(&sel("div.ver-wrap")).next().is_some()
        || doc.select(&sel("div#version-list")).next().is_some()
}

fn absolute(href: &str) -> String {
    if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://apkpure.com{href}")
    } else {
        href.to_string()
    }
}

/// Turn `44.9 MB` into a byte count. Approximate by nature — the exact size
/// comes from the HTTP response, this is only for display before downloading.
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let idx = s.find(|c: char| c.is_alphabetic())?;
    let (num, unit) = s.split_at(idx);
    let num: f64 = num.trim().replace(',', "").parse().ok()?;
    let mult = match unit.trim().to_ascii_uppercase().as_str() {
        "B" => 1.0,
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((num * mult) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_round_trip() {
        assert_eq!(parse_size("44.9 MB"), Some(47_081_062));
        assert_eq!(parse_size("512 KB"), Some(524288));
        assert_eq!(parse_size("nonsense"), None);
    }

    #[test]
    fn relative_urls_become_absolute() {
        assert_eq!(absolute("/a/b"), "https://apkpure.com/a/b");
        assert_eq!(absolute("//d.apkpure.com/x"), "https://d.apkpure.com/x");
        assert_eq!(absolute("https://e/x"), "https://e/x");
    }

    #[test]
    fn parses_a_variant_block() {
        let html = r#"
        <body data-pkg="com.example">
        <div id="version-list"><div class="apk">
          <div class="info">
            <div class="info-top"><span class="name">1.2.3</span><span class="code">(45)</span>
              <span class="tag" data-tag="XAPK">XAPK</span></div>
            <div class="info-bottom"><span class="time">Jul 1, 2026</span>
              <span class="size">44.9 MB</span><span class="sdk">Android 4.2+</span></div>
          </div>
          <a class="download-btn" href="https://d.apkpure.com/b/XAPK/com.example?versionCode=45"></a>
          <div class="variants-desc-dialog"><div class="content">
            <p><span class="label">Architecture</span><span class="value">arm64-v8a</span></p>
            <p><span class="label">Screen DPI</span><span class="value">nodpi</span></p>
            <p><span class="label">File SHA1</span><span class="value">ABCDEF</span></p>
          </div></div>
        </div></div></body>"#;
        let v = variants(html, "com.example").unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].version_code, 45);
        assert_eq!(v[0].kind, FileKind::Xapk);
        assert_eq!(v[0].arch.as_deref(), Some("arm64-v8a"));
        assert_eq!(v[0].sha1.as_deref(), Some("abcdef"));
        assert_eq!(v[0].descriptor(), "arm64-v8a · XAPK");
    }

    #[test]
    fn parses_a_release_row() {
        let html = r#"<body data-pkg="com.example"><ul class="ver-wrap"><li>
          <div class="ver_download_link" data-dt-version="3.7.1" data-dt-versioncode="13070106"
               data-dt-filesize="47990842" data-dt-variant="true">
            <span class="apk-type-tag" data-tag="APK">APK</span>
            <span class="ver-item-s">45.8 MB</span>
            <span class="update-on">Jul 1, 2026</span>
          </div></li></ul></body>"#;
        let r = releases(html, "com.example").unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].version_code, Some(13070106));
        assert_eq!(r[0].size, Some(47990842));
        assert_eq!(r[0].kinds, vec![FileKind::Apk]);
        assert!(r[0].has_variants);
    }

    #[test]
    fn a_release_published_in_both_formats_keeps_both() {
        let html = r#"<body data-pkg="com.example"><ul class="ver-wrap"><li>
          <div class="ver_download_link" data-dt-version="3.7.0" data-dt-versioncode="13070009">
            <div class="apk-type-tag-list">
              <span class="apk-type-tag" data-tag="APK">APK</span>
              <span class="apk-type-tag" data-tag="XAPK">XAPK</span>
            </div>
          </div></li></ul></body>"#;
        let r = releases(html, "com.example").unwrap();
        assert_eq!(r[0].kinds, vec![FileKind::Apk, FileKind::Xapk]);
        assert_eq!(r[0].kinds_display(), "APK + XAPK");
        assert!(r[0].offers(FileKind::Xapk));
    }

    #[test]
    fn unknown_package_is_reported_as_not_found() {
        let err = releases("<html><body>nope</body></html>", "com.nope").unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }
}
