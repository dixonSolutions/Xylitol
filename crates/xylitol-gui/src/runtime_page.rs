//! Which Android runtime Xylitol can hand packages to.

use adw::prelude::*;
use xylitol_core::runtime;

use crate::state::Shared;
use crate::tasks;

pub fn build(ctx: &Shared) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);

    let explain = adw::PreferencesGroup::builder()
        .title("Running the apps")
        .description(
            "Xylitol downloads and inspects packages itself, and installs them through an \
             Android runtime that is already on this system. Waydroid runs Android in a \
             container on the Wayland session; adb covers a connected device or emulator.",
        )
        .build();
    content.append(&explain);

    // A holder, because a PreferencesGroup wraps its rows in widgets of its own:
    // the group is rebuilt wholesale rather than having its children removed.
    let status_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&status_holder);

    let refresh_button = gtk::Button::builder()
        .label("Check again")
        .halign(gtk::Align::Center)
        .build();
    content.append(&refresh_button);

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

    let probe = {
        let ctx = ctx.clone();
        let status_holder = status_holder.clone();
        let refresh_button = refresh_button.clone();
        move || {
            refresh_button.set_sensitive(false);
            let status_holder = status_holder.clone();
            let refresh_button = refresh_button.clone();
            let ctx = ctx.clone();
            tasks::spawn(runtime::detect(), move |statuses: Vec<runtime::Status>| {
                refresh_button.set_sensitive(true);
                while let Some(child) = status_holder.first_child() {
                    status_holder.remove(&child);
                }
                let group = adw::PreferencesGroup::builder().title("Detected").build();
                for status in &statuses {
                    group.add(&status_row(status));
                }
                status_holder.append(&group);
                if !statuses.iter().any(|s| s.ready) {
                    ctx.toast("No Android runtime is ready — downloads still work.");
                }
            });
        }
    };

    refresh_button.connect_clicked({
        let probe = probe.clone();
        move |_| probe()
    });
    probe();

    scroller.upcast()
}

fn status_row(status: &runtime::Status) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(status.backend.label())
        .subtitle(gtk::glib::markup_escape_text(&status.detail))
        .build();

    let icon_name = if status.ready {
        "emblem-ok-symbolic"
    } else if status.installed {
        "dialog-warning-symbolic"
    } else {
        "window-close-symbolic"
    };
    let icon = gtk::Image::from_icon_name(icon_name);
    if status.ready {
        icon.add_css_class("success");
    } else if status.installed {
        icon.add_css_class("warning");
    } else {
        icon.add_css_class("dim-label");
    }
    row.add_prefix(&icon);
    row
}
