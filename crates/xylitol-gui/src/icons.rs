//! Loading app icons from APKPure's image CDN.

use adw::prelude::*;
use gtk::gdk;

use crate::tasks;

/// Fetch `url` and show it in `image` once it arrives.
///
/// A failure is silent on purpose: the placeholder icon already in the widget is
/// a perfectly good outcome, and a toast per missing icon would be noise.
pub fn load_into(image: &gtk::Image, url: &str) {
    let url = url.to_string();
    let image = image.clone();
    tasks::spawn(
        async move {
            let response = reqwest::get(&url).await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let bytes = response.bytes().await.ok()?;
            Some(bytes.to_vec())
        },
        move |bytes: Option<Vec<u8>>| {
            let Some(bytes) = bytes else { return };
            let bytes = gtk::glib::Bytes::from_owned(bytes);
            if let Ok(texture) = gdk::Texture::from_bytes(&bytes) {
                image.set_paintable(Some(&texture));
            }
        },
    );
}

/// A square placeholder that an icon can be loaded into later.
pub fn placeholder(size: i32) -> gtk::Image {
    let image = gtk::Image::from_icon_name("application-x-executable-symbolic");
    image.set_pixel_size(size);
    image.add_css_class("icon-dropshadow");
    image
}
