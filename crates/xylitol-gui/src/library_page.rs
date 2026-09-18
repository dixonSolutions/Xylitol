//! Downloaded packages, and what can be done with them.

use adw::prelude::*;
use xylitol_core::human_size;
use xylitol_core::library::Entry;
use xylitol_core::shim;

use crate::state::Shared;
use crate::tasks;

pub fn build(ctx: &Shared) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);

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

    let refresh = {
        let ctx = ctx.clone();
        let content = content.clone();
        move || {
            while let Some(child) = content.first_child() {
                content.remove(&child);
            }

            let entries: Vec<Entry> = ctx
                .library
                .borrow()
                .entries()
                .into_iter()
                .cloned()
                .collect();

            if entries.is_empty() {
                content.append(
                    &adw::StatusPage::builder()
                        .icon_name("folder-download-symbolic")
                        .title("Nothing downloaded yet")
                        .description("Files you download from Discover appear here.")
                        .vexpand(true)
                        .build(),
                );
                return;
            }

            let group = adw::PreferencesGroup::builder()
                .title(format!("{} package(s)", entries.len()))
                .build();
            for entry in &entries {
                group.add(&row_for(&ctx, entry));
            }
            content.append(&group);
        }
    };

    refresh();
    ctx.on_library_changed(refresh);

    scroller.upcast()
}

fn row_for(ctx: &Shared, entry: &Entry) -> adw::ExpanderRow {
    let info = &entry.info;

    let row = adw::ExpanderRow::builder()
        .title(gtk::glib::markup_escape_text(info.display_name()))
        .subtitle(gtk::glib::markup_escape_text(&format!(
            "{} · {} · {}",
            info.package,
            info.version_display(),
            human_size(info.file_size)
        )))
        .build();

    row.add_row(&detail_row("Format", &format!("{:?}", info.kind)));
    if !info.abis.is_empty() {
        row.add_row(&detail_row("ABIs", &info.abis.join(", ")));
    }
    if let (Some(min), Some(target)) = (info.min_sdk, info.target_sdk) {
        row.add_row(&detail_row("SDK", &format!("min {min}, target {target}")));
    }
    row.add_row(&detail_row(
        "Permissions",
        &info.permissions.len().to_string(),
    ));
    row.add_row(&detail_row(
        "Checksum",
        if entry.verified {
            "verified against APKPure"
        } else {
            "not verified"
        },
    ));
    row.add_row(&detail_row("File", &entry.path.to_string_lossy()));

    let check = gtk::Button::builder()
        .label("Check")
        .tooltip_text("See whether Xylitol's shim can run this app")
        .valign(gtk::Align::Center)
        .build();
    check.add_css_class("suggested-action");
    check.connect_clicked({
        let ctx = ctx.clone();
        let entry = entry.clone();
        let row = row.clone();
        move |button| check_clicked(&ctx, &entry, &row, button.clone())
    });

    let remove = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .valign(gtk::Align::Center)
        .tooltip_text("Remove from library and delete the file")
        .build();
    remove.add_css_class("flat");
    remove.connect_clicked({
        let ctx = ctx.clone();
        let key = entry.key();
        let name = info.display_name().to_string();
        move |_| {
            // Let the borrow end before notifying: observers rebuild this list
            // and need to read the library themselves.
            let outcome = ctx.library.borrow_mut().remove(&key, true);
            match outcome {
                Ok(true) => ctx.toast(format!("Removed {name}")),
                Ok(false) => ctx.toast("That package was already gone"),
                Err(e) => ctx.toast_error("Could not remove it", e),
            }
            ctx.library_changed();
        }
    });

    let suffix = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    suffix.set_valign(gtk::Align::Center);
    suffix.append(&check);
    suffix.append(&remove);
    row.add_suffix(&suffix);

    row
}

fn detail_row(title: &str, value: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(title)
        .subtitle(gtk::glib::markup_escape_text(value))
        .subtitle_selectable(true)
        .build()
}

/// Ask the shim what it makes of this package, and show the answer in place.
fn check_clicked(ctx: &Shared, entry: &Entry, row: &adw::ExpanderRow, button: gtk::Button) {
    button.set_sensitive(false);
    button.set_label("Checking…");

    let entry = entry.clone();
    tasks::spawn(async move { shim::check(&entry) }, {
        let ctx = ctx.clone();
        let row = row.clone();
        move |outcome: anyhow::Result<shim::Report>| {
            button.set_sensitive(true);
            button.set_label("Check");
            match outcome {
                Ok(report) => {
                    let verdict = report.verdict.headline();
                    row.add_row(&detail_row("Shim verdict", &verdict));
                    if report.totals.total() > 0 {
                        row.add_row(&detail_row(
                            "Symbols",
                            &format!(
                                "{} total — {} shim, {} host, {} unimplemented",
                                report.totals.total(),
                                report.totals.shim,
                                report.totals.host,
                                report.totals.stub
                            ),
                        ));
                    }
                    if !report.android_libraries.is_empty() {
                        row.add_row(&detail_row(
                            "Links against",
                            &report.android_libraries.join(", "),
                        ));
                    }
                    row.set_expanded(true);
                    ctx.toast(if report.verdict.is_runnable() {
                        "This app's native code can be loaded here"
                    } else {
                        "This app cannot run under the shim yet"
                    });
                }
                Err(e) => ctx.toast_error("Could not analyse the package", e),
            }
        }
    });
}
