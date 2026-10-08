//! Quitting while agents work. ⌘Q offers to stop them now, or to hide the
//! window and quit by itself once they have finished. Closing the window keeps
//! them working too; the Dock icon brings it back.

use crate::app_state::AppState;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, Global, Window, div, px};
use std::time::Duration;

/// How often to look whether the agents have finished, while waiting to quit.
const IDLE_CHECK: Duration = Duration::from_secs(2);

struct Quitting {
    app: Entity<AppState>,
    /// Quit as soon as no agent is working.
    when_idle: bool,
}

impl Global for Quitting {}

pub fn init(app: Entity<AppState>, cx: &mut App) {
    cx.set_global(Quitting {
        app,
        when_idle: false,
    });
}

/// Waiting to quit or restart: don't start new work (Félagi tasks,
/// automations), or the wait might never end.
pub fn draining(cx: &App) -> bool {
    draining_to_quit(cx) || crate::updater::waiting(cx)
}

fn draining_to_quit(cx: &App) -> bool {
    cx.try_global::<Quitting>().is_some_and(|q| q.when_idle)
}

/// Quit, asking first when agents are still working.
pub fn request(cx: &mut App) {
    let app = cx.global::<Quitting>().app.clone();
    let running = app.read(cx).running_threads(cx);
    if !running.is_empty() && cx.windows().is_empty() {
        // The window was closed: bring it back to ask.
        reopen(cx);
    }
    let window = cx.active_window().or_else(|| cx.windows().first().copied());
    let (false, Some(window)) = (running.is_empty(), window) else {
        quit_now(cx);
        return;
    };
    let names = running
        .iter()
        .take(5)
        .map(|t| format!("• {}", t.title))
        .collect::<Vec<_>>()
        .join("\n");
    let count = running.len();
    let _ = window.update(cx, move |_, window, cx| {
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(if count == 1 {
                    "An agent is still working".to_string()
                } else {
                    format!("{count} agents are still working")
                })
                .w(px(480.))
                .child(div().text_sm().whitespace_normal().child(format!(
                    "{names}\n\nQuit when they finish: the window closes and Elyra Workspace quits by itself. Quitting now stops them; you can resume each thread after the next launch."
                )))
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("quit-cancel")
                                .small()
                                .label("Cancel")
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                    crate::updater::cancel_relaunch(cx);
                                }),
                        )
                        .child(
                            Button::new("quit-now")
                                .small()
                                .danger()
                                .label("Quit now")
                                .on_click(|_, window: &mut Window, cx| {
                                    window.close_dialog(cx);
                                    quit_now(cx);
                                }),
                        )
                        .child(
                            Button::new("quit-when-idle")
                                .small()
                                .primary()
                                .label("Quit when they finish")
                                .on_click(|_, window: &mut Window, cx| {
                                    window.close_dialog(cx);
                                    quit_when_idle(cx);
                                }),
                        ),
                )
        });
    });
}

/// Stop the agents (their turns can be resumed after the next launch) and quit.
pub(crate) fn quit_now(cx: &mut App) {
    let app = cx.global::<Quitting>().app.clone();
    app.update(cx, |app, cx| app.prepare_quit(cx));
    cx.quit();
}

/// Hide Elyra Workspace and quit once no agent is working. Coming back to
/// the window calls it off.
fn quit_when_idle(cx: &mut App) {
    if cx.global::<Quitting>().when_idle {
        return;
    }
    cx.global_mut::<Quitting>().when_idle = true;
    cx.hide();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(IDLE_CHECK).await;
            let done = cx.update(|cx| {
                let quitting = cx.global::<Quitting>();
                if !quitting.when_idle {
                    return true;
                }
                let idle = quitting.app.read(cx).running_threads(cx).is_empty();
                if idle {
                    quit_now(cx);
                }
                idle
            });
            if done {
                break;
            }
        }
    })
    .detach();
}

/// The window became active again: the user is back, so stay open.
pub fn came_back(window: &mut Window, cx: &mut App) {
    if !draining_to_quit(cx) {
        return;
    }
    cx.global_mut::<Quitting>().when_idle = false;
    window.push_notification(
        Notification::info("Elyra Workspace stays open: you came back before the agents finished."),
        cx,
    );
}

/// The Dock icon was clicked (or the app opened again): show the window,
/// opening a new one if it was closed.
pub fn reopen(cx: &mut App) {
    let shown = cx
        .try_global::<crate::browser_tools::BrowserHub>()
        .map(|hub| hub.window)
        .is_some_and(|window| {
            window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        });
    if !shown && cx.has_global::<Quitting>() {
        // A new window may not report becoming active: call the quit off here.
        let quitting = cx.global_mut::<Quitting>();
        quitting.when_idle = false;
        let app = quitting.app.clone();
        crate::open_main_window(app, true, cx);
    }
    cx.activate(true);
}

/// At launch: offer to continue every turn that was cut off when the app
/// quit (or crashed), instead of one thread at a time.
pub fn offer_resume(cx: &mut App) {
    let app = cx.global::<Quitting>().app.clone();
    let interrupted: Vec<elyra_core::ThreadId> = app
        .read(cx)
        .threads
        .iter()
        .filter(|t| t.status == elyra_core::ThreadStatus::Interrupted && !t.archived)
        .map(|t| t.id)
        .collect();
    let Some(window) = cx
        .try_global::<crate::browser_tools::BrowserHub>()
        .map(|hub| hub.window)
    else {
        return;
    };
    let message = match interrupted.len() {
        0 => return,
        1 => "A turn was cut off when Elyra Workspace quit. Click to continue it.".to_string(),
        n => format!("{n} turns were cut off when Elyra Workspace quit. Click to continue them."),
    };
    let note = Notification::warning(message)
        .autohide(false)
        .on_click(move |_, _, cx| {
            for id in &interrupted {
                let session = app.update(cx, |app, cx| app.session(*id, cx));
                if let Some(session) = session {
                    session.update(cx, |session, cx| session.resume_interrupted(cx));
                }
            }
        });
    let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
}
