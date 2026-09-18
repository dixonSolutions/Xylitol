//! The application window: a view switcher over the three things Xylitol does.

use adw::prelude::*;
use xylitol_core::apkpure::Client;
use xylitol_core::library::Library;

use crate::state::Ctx;
use crate::{discover, library_page, runtime_page};

pub fn build(app: &adw::Application) -> adw::ApplicationWindow {
    let nav = adw::NavigationView::new();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&nav));

    // A library that cannot be read is not fatal: the rest of the app still
    // works, so fall back to an empty one and say so.
    let (library, library_error) = match Library::open() {
        Ok(library) => (library, None),
        Err(e) => (
            Library::open_at(std::env::temp_dir().join("xylitol-session-library.json"))
                .unwrap_or_else(|_| unreachable!("a temp-file library cannot fail to open")),
            Some(e.to_string()),
        ),
    };

    let ctx = Ctx::new(Client::new(), toasts.clone(), nav.clone(), library);

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(
        &discover::build(&ctx),
        Some("discover"),
        "Discover",
        "system-search-symbolic",
    );
    stack.add_titled_with_icon(
        &library_page::build(&ctx),
        Some("library"),
        "Library",
        "folder-download-symbolic",
    );
    stack.add_titled_with_icon(
        &runtime_page::build(&ctx),
        Some("runtime"),
        "Runtime",
        "phone-symbolic",
    );

    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    let switcher_bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    toolbar.add_bottom_bar(&switcher_bar);

    let page = adw::NavigationPage::builder()
        .title("Xylitol")
        .tag("main")
        .child(&toolbar)
        .build();
    nav.add(&page);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Xylitol")
        .default_width(900)
        .default_height(680)
        .content(&toasts)
        .build();

    // Narrow windows move the switcher from the header to the bottom bar.
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        560.0,
        adw::LengthUnit::Sp,
    ));
    breakpoint.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
    // An explicit "no widget" value: `add_setter` requires a real GValue, and a
    // plain `None` is a null pointer rather than a typed empty one.
    breakpoint.add_setter(
        &header,
        "title-widget",
        Some(&None::<gtk::Widget>.to_value()),
    );
    window.add_breakpoint(breakpoint);

    if let Some(error) = library_error {
        ctx.toast_error("Could not open the library", error);
    }

    window
}
