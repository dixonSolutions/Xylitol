//! What Xylitol's shim is, and what it can do on this machine.

use adw::prelude::*;
use xylitol_core::shim;

use crate::state::Shared;

pub fn build(_ctx: &Shared) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);

    let explain = adw::PreferencesGroup::builder()
        .title("How Xylitol runs apps")
        .description(
            "Xylitol loads an app's native code into its own process and answers the \
             calls that code makes. There is no Android runtime, emulator or container \
             involved, and no instruction translation — so an app has to ship native \
             code built for this machine's own CPU.",
        )
        .build();
    content.append(&explain);

    let capability = shim::capability();

    let group = adw::PreferencesGroup::builder()
        .title("On this machine")
        .build();
    group.add(&detail_row(
        "Loadable code",
        &capability
            .host_abi
            .clone()
            .unwrap_or_else(|| "none — this CPU is not supported".into()),
        capability.host_abi.is_some(),
    ));
    group.add(&detail_row(
        "Platform libraries",
        &format!("{} known by name", capability.known_libraries),
        true,
    ));
    group.add(&detail_row(
        "Implemented symbols",
        &format!("{}", capability.implemented_symbols),
        true,
    ));
    content.append(&group);

    let limits = adw::PreferencesGroup::builder()
        .title("What this cannot do yet")
        .description(
            "An app written in Java or Kotlin keeps its logic in DEX bytecode and calls \
             the android.* framework classes. Running one needs a bytecode interpreter \
             and those classes, neither of which Xylitol has. Apps whose logic is native \
             — NativeActivity and GameActivity games — are the ones this approach reaches \
             first. Check any package from the Library tab to see where it falls.",
        )
        .build();
    content.append(&limits);

    let clamp = adw::Clamp::builder()
        .maximum_size(720)
        .margin_top(18)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .child(&content)
        .build();

    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build()
        .upcast()
}

fn detail_row(title: &str, value: &str, good: bool) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(gtk::glib::markup_escape_text(value))
        .build();
    let icon = gtk::Image::from_icon_name(if good {
        "emblem-ok-symbolic"
    } else {
        "dialog-warning-symbolic"
    });
    icon.add_css_class(if good { "success" } else { "warning" });
    row.add_prefix(&icon);
    row
}
