//! One round trip for the storage and alongside surveys: a request bumps the
//! state's generation and marks it busy; the result is applied only when no
//! newer request (or a cancel, which also bumps the generation) started since.

use crate::ui::App;
use slint::{ComponentHandle, SharedString};
use std::future::Future;

/// A Slint global with `generation`, `busy` and `error` properties.
pub trait Guarded {
    fn generation(app: &App) -> i32;
    fn set_generation(app: &App, generation: i32);
    fn set_busy(app: &App, busy: bool);
    fn set_error(app: &App, error: SharedString);
}

/// Starts a request: bumps the generation, marks the state busy and clears
/// the previous error. Returns the generation identifying this request.
pub fn begin<G: Guarded>(app: &App) -> i32 {
    let generation = G::generation(app) + 1;
    G::set_generation(app, generation);
    G::set_busy(app, true);
    G::set_error(app, "".into());
    generation
}

/// Runs `work` on the runtime and hands its result to `apply` on the UI
/// thread, unless the request is no longer current. Busy is cleared first.
pub fn spawn<G, T>(
    app: &App,
    generation: i32,
    work: impl Future<Output = T> + Send + 'static,
    apply: impl FnOnce(&App, T) + Send + 'static,
) where
    G: Guarded + 'static,
    T: Send + 'static,
{
    let weak = app.as_weak();
    tokio::spawn(async move {
        let result = work.await;
        let _ = weak.upgrade_in_event_loop(move |app| {
            if G::generation(&app) != generation {
                return;
            }
            G::set_busy(&app, false);
            apply(&app, result);
        });
    });
}
