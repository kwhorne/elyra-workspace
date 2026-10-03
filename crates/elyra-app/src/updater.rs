//! Automatic updates: check GitHub at launch and every few hours, download
//! and verify a newer release in the background, then install it when the
//! user restarts (or the next time they quit). See `updates` for the steps.

use crate::app_state::AppState;
use crate::preferences::Preferences;
use crate::updates::{self, Release};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AsyncApp, Entity, Global};
use std::path::PathBuf;
use std::time::Duration;

const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Idle,
    Busy,
    Ready {
        version: String,
        staged: PathBuf,
        bundle: PathBuf,
    },
}

struct Updater {
    app: Entity<AppState>,
    status: Status,
    /// Start the new version after quitting.
    relaunch: bool,
}

impl Global for Updater {}

pub fn init(app: Entity<AppState>, cx: &mut App) {
    cx.set_global(Updater {
        app,
        status: Status::Idle,
        relaunch: false,
    });
    cx.spawn(async move |cx| {
        loop {
            let enabled = cx.update(|cx| Preferences::global(cx).check_updates);
            if enabled {
                cx.update(|cx| check(false, cx));
            }
            cx.background_executor().timer(CHECK_INTERVAL).await;
        }
    })
    .detach();
}

fn notify(note: Notification, cx: &mut App) {
    if let Some(window) = cx.windows().first().copied() {
        let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
    }
}

/// Look for a newer release. `manual` also reports "up to date" and errors.
pub fn check(manual: bool, cx: &mut App) {
    let status = cx.global::<Updater>().status.clone();
    match status {
        Status::Busy => {
            if manual {
                notify(
                    Notification::info("Already looking for or downloading an update."),
                    cx,
                );
            }
            return;
        }
        Status::Ready { version, .. } => {
            if manual {
                notify(ready_note(&version), cx);
            }
            return;
        }
        Status::Idle => {}
    }
    cx.global_mut::<Updater>().status = Status::Busy;
    let job = cx
        .background_executor()
        .spawn(async { updates::latest_release() });
    cx.spawn(async move |cx| {
        let result = job.await;
        cx.update(|cx| {
            cx.global_mut::<Updater>().status = Status::Idle;
            let current = env!("CARGO_PKG_VERSION");
            match result {
                Ok(release) if updates::is_newer(&release.version, current) => {
                    found(release, manual, cx)
                }
                Ok(_) if manual => notify(
                    Notification::success(format!("Elyra Workspace {current} is up to date.")),
                    cx,
                ),
                Err(err) if manual => notify(
                    Notification::warning(format!("Could not check for updates: {err:#}")),
                    cx,
                ),
                _ => {}
            }
        });
    })
    .detach();
}

/// A newer release exists: install it automatically when we can, otherwise
/// point to the download.
fn found(release: Release, manual: bool, cx: &mut App) {
    let bundle = std::env::current_exe()
        .ok()
        .and_then(|exe| updates::bundle_of(&exe));
    let blocker = updates::self_update_blocker(bundle.as_deref());
    let current = env!("CARGO_PKG_VERSION");
    if let Some(reason) = blocker.or(release
        .dmg
        .is_none()
        .then_some("the release has no installer"))
    {
        let url = release.url.clone();
        notify(
            Notification::info(format!(
                "Elyra Workspace {} is available (you have {current}). It can't update itself because {reason}. Click to download it.",
                release.version
            ))
            .autohide(false)
            .on_click(move |_, _, cx| cx.open_url(&url)),
            cx,
        );
        return;
    }
    let bundle = bundle.expect("checked above");
    if Preferences::global(cx).auto_update || manual {
        download(release, bundle, manual, cx);
    } else {
        let version = release.version.clone();
        notify(
            Notification::info(format!(
                "Elyra Workspace {version} is available (you have {current}). Click to download and install it."
            ))
            .autohide(false)
            .on_click(move |_, _, cx| download(release.clone(), bundle.clone(), true, cx)),
            cx,
        );
    }
}

fn download(release: Release, bundle: PathBuf, manual: bool, cx: &mut App) {
    if cx.global::<Updater>().status != Status::Idle {
        return;
    }
    cx.global_mut::<Updater>().status = Status::Busy;
    if manual {
        notify(
            Notification::info(format!("Downloading Elyra Workspace {}…", release.version)),
            cx,
        );
    }
    let work = elyra_core::paths::data_dir()
        .join("updates")
        .join(&release.version);
    let job = cx.background_executor().spawn(async move {
        // Only accept updates signed by the same team as this copy.
        let team = updates::team_id(&bundle)
            .ok_or_else(|| anyhow::anyhow!("this copy is not signed with a Developer ID"))?;
        let staged = updates::prepare(&release, &bundle, &team, &work);
        let _ = std::fs::remove_dir_all(&work);
        staged.map(|staged| (release.version, staged, bundle))
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = job.await;
        cx.update(|cx| match result {
            Ok((version, staged, bundle)) => {
                cx.global_mut::<Updater>().status = Status::Ready {
                    version: version.clone(),
                    staged,
                    bundle,
                };
                notify(ready_note(&version), cx);
                // Development: exercise the whole cycle without input.
                if std::env::var_os("ELYRA_UPDATE_RESTART_WHEN_READY").is_some() {
                    restart(cx);
                }
            }
            Err(err) => {
                cx.global_mut::<Updater>().status = Status::Idle;
                log::warn!("update failed: {err:#}");
                if manual {
                    notify(
                        Notification::error(format!("The update could not be installed: {err:#}")),
                        cx,
                    );
                }
            }
        });
    })
    .detach();
}

fn ready_note(version: &str) -> Notification {
    Notification::success(format!(
        "Elyra Workspace {version} is ready. Click to restart and update — or it installs the next time you quit."
    ))
    .autohide(false)
    .on_click(|_, _, cx| restart(cx))
}

/// Quit (asking first if agents are working), install, and start again.
pub fn restart(cx: &mut App) {
    if !matches!(cx.global::<Updater>().status, Status::Ready { .. }) {
        return;
    }
    cx.global_mut::<Updater>().relaunch = true;
    let app = cx.global::<Updater>().app.clone();
    crate::request_quit(&app, cx);
}

/// Called while the app quits: swap in a staged update and relaunch if asked.
pub fn on_quit(cx: &mut App) {
    let Some(updater) = cx.try_global::<Updater>() else {
        return;
    };
    let Status::Ready { staged, bundle, .. } = updater.status.clone() else {
        return;
    };
    let relaunch = updater.relaunch;
    match updates::install(&staged, &bundle) {
        Ok(()) => {
            if relaunch && let Err(err) = updates::relaunch_after_exit(&bundle) {
                log::error!("relaunch: {err:#}");
            }
        }
        Err(err) => log::error!("installing the update: {err:#}"),
    }
}

/// A quit that was cancelled (e.g. at the "agents are working" prompt)
/// shouldn't relaunch later.
pub fn cancel_relaunch(cx: &mut App) {
    if cx.has_global::<Updater>() {
        cx.global_mut::<Updater>().relaunch = false;
    }
}
