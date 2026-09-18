//! The application window: a view switcher over the three things Xylitol does.

use adw::prelude::*;
use xylitol_core::apkpure::Client;
use xylitol_core::library::Library;
use xylitol_core::paths::APP_ID;

use crate::state::{Ctx, Shared};
use crate::{discover, library_page, shim_page};

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
        &shim_page::build(&ctx),
        Some("shim"),
        "Shim",
        "cpu-symbolic",
    );

    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    header.pack_end(&primary_menu_button());

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

    install_actions(app, &window, &ctx);

    if let Some(error) = library_error {
        ctx.toast_error("Could not open the library", error);
    }

    window
}

fn primary_menu_button() -> gtk::MenuButton {
    let menu = gtk::gio::Menu::new();
    menu.append(Some("Open Download Folder"), Some("app.open-downloads"));
    menu.append(Some("About Xylitol"), Some("app.about"));

    gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Main Menu")
        .menu_model(&menu)
        .primary(true)
        .build()
}

fn install_actions(app: &adw::Application, window: &adw::ApplicationWindow, ctx: &Shared) {
    let about = gtk::gio::SimpleAction::new("about", None);
    about.connect_activate({
        let window = window.clone();
        move |_, _| show_about(&window)
    });
    app.add_action(&about);

    let open_downloads = gtk::gio::SimpleAction::new("open-downloads", None);
    open_downloads.connect_activate({
        let ctx = ctx.clone();
        let window = window.clone();
        move |_, _| {
            let dir = xylitol_core::paths::download_dir();
            // The folder only exists once something has been downloaded.
            if let Err(e) = std::fs::create_dir_all(&dir) {
                ctx.toast_error("Could not open the download folder", e);
                return;
            }
            let launcher = gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(&dir)));
            launcher.launch(Some(&window), gtk::gio::Cancellable::NONE, {
                let ctx = ctx.clone();
                move |result| {
                    if let Err(e) = result {
                        ctx.toast_error("Could not open the download folder", e);
                    }
                }
            });
        }
    });
    app.add_action(&open_downloads);

    let quit = gtk::gio::SimpleAction::new("quit", None);
    quit.connect_activate({
        let app = app.clone();
        move |_, _| app.quit()
    });
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
}

fn show_about(window: &adw::ApplicationWindow) {
    let about = adw::AboutWindow::builder()
        .transient_for(window)
        .application_name("Xylitol")
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("The Xylitol contributors")
        .license_type(gtk::License::Gpl30)
        .website("https://github.com/dixonSolutions/Xylitol")
        .issue_url("https://github.com/dixonSolutions/Xylitol/issues")
        .comments(
            "Find, download and run Android packages.\n\n\
             Xylitol is a restart of Shashlik, which set out in 2014 to run \
             Android apps on the Linux desktop. It finds, downloads, verifies \
             and inspects packages, then runs an app's native code in its own \
             process — no Android runtime, emulator or container.\n\n\
             The shim approach is taken from Cordial.",
        )
        .build();
    about.add_credit_section(
        Some("Based on work by"),
        &[
            "Dan Leinir Turthra Jensen",
            "Inge Wallin",
            "The Shashlik contributors",
        ],
    );
    about.present();
}
