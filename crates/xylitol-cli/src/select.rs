//! Narrowing a list of downloadable files down to the one to fetch.
//!
//! An app on APKPure usually publishes several files for the same release —
//! one per ABI, sometimes per screen density, sometimes an XAPK bundle beside a
//! plain APK. Picking the wrong one gives an app that will not install, so
//! Xylitol never guesses silently: it filters on what the user asked for and
//! then either has exactly one answer, or asks.

use std::io::{BufRead, Write};

use anyhow::bail;
use xylitol_core::apkpure::{FileKind, Variant};
use xylitol_core::human_size;

/// Restrictions the user placed on the choice.
#[derive(Default, Clone, Copy)]
pub struct Filters<'a> {
    pub version_code: Option<i64>,
    pub arch: Option<&'a str>,
    pub kind: Option<FileKind>,
    /// A 1-based index into the unfiltered list.
    pub pick: Option<usize>,
    /// Take the first remaining candidate rather than prompting or failing.
    pub assume_yes: bool,
    /// Whether there is a user on the other end who can be asked.
    pub interactive: bool,
}

/// Apply `filters` and return the single file to download.
pub fn choose<'a>(all: &'a [Variant], filters: Filters<'_>) -> anyhow::Result<&'a Variant> {
    // An explicit position bypasses every other filter: it refers to the list
    // the user was just shown.
    if let Some(pick) = filters.pick {
        return all.get(pick.wrapping_sub(1)).ok_or_else(|| {
            anyhow::anyhow!(
                "--pick {pick} is out of range: there are {} file(s)",
                all.len()
            )
        });
    }

    let candidates = apply(all, filters);

    match candidates.len() {
        0 => bail!(
            "no file matches that selection. Available:\n{}",
            render_table(all)
        ),
        1 => Ok(candidates[0]),
        // An explicit yes is the only way to have one picked for you. Without
        // it, a script that asked for something ambiguous stops rather than
        // fetching an arbitrary ABI that may not even install.
        _ if filters.assume_yes => Ok(candidates[0]),
        _ if filters.interactive => prompt(&candidates),
        _ => bail!(
            "{} files match; narrow it with --pick, --arch, --version-code or \
             --kind, or pass --yes to take the first:\n{}",
            candidates.len(),
            render_rows(&candidates)
        ),
    }
}

/// The candidates left after filtering, in the order they were given.
pub fn apply<'a>(all: &'a [Variant], filters: Filters<'_>) -> Vec<&'a Variant> {
    all.iter()
        .filter(|v| filters.version_code.is_none_or(|c| v.version_code == c))
        .filter(|v| filters.kind.is_none_or(|k| v.kind == k))
        .filter(|v| match filters.arch {
            None => true,
            Some(want) => matches_arch(v, want),
        })
        .collect()
}

/// Match an ABI request against a variant.
///
/// `universal` also matches a variant with no ABI listed, because APKPure omits
/// the field for builds that carry no native code at all.
fn matches_arch(variant: &Variant, want: &str) -> bool {
    let want = want.trim();
    match variant.arch.as_deref().map(str::trim) {
        Some(have) => {
            have.eq_ignore_ascii_case(want)
                // Multi-ABI builds are listed comma-separated.
                || have
                    .split(',')
                    .any(|part| part.trim().eq_ignore_ascii_case(want))
        }
        None => want.eq_ignore_ascii_case("universal"),
    }
}

/// Render the numbered table used both for listing and for prompting.
pub fn render_table(variants: &[Variant]) -> String {
    let refs: Vec<&Variant> = variants.iter().collect();
    render_rows(&refs)
}

fn render_rows(variants: &[&Variant]) -> String {
    let mut out = String::new();
    for (i, v) in variants.iter().enumerate() {
        let size = v.size.map(human_size).unwrap_or_else(|| "?".into());
        out.push_str(&format!(
            "  {:>2}. {:<14} {:<10} {:<6} {:>10}  {}\n",
            i + 1,
            v.version_name,
            v.version_code,
            v.kind,
            size,
            v.descriptor()
        ));
        let mut detail = Vec::new();
        if let Some(min) = &v.min_android {
            detail.push(min.clone());
        }
        if let Some(when) = &v.published {
            detail.push(when.clone());
        }
        if v.sha1.is_some() {
            detail.push("checksum published".into());
        }
        if !detail.is_empty() {
            out.push_str(&format!("      {}\n", detail.join(" · ")));
        }
    }
    out
}

