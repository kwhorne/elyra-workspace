//! A child process speaking JSON lines over stdio, shared by adapters.

use crate::ProviderEvent;
use anyhow::{Context as _, Result, anyhow};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

pub(crate) struct JsonlProcess {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
}

/// Find an executable on PATH or in common per-user install locations.
pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
    if let Ok(path) = which::which(name) {
        return Some(path);
    }
    let home = dirs::home_dir()?;
    [
        format!(".{name}/local/{name}"),
        format!(".local/bin/{name}"),
        format!(".npm-global/bin/{name}"),
        format!(".bun/bin/{name}"),
        format!(".volta/bin/{name}"),
    ]
    .into_iter()
    .map(|rel| home.join(rel))
    .chain([
        PathBuf::from(format!("/opt/homebrew/bin/{name}")),
        PathBuf::from(format!("/usr/local/bin/{name}")),
    ])
    .find(|path| path.is_file())
}

impl JsonlProcess {
    /// Spawn `executable args…` in `cwd`. Every stdout JSON line is passed to
    /// `parse`, whose events go to `tx`; `on_event` sees each event first (for
    /// adapter bookkeeping). An `Exited` event is sent when stdout closes.
    pub(crate) fn spawn<P, F>(
        executable: &Path,
        args: &[String],
        cwd: &Path,
        envs: &[(&str, &str)],
        mut parse: P,
        on_event: F,
    ) -> Result<(Arc<Self>, async_channel::Receiver<ProviderEvent>)>
    where
        P: FnMut(&Value) -> Vec<ProviderEvent> + Send + 'static,
        F: Fn(&ProviderEvent) + Send + 'static,
    {
        let mut command = Command::new(executable);
        command
            .current_dir(cwd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("starting {}", executable.display()))?;
        let stdin = child.stdin.take().context("stdin")?;
        let stdout = child.stdout.take().context("stdout")?;
        let stderr = child.stderr.take().context("stderr")?;
        let process = Arc::new(Self {
            stdin: Mutex::new(Some(stdin)),
            child: Mutex::new(child),
        });
        let (tx, rx) = async_channel::unbounded();
        let name = executable
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "provider".into());

        let stderr_tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
        {
            let tail = stderr_tail.clone();
            let name = name.clone();
            std::thread::Builder::new()
                .name(format!("{name}-stderr"))
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        log::debug!("{name} stderr: {line}");
                        let mut tail = tail.lock().unwrap();
                        if tail.len() >= 20 {
                            tail.pop_front();
                        }
                        tail.push_back(line);
                    }
                })?;
        }
        {
            let process = process.clone();
            std::thread::Builder::new()
                .name(format!("{name}-stdout"))
                .spawn(move || {
                    // JSONL framing: split on '\n' only (U+2028 is valid JSON).
                    let mut reader = BufReader::new(stdout);
                    let mut buf = Vec::new();
                    loop {
                        buf.clear();
                        match reader.read_until(b'\n', &mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {}
                        }
                        let line = String::from_utf8_lossy(&buf);
                        let line = line.trim_end_matches(['\n', '\r']);
                        if line.trim().is_empty() {
                            continue;
                        }
                        let Ok(value) = serde_json::from_str::<Value>(line) else {
                            log::debug!("{name} non-json stdout: {line}");
                            continue;
                        };
                        for event in parse(&value) {
                            on_event(&event);
                            if tx.send_blocking(event).is_err() {
                                return;
                            }
                        }
                    }
                    let status = process.child.lock().unwrap().wait().ok();
                    let error = match status {
                        Some(status) if status.success() => None,
                        _ => {
                            let tail = stderr_tail.lock().unwrap();
                            let detail = tail.iter().cloned().collect::<Vec<_>>().join("\n");
                            Some(match status {
                                Some(status) => format!("{name} exited with {status}. {detail}"),
                                None => format!("{name} exited. {detail}"),
                            })
                        }
                    };
                    let _ = tx.send_blocking(ProviderEvent::Exited { error });
                })?;
        }
        Ok((process, rx))
    }

    pub(crate) fn write(&self, frame: &Value) -> Result<()> {
        let mut guard = self.stdin.lock().unwrap();
        let stdin = guard
            .as_mut()
            .ok_or_else(|| anyhow!("the session is closed"))?;
        let mut line = serde_json::to_string(frame)?;
        line.push('\n');
        stdin.write_all(line.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    /// Close stdin and terminate the process.
    pub(crate) fn shutdown(&self) {
        self.stdin.lock().unwrap().take();
        let mut child = self.child.lock().unwrap();
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
        }
    }
}

/// Concatenate the text of a content value (string or block array).
pub(crate) fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Some("image") => Some("[image]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
