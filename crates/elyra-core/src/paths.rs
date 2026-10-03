use std::path::PathBuf;

/// Root directory for Elyra's durable state. `ELYRA_HOME` overrides the
/// default `~/.elyra`, which keeps isolated dev/test instances apart from
/// the user's real data.
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("ELYRA_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".elyra")
}

pub fn database_path() -> PathBuf {
    data_dir().join("state.db")
}

/// Pictures of the browser page taken around turns, one folder per thread.
pub fn snapshots_dir() -> PathBuf {
    data_dir().join("snapshots")
}

/// Managed worktrees live outside the project checkout so parallel threads
/// never write into the user's working copy.
pub fn worktrees_dir() -> PathBuf {
    data_dir().join("worktrees")
}
