//! External editors and terminals the user can open a folder or file in.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorApp {
    pub name: &'static str,
    /// macOS application name for `open -a`.
    pub app: &'static str,
    /// Command-line launcher that can jump to a line (`code -g file:line`).
    pub cli: Option<(&'static str, LineStyle)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineStyle {
    /// `cli -g file:line`
    Goto,
    /// `cli file:line`
    Suffix,
}

pub const EDITORS: &[EditorApp] = &[
    EditorApp {
        name: "VS Code",
        app: "Visual Studio Code",
        cli: Some(("code", LineStyle::Goto)),
    },
    EditorApp {
        name: "Cursor",
        app: "Cursor",
        cli: Some(("cursor", LineStyle::Goto)),
    },
    EditorApp {
        name: "Windsurf",
        app: "Windsurf",
        cli: Some(("windsurf", LineStyle::Goto)),
    },
    EditorApp {
        name: "Zed",
        app: "Zed",
        cli: Some(("zed", LineStyle::Suffix)),
    },
    EditorApp {
        name: "Sublime Text",
        app: "Sublime Text",
        cli: Some(("subl", LineStyle::Suffix)),
    },
    EditorApp {
        name: "Nova",
        app: "Nova",
        cli: None,
    },
    EditorApp {
        name: "Xcode",
        app: "Xcode",
        cli: None,
    },
    EditorApp {
        name: "IntelliJ IDEA",
        app: "IntelliJ IDEA",
        cli: None,
    },
    EditorApp {
        name: "PhpStorm",
        app: "PhpStorm",
        cli: None,
    },
    EditorApp {
        name: "WebStorm",
        app: "WebStorm",
        cli: None,
    },
    EditorApp {
        name: "PyCharm",
        app: "PyCharm",
        cli: None,
    },
    EditorApp {
        name: "RustRover",
        app: "RustRover",
        cli: None,
    },
    EditorApp {
        name: "GoLand",
        app: "GoLand",
        cli: None,
    },
    EditorApp {
        name: "Fleet",
        app: "Fleet",
        cli: None,
    },
    EditorApp {
        name: "BBEdit",
        app: "BBEdit",
        cli: None,
    },
    EditorApp {
        name: "Terminal",
        app: "Terminal",
        cli: None,
    },
    EditorApp {
        name: "iTerm",
        app: "iTerm",
        cli: None,
    },
    EditorApp {
        name: "Ghostty",
        app: "Ghostty",
        cli: None,
    },
    EditorApp {
        name: "Warp",
        app: "Warp",
        cli: None,
    },
    EditorApp {
        name: "Finder",
        app: "Finder",
        cli: None,
    },
];

fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
        dirs.push(home.join("Applications/JetBrains Toolbox"));
    }
    dirs
}

/// Editors present on this machine (Finder always is).
pub fn installed() -> Vec<EditorApp> {
    let dirs = app_dirs();
    EDITORS
        .iter()
        .copied()
        .filter(|editor| {
            editor.app == "Finder"
                || dirs
                    .iter()
                    .any(|dir| dir.join(format!("{}.app", editor.app)).exists())
        })
        .collect()
}

pub fn by_name(name: &str) -> Option<EditorApp> {
    EDITORS.iter().copied().find(|editor| editor.name == name)
}

/// Open `path` (optionally at `line`) in the editor.
pub fn open(editor: EditorApp, path: &Path, line: Option<usize>) -> anyhow::Result<()> {
    if editor.app == "Finder" {
        Command::new("open").arg("-R").arg(path).spawn()?;
        return Ok(());
    }
    if let (Some((cli, style)), Some(line)) = (editor.cli, line)
        && let Ok(cli) = which::which(cli)
    {
        let target = format!("{}:{line}", path.display());
        let mut command = Command::new(cli);
        if style == LineStyle::Goto {
            command.arg("-g");
        }
        command.arg(target).spawn()?;
        return Ok(());
    }
    Command::new("open")
        .arg("-a")
        .arg(editor.app)
        .arg(path)
        .spawn()?;
    Ok(())
}
