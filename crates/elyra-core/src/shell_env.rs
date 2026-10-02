//! GUI apps launched from Finder/Dock do not inherit the login shell's
//! environment, so CLIs such as `claude` or `git` installed via npm/Homebrew
//! are missing from PATH. Load PATH from the user's login shell once at startup.

use std::process::Command;

pub fn inherit_login_shell_path() {
    if cfg!(windows) {
        return;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let output = Command::new(&shell)
        .args(["-l", "-i", "-c", "printf '__ELYRA_PATH__%s' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(output) = output else {
        log::warn!("could not run login shell {shell} to resolve PATH");
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(path) = stdout
        .rsplit("__ELYRA_PATH__")
        .next()
        .filter(|p| !p.is_empty())
    else {
        return;
    };
    let path = path.trim();
    if !path.is_empty() {
        // SAFETY: called once on the main thread before any other threads start.
        unsafe { std::env::set_var("PATH", path) };
    }
}
