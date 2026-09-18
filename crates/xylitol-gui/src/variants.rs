//! Choosing which file to download.
//!
//! An app's release usually exists as several files — one per ABI, sometimes
//! per screen density, sometimes an XAPK bundle beside a plain APK. Installing
//! the wrong one simply fails, so this page never picks for the user: it lists
//! every file APKPure hosts, grouped by release, with the ABI filter set to the
//! host's own architecture as a starting point.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use xylitol_core::apkpure::{CancelToken, Downloaded, Progress, Variant};
use xylitol_core::{human_size, paths};

use crate::state::Shared;
use crate::tasks;

const ANY_ARCH: &str = "Any architecture";

/// Open the file picker for one app.
pub fn push(ctx: &Shared, package: &str, title: &str) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);

    let spinner = gtk::Spinner::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .build();
    spinner.start();
    content.append(&spinner);

    let clamp = adw::Clamp::builder()
        .maximum_size(720)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .child(&content)
        .build();

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroller));

    let page = adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build();
    ctx.nav.push(&page);

    let client = ctx.client.clone();
    let package_owned = package.to_string();
    tasks::spawn(
        async move { client.variants(&package_owned, None).await },
        {
            let ctx = ctx.clone();
            let package = package.to_string();
            move |outcome| {
                content.remove(&spinner);
                match outcome {
                    Ok(variants) if variants.is_empty() => {
                        content.append(&empty_state(
                            "Nothing to download",
                            &format!("APKPure lists no files for {package}."),
                        ));
                    }
                    Ok(variants) => fill(&ctx, &content, variants),
                    Err(e) => {
                        content
                            .append(&empty_state("Could not load the file list", &e.to_string()));
                    }
                }
            }
        },
    );
}

fn empty_state(title: &str, description: &str) -> adw::StatusPage {
    adw::StatusPage::builder()
        .icon_name("dialog-warning-symbolic")
        .title(title)
        .description(description)
        .vexpand(true)
        .build()
}

fn fill(ctx: &Shared, content: &gtk::Box, variants: Vec<Variant>) {
    let variants = Rc::new(variants);

    let mut architectures = vec![ANY_ARCH.to_string()];
    for variant in variants.iter() {
        let arch = variant.arch.clone().unwrap_or_else(|| "universal".into());
        if !architectures.contains(&arch) {
            architectures.push(arch);
        }
    }

    let arch_strs: Vec<&str> = architectures.iter().map(String::as_str).collect();
    let arch_filter = gtk::DropDown::from_strings(&arch_strs);
    // Default to the architecture this machine runs, when that is on offer:
    // it is the only build that can work under an emulator-free runtime.
    if let Some(index) = architectures.iter().position(|a| a == host_abi()) {
        arch_filter.set_selected(index as u32);
    }

    let kind_filter = gtk::DropDown::from_strings(&["Any format", "APK only", "XAPK only"]);

    let filter_row = adw::ActionRow::builder()
        .title("Show")
        .subtitle("Only files matching these will be listed")
        .build();
    let filter_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    filter_box.set_valign(gtk::Align::Center);
    filter_box.append(&arch_filter);
    filter_box.append(&kind_filter);
    filter_row.add_suffix(&filter_box);

    let filter_group = adw::PreferencesGroup::new();
    filter_group.add(&filter_row);
    content.append(&filter_group);

    let list_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.append(&list_box);

    let architectures = Rc::new(architectures);
    let rebuild: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        let variants = variants.clone();
        let list_box = list_box.clone();
        let arch_filter = arch_filter.clone();
        let kind_filter = kind_filter.clone();
        let architectures = architectures.clone();
        Rc::new(move || {
            let wanted_arch = architectures
                .get(arch_filter.selected() as usize)
                .filter(|a| a.as_str() != ANY_ARCH)
                .cloned();
            let wanted_kind = match kind_filter.selected() {
                1 => Some(xylitol_core::apkpure::FileKind::Apk),
                2 => Some(xylitol_core::apkpure::FileKind::Xapk),
                _ => None,
            };

            let matching: Vec<&Variant> = variants
                .iter()
                .filter(|v| match &wanted_arch {
                    None => true,
                    Some(want) => v
                        .arch
                        .as_deref()
                        .unwrap_or("universal")
                        .eq_ignore_ascii_case(want),
                })
                .filter(|v| wanted_kind.is_none_or(|k| v.kind == k))
                .collect();

            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            if matching.is_empty() {
                list_box.append(&empty_state(
                    "No matching files",
                    "Widen the filter to see the other builds of this app.",
                ));
                return;
            }

            // One group per release, newest first; `variants` is already sorted.
            let mut current_code = None;
            let mut group: Option<adw::PreferencesGroup> = None;
            for variant in matching {
                if current_code != Some(variant.version_code) {
                    current_code = Some(variant.version_code);
                    let new_group = adw::PreferencesGroup::builder()
                        .title(format!(
                            "{} ({})",
                            gtk::glib::markup_escape_text(&variant.version_name),
                            variant.version_code
                        ))
                        .description(variant.published.clone().unwrap_or_default())
                        .build();
                    list_box.append(&new_group);
                    group = Some(new_group);
                }
                if let Some(group) = &group {
                    group.add(&variant_row(&ctx, variant));
                }
            }
        })
    };

    arch_filter.connect_selected_notify({
        let rebuild = rebuild.clone();
        move |_| rebuild()
    });
    kind_filter.connect_selected_notify({
        let rebuild = rebuild.clone();
        move |_| rebuild()
    });
    rebuild();
}

