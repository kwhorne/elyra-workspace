//! Coming back from an update that doesn't start.
//!
//! An update keeps the version it replaced beside the app (see `updates`).
//! Every launch writes a marker that is cleared once the app has run for a
//! while or quits normally. When a version fails to get that far twice in a
//! row, the previous version is put back and started, the broken one is
//! skipped until a newer release, and the user is told. The previous version
//! can also be brought back by hand (command palette).

use crate::updates;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AsyncApp};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A launch that runs this long counts as a good start.
const HEALTHY_AFTER: Duration = Duration::from_secs(20);
/// Failed starts of one version before going back to the previous one.
const FAILED_STARTS: u32 = 2;

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct LaunchState {
    /// The version whose start hasn't been confirmed yet.
    starting: Option<String>,
    /// Earlier starts of that version that never got confirmed.
    failures: u32,
    /// A version that didn't start: not offered again.
    skipped: Option<String>,
    /// Set when going back, to say so after the start: (from, to).
    rolled_back: Option<(String, String)>,
}

fn state_path() -> PathBuf {
    elyra_core::paths::data_dir().join("launch.json")
}

fn load() -> LaunchState {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(state: &LaunchState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(state)
        && let Err(err) = std::fs::write(&path, text)
    {
        log::warn!("saving {}: {err}", path.display());
    }
}

/// Note a start of `version`; true when its earlier starts failed often
/// enough to go back.
fn note_start(state: &mut LaunchState, version: &str) -> bool {
    if state.starting.as_deref() == Some(version) {
        state.failures += 1;
    } else {
        state.starting = Some(version.to_string());
        state.failures = 0;
    }
    state.failures >= FAILED_STARTS
}

/// The installed app, when running from one (not `cargo run`).
fn bundle() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| updates::bundle_of(&exe))
}

/// First thing at launch. Returns true when the previous version was put
/// back and started instead: then exit at once.
pub fn on_launch() -> bool {
    let Some(bundle) = bundle() else {
        return false;
    };
    let current = env!("CARGO_PKG_VERSION");
    let mut state = load();
    if note_start(&mut state, current)
        && let Some(previous) = updates::previous_version(&bundle).filter(|v| v != current)
    {
        log::error!("{current} failed to start {FAILED_STARTS} times; going back to {previous}");
        match go_back(&bundle, &mut state, current, &previous) {
            Ok(()) => return true,
            Err(err) => log::error!("going back to {previous}: {err:#}"),
        }
    }
    save(&state);
    false
}

/// Put the previous version back and start it once this process exits.
fn go_back(bundle: &Path, state: &mut LaunchState, from: &str, to: &str) -> anyhow::Result<()> {
    updates::roll_back(bundle)?;
    state.skipped = Some(from.to_string());
    state.rolled_back = Some((from.to_string(), to.to_string()));
    state.starting = None;
    state.failures = 0;
    save(state);
    updates::relaunch_after_exit(bundle)?;
    Ok(())
}

/// The start went well (it ran a while, or quit normally).
pub fn confirm_start() {
    let mut state = load();
    if state.starting.is_some() || state.failures > 0 {
        state.starting = None;
        state.failures = 0;
        save(&state);
    }
}

/// Confirm the start once the app has run for a while.
pub fn confirm_start_later(cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(HEALTHY_AFTER).await;
        confirm_start();
    })
    .detach();
}

/// A version not to offer as an update (it didn't start here).
pub fn is_skipped(version: &str) -> bool {
    load().skipped.as_deref() == Some(version)
}

/// After going back: tell the user, once.
pub fn announce(cx: &mut App) {
    let mut state = load();
    let Some((from, to)) = state.rolled_back.take() else {
        return;
    };
    save(&state);
    let Some(window) = cx
        .try_global::<crate::browser_tools::BrowserHub>()
        .map(|hub| hub.window)
    else {
        return;
    };
    let note = Notification::warning(format!(
        "Elyra Workspace {from} didn't start, so you're back on {to}. {from} won't be offered again; the next release will."
    ))
    .autohide(false);
    let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
}

/// Command palette: go back to the version before the last update.
pub fn offer_go_back(cx: &mut App) {
    let window = cx.active_window().or_else(|| cx.windows().first().copied());
    let Some(window) = window else {
        return;
    };
    let current = env!("CARGO_PKG_VERSION");
    let previous = bundle().and_then(|bundle| updates::previous_version(&bundle));
    let _ = window.update(cx, move |_, window, cx| {
        let Some(previous) = previous.filter(|v| v != current) else {
            window.push_notification(
                Notification::info(
                    "There is no earlier version to go back to: one is kept after each update.",
                ),
                cx,
            );
            return;
        };
        window.open_alert_dialog(cx, move |alert, _, _| {
            let previous = previous.clone();
            alert
                .title(format!("Go back to Elyra Workspace {previous}?"))
                .description(format!(
                    "Elyra Workspace restarts as {previous}; running agents stop and can be resumed. {current} won't be offered again; the next release will."
                ))
                .confirm()
                .ok_text("Go back")
                .on_ok(move |_, _, cx| {
                    let Some(bundle) = bundle() else {
                        return true;
                    };
                    // A staged update must not be installed over it on quit.
                    crate::updater::discard_staged(cx);
                    let mut state = load();
                    match go_back(&bundle, &mut state, current, &previous) {
                        Ok(()) => crate::quitting::quit_now(cx),
                        Err(err) => log::error!("going back to {previous}: {err:#}"),
                    }
                    true
                })
        });
    });
}

#[cfg(test)]
mod tests {
    use super::{FAILED_STARTS, LaunchState, note_start};

    #[test]
    fn goes_back_after_repeated_failed_starts() {
        let mut state = LaunchState::default();
        assert!(!note_start(&mut state, "1.1.0"), "first start");
        // It never got confirmed, and starts again.
        for _ in 1..FAILED_STARTS {
            assert!(!note_start(&mut state, "1.1.0"));
        }
        assert!(note_start(&mut state, "1.1.0"), "failed twice: go back");

        // A confirmed start clears it (as confirm_start does).
        let mut state = LaunchState::default();
        note_start(&mut state, "1.1.0");
        state.starting = None;
        state.failures = 0;
        assert!(!note_start(&mut state, "1.1.0"));
        assert_eq!(state.failures, 0);

        // A different version starts counting afresh.
        let mut state = LaunchState {
            starting: Some("1.0.0".into()),
            failures: 5,
            ..LaunchState::default()
        };
        assert!(!note_start(&mut state, "1.1.0"));
        assert_eq!(state.failures, 0);
    }
}
