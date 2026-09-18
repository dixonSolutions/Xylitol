//! Bridging between GTK's main loop and Tokio.
//!
//! GTK widgets are not `Send`, and Tokio's work is not on the main loop, so the
//! two never touch: futures run on a background runtime and hand their result
//! back over a channel that the main loop awaits.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Runtime;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Run `future` off the main loop and call `on_done` back on it.
pub fn spawn<F, T>(future: F, on_done: impl FnOnce(T) + 'static)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    runtime().spawn(async move {
        let _ = tx.send(future.await).await;
    });
    gtk::glib::spawn_future_local(async move {
        if let Ok(value) = rx.recv().await {
            on_done(value);
        }
    });
}

/// Run `future` off the main loop while it reports progress.
///
/// `future` is given a sender for progress updates; each one is delivered to
/// `on_update` on the main loop, and the final value goes to `on_done`.
pub fn spawn_with_progress<F, Fut, P, T>(
    make_future: F,
    mut on_update: impl FnMut(P) + 'static,
    on_done: impl FnOnce(T) + 'static,
) where
    F: FnOnce(async_channel::Sender<P>) -> Fut,
    Fut: Future<Output = T> + Send + 'static,
    P: Send + 'static,
    T: Send + 'static,
{
    // Bounded so a fast producer cannot outrun the UI without limit; progress
    // updates are cheap to drop, and `try_send` below does exactly that.
    let (progress_tx, progress_rx) = async_channel::bounded(16);
    let (done_tx, done_rx) = async_channel::bounded(1);

    let future = make_future(progress_tx);
    runtime().spawn(async move {
        let _ = done_tx.send(future.await).await;
    });

    gtk::glib::spawn_future_local(async move {
        while let Ok(update) = progress_rx.recv().await {
            on_update(update);
        }
    });
    gtk::glib::spawn_future_local(async move {
        if let Ok(value) = done_rx.recv().await {
            on_done(value);
        }
    });
}
