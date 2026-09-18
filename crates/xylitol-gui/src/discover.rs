//! Searching APKPure.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use xylitol_core::apkpure::SearchHit;
use xylitol_core::human_size;

use crate::state::Shared;
use crate::{icons, tasks, variants};

/// Wait this long after the last keystroke before searching, so that typing a
/// name does not fire a request per character.
const DEBOUNCE: Duration = Duration::from_millis(350);

pub fn build(ctx: &Shared) -> gtk::Widget {
    let entry = gtk::SearchEntry::builder()
        .placeholder_text("Search by app name or package name")
        .hexpand(true)
        .build();

    let results = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    results.add_css_class("boxed-list");
    results.set_visible(false);

    let status = adw::StatusPage::builder()
        .icon_name("system-search-symbolic")
        .title("Find an app")
        .description("Search APKPure, then pick which build to download.")
        .vexpand(true)
        .build();

    let spinner = gtk::Spinner::builder()
        .halign(gtk::Align::Center)
        .margin_top(24)
        .build();
    spinner.set_visible(false);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&entry);
    content.append(&spinner);
    content.append(&results);
    content.append(&status);

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

    // Each keystroke invalidates the previous pending search and any request
    // still in flight, so a slow reply cannot overwrite a newer one.
    let generation = Rc::new(Cell::new(0u64));

    entry.connect_search_changed({
        let ctx = ctx.clone();
        let generation = generation.clone();
        let results = results.clone();
        let status = status.clone();
        let spinner = spinner.clone();
        move |entry| {
            let query = entry.text().trim().to_string();
            let current = generation.get().wrapping_add(1);
            generation.set(current);

            if query.is_empty() {
                results.set_visible(false);
                spinner.set_visible(false);
                status.set_visible(true);
                status.set_title("Find an app");
                status.set_description(Some("Search APKPure, then pick which build to download."));
                return;
            }

            let ctx = ctx.clone();
            let generation = generation.clone();
            let results = results.clone();
            let status = status.clone();
            let spinner = spinner.clone();
            gtk::glib::timeout_add_local_once(DEBOUNCE, move || {
                if generation.get() != current {
                    return;
                }
                spinner.set_visible(true);
                spinner.start();

                let client = ctx.client.clone();
                let query_for_request = query.clone();
                tasks::spawn(
                    async move { client.search(&query_for_request, 20).await },
                    move |outcome| {
                        if generation.get() != current {
                            return;
                        }
                        spinner.stop();
                        spinner.set_visible(false);
                        match outcome {
                            Ok(hits) => show(&ctx, &results, &status, &query, hits),
                            Err(e) => {
                                results.set_visible(false);
                                status.set_visible(true);
                                status.set_title("Search failed");
                                status.set_description(Some(&e.to_string()));
                            }
                        }
                    },
                );
            });
        }
    });

    scroller.upcast()
}

fn show(
    ctx: &Shared,
    results: &gtk::ListBox,
    status: &adw::StatusPage,
    query: &str,
    hits: Vec<SearchHit>,
) {
    while let Some(child) = results.first_child() {
        results.remove(&child);
    }

    if hits.is_empty() {
        results.set_visible(false);
        status.set_visible(true);
        status.set_title("No results");
        status.set_description(Some(&format!("APKPure has no app matching “{query}”.")));
        return;
    }

    status.set_visible(false);
    results.set_visible(true);

    for hit in hits {
        results.append(&row_for(ctx, &hit));
    }
}

fn row_for(ctx: &Shared, hit: &SearchHit) -> adw::ActionRow {
    let mut subtitle_parts = vec![hit.package.clone()];
    if let Some(version) = &hit.latest_version {
        subtitle_parts.push(version.clone());
    }
    if let Some(size) = hit.latest_size {
        subtitle_parts.push(human_size(size));
    }
    if let Some(installs) = &hit.installs {
        subtitle_parts.push(format!("{installs} installs"));
    }

    let row = adw::ActionRow::builder()
        .title(gtk::glib::markup_escape_text(&hit.title))
        .subtitle(gtk::glib::markup_escape_text(&subtitle_parts.join(" · ")))
        .activatable(true)
        .build();

    let icon = icons::placeholder(40);
    if let Some(url) = &hit.icon_url {
        icons::load_into(&icon, url);
    }
    row.add_prefix(&icon);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    row.connect_activated({
        let ctx = ctx.clone();
        let package = hit.package.clone();
        let title = hit.title.clone();
        move |_| variants::push(&ctx, &package, &title)
    });

    row
}
