//! Xylitol — find, download and install Android packages on the desktop.

mod discover;
mod icons;
mod library_page;
mod shim_page;
mod state;
mod tasks;
mod variants;
mod window;

use adw::prelude::*;
use xylitol_core::paths::APP_ID;

fn main() -> gtk::glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("XYLITOL_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        window::build(app).present();
    });
    app.run()
}
