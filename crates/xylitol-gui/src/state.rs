//! State shared by every page of the window.

use std::cell::RefCell;
use std::rc::Rc;

use xylitol_core::apkpure::Client;
use xylitol_core::library::Library;

pub struct Ctx {
    pub client: Client,
    pub toasts: adw::ToastOverlay,
    pub nav: adw::NavigationView,
    pub library: RefCell<Library>,
    /// Called after anything changes the library, so open pages can refresh.
    library_observers: RefCell<Vec<Rc<dyn Fn()>>>,
}

pub type Shared = Rc<Ctx>;

impl Ctx {
    pub fn new(
        client: Client,
        toasts: adw::ToastOverlay,
        nav: adw::NavigationView,
        library: Library,
    ) -> Shared {
        Rc::new(Ctx {
            client,
            toasts,
            nav,
            library: RefCell::new(library),
            library_observers: RefCell::new(Vec::new()),
        })
    }

    pub fn toast(&self, message: impl AsRef<str>) {
        self.toasts.add_toast(adw::Toast::new(message.as_ref()));
    }

    /// Show an error in a toast that stays until dismissed.
    pub fn toast_error(&self, context: &str, error: impl std::fmt::Display) {
        let toast = adw::Toast::builder()
            .title(format!("{context}: {error}"))
            .timeout(0)
            .build();
        self.toasts.add_toast(toast);
    }

    pub fn on_library_changed(&self, observer: impl Fn() + 'static) {
        self.library_observers.borrow_mut().push(Rc::new(observer));
    }

    pub fn library_changed(&self) {
        // Take a copy of the list first: an observer is free to register another
        // one, and holding the borrow across the calls would panic.
        let observers: Vec<Rc<dyn Fn()>> = self.library_observers.borrow().clone();
        for observer in observers {
            observer();
        }
    }
}