fn variant_row(ctx: &Shared, variant: &Variant) -> adw::ActionRow {
    let mut detail = Vec::new();
    if let Some(size) = variant.size {
        detail.push(human_size(size));
    }
    if let Some(min) = &variant.min_android {
        detail.push(min.clone());
    }
    if let Some(dpi) = &variant.dpi {
        if dpi != "nodpi" {
            detail.push(dpi.clone());
        }
    }
    detail.push(
        if variant.sha1.is_some() {
            "checksum published"
        } else {
            "no checksum published"
        }
        .to_string(),
    );

    let row = adw::ActionRow::builder()
        .title(gtk::glib::markup_escape_text(&variant.descriptor()))
        .subtitle(gtk::glib::markup_escape_text(&detail.join(" · ")))
        .build();

    let suffix = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    suffix.set_valign(gtk::Align::Center);

    let button = gtk::Button::builder()
        .label("Download")
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("suggested-action");
    suffix.append(&button);
    row.add_suffix(&suffix);

    button.connect_clicked({
        let ctx = ctx.clone();
        let variant = variant.clone();
        let suffix = suffix.clone();
        let row = row.clone();
        move |button| {
            button.set_visible(false);
            start_download(&ctx, &row, &suffix, variant.clone(), button.clone());
        }
    });

    row
}

fn start_download(
    ctx: &Shared,
    row: &adw::ActionRow,
    suffix: &gtk::Box,
    variant: Variant,
    button: gtk::Button,
) {
    let progress = gtk::ProgressBar::builder()
        .valign(gtk::Align::Center)
        .width_request(140)
        .show_text(true)
        .text("Starting…")
        .build();

    let cancel_button = gtk::Button::builder()
        .icon_name("process-stop-symbolic")
        .valign(gtk::Align::Center)
        .tooltip_text("Cancel")
        .build();
    cancel_button.add_css_class("flat");

    suffix.append(&progress);
    suffix.append(&cancel_button);

    let cancel = CancelToken::new();
    cancel_button.connect_clicked({
        let cancel = cancel.clone();
        move |button| {
            button.set_sensitive(false);
            cancel.cancel();
        }
    });

    let client = ctx.client.clone();
    let dir = paths::download_dir();
    let variant_for_task = variant.clone();
    let cancel_for_task = cancel.clone();

    // Throttle: the row only needs to move at screen refresh rate, not per chunk.
    let last_shown = Rc::new(RefCell::new(0.0f64));

    tasks::spawn_with_progress(
        move |tx| async move {
            client
                .download(
                    &variant_for_task,
                    &dir,
                    &cancel_for_task,
                    move |p: Progress| {
                        let _ = tx.try_send(p);
                    },
                )
                .await
        },
        {
            let progress = progress.clone();
            move |p: Progress| match p.fraction() {
                Some(fraction) => {
                    let mut last = last_shown.borrow_mut();
                    if (fraction - *last).abs() < 0.005 && fraction < 1.0 {
                        return;
                    }
                    *last = fraction;
                    progress.set_fraction(fraction);
                    progress.set_text(Some(&format!("{:.0}%", fraction * 100.0)));
                }
                None => {
                    progress.pulse();
                    progress.set_text(Some(&human_size(p.downloaded)));
                }
            }
        },
        {
            let ctx = ctx.clone();
            let row = row.clone();
            let suffix = suffix.clone();
            let progress = progress.clone();
            move |outcome: Result<Downloaded, xylitol_core::apkpure::Error>| {
                suffix.remove(&progress);
                suffix.remove(&cancel_button);
                match outcome {
                    Ok(done) => {
                        finish(&ctx, &row, &suffix, &variant, done);
                    }
                    Err(xylitol_core::apkpure::Error::Cancelled) => {
                        // The partial file is kept, so offer to carry on rather
                        // than treating the user's own choice as a failure.
                        button.set_visible(true);
                        button.set_label("Resume");
                        ctx.toast("Download stopped");
                    }
                    Err(e) => {
                        button.set_visible(true);
                        button.set_label("Retry");
                        ctx.toast_error("Download failed", e);
                    }
                }
            }
        },
    );
}

fn finish(
    ctx: &Shared,
    row: &adw::ActionRow,
    suffix: &gtk::Box,
    variant: &Variant,
    done: Downloaded,
) {
    let added = ctx
        .library
        .borrow_mut()
        .add(&done.path, Some(variant.clone()), done.verified);

    match added {
        Ok(_) => {
            ctx.library_changed();
            let mark = gtk::Image::from_icon_name("object-select-symbolic");
            mark.add_css_class("success");
            suffix.append(&mark);
            row.set_subtitle(&format!(
                "Downloaded · {} · {}",
                human_size(done.bytes),
                if done.verified {
                    "checksum verified"
                } else {
                    "no checksum to verify against"
                }
            ));
            ctx.toast(format!("Saved to {}", done.path.display()));
        }
        Err(e) => {
            // The bytes are on disk; only the index entry failed.
            ctx.toast_error("Downloaded, but could not add it to the library", e);
        }
    }
}

/// The ABI of the machine Xylitol is running on, in Android's naming.
fn host_abi() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "x86" => "x86",
        "aarch64" => "arm64-v8a",
        "arm" => "armeabi-v7a",
        _ => "universal",
    }
}
