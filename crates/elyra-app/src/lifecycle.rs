//! Process lifecycle: one instance per data directory, crash logs.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;

/// Hold an exclusive lock on the data directory so two instances never
/// write the same database. Returns `None` when another instance runs.
pub fn acquire_instance_lock() -> Option<File> {
    let dir = elyra_core::paths::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("instance.lock"))
        .ok()?;
    file.try_lock().ok()?;
    Some(file)
}

pub fn crash_log_path() -> PathBuf {
    elyra_core::paths::data_dir().join("logs").join("crash.log")
}

/// Append panics (with backtrace) to the crash log, then run the default hook.
pub fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let path = crash_log_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(
                file,
                "=== {} · v{} ===\n{info}\n{}\n",
                chrono::Local::now().to_rfc3339(),
                env!("CARGO_PKG_VERSION"),
                std::backtrace::Backtrace::force_capture()
            );
        }
        default(info);
    }));
}

/// The crash log's modification time, as seconds since the epoch.
pub fn last_crash_time() -> Option<u64> {
    let modified = std::fs::metadata(crash_log_path()).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs(),
    )
}