fn prompt<'a>(candidates: &[&'a Variant]) -> anyhow::Result<&'a Variant> {
    eprintln!("{} files match. Which one?\n", candidates.len());
    eprint!("{}", render_rows(candidates));
    eprint!("\nNumber [1]: ");
    std::io::stderr().flush()?;

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim();
    if line.is_empty() {
        return Ok(candidates[0]);
    }
    let n: usize = line
        .parse()
        .map_err(|_| anyhow::anyhow!("{line:?} is not a number"))?;
    candidates
        .get(n.wrapping_sub(1))
        .copied()
        .ok_or_else(|| anyhow::anyhow!("{n} is out of range: pick 1 to {}", candidates.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(code: i64, arch: Option<&str>, kind: FileKind) -> Variant {
        Variant {
            package: "com.example".into(),
            version_name: "1.0".into(),
            version_code: code,
            kind,
            size: Some(1024),
            published: None,
            arch: arch.map(str::to_string),
            dpi: None,
            min_android: None,
            sha1: None,
            signature: None,
            uploader: None,
            download_url: format!("https://d.apkpure.com/b/APK/com.example?versionCode={code}"),
        }
    }

    fn sample() -> Vec<Variant> {
        vec![
            variant(3, Some("arm64-v8a"), FileKind::Apk),
            variant(3, Some("armeabi-v7a"), FileKind::Apk),
            variant(2, None, FileKind::Xapk),
        ]
    }

    #[test]
    fn filtering_by_arch_narrows_to_one() {
        let all = sample();
        let chosen = choose(
            &all,
            Filters {
                arch: Some("armeabi-v7a"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(chosen.arch.as_deref(), Some("armeabi-v7a"));
    }

    #[test]
    fn universal_matches_a_variant_with_no_abi() {
        let all = sample();
        let chosen = choose(
            &all,
            Filters {
                arch: Some("universal"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(chosen.kind, FileKind::Xapk);
    }

    #[test]
    fn a_multi_abi_build_matches_each_of_its_abis() {
        let v = variant(1, Some("armeabi-v7a, arm64-v8a"), FileKind::Apk);
        assert!(matches_arch(&v, "arm64-v8a"));
        assert!(matches_arch(&v, "armeabi-v7a"));
        assert!(!matches_arch(&v, "x86"));
    }

    #[test]
    fn an_ambiguous_choice_fails_rather_than_picking_for_a_script() {
        let all = sample();
        let err = choose(
            &all,
            Filters {
                version_code: Some(3),
                interactive: false,
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("2 files match"), "got: {err}");
        assert!(
            err.contains("--pick"),
            "the error should say how to resolve it"
        );
    }

    #[test]
    fn an_ambiguous_choice_is_resolved_by_assume_yes_not_guessed() {
        let all = sample();
        let filters = Filters {
            version_code: Some(3),
            ..Default::default()
        };
        // Two candidates remain, so the caller must opt in to taking the first.
        assert_eq!(apply(&all, filters).len(), 2);
        let chosen = choose(
            &all,
            Filters {
                assume_yes: true,
                ..filters
            },
        )
        .unwrap();
        assert_eq!(chosen.arch.as_deref(), Some("arm64-v8a"));
    }

    #[test]
    fn an_impossible_filter_lists_what_was_available() {
        let all = sample();
        let err = choose(
            &all,
            Filters {
                arch: Some("mips"),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("no file matches"));
        assert!(err.contains("arm64-v8a"));
    }

    #[test]
    fn pick_is_one_based_and_range_checked() {
        let all = sample();
        assert_eq!(
            choose(
                &all,
                Filters {
                    pick: Some(1),
                    ..Default::default()
                }
            )
            .unwrap()
            .arch
            .as_deref(),
            Some("arm64-v8a")
        );
        assert!(choose(
            &all,
            Filters {
                pick: Some(0),
                ..Default::default()
            }
        )
        .is_err());
        assert!(choose(
            &all,
            Filters {
                pick: Some(9),
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn kind_filter_selects_bundles() {
        let all = sample();
        let chosen = choose(
            &all,
            Filters {
                kind: Some(FileKind::Xapk),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(chosen.version_code, 2);
    }
}
