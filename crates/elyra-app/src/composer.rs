//! Composer logic independent of rendering: slash and @-mention completion,
//! attachments and prompt assembly.

use crate::thread_session::ThreadSession;
use base64::Engine as _;
use elyra_provider::{ImageAttachment, Prompt, SlashCommand};
use gpui_kit::ImageFormat;
use std::path::{Path, PathBuf};

const MAX_INDEXED_FILES: usize = 40_000;
const MAX_SUGGESTIONS: usize = 40;

#[derive(Clone, Debug, PartialEq)]
pub enum Attachment {
    Image {
        media_type: String,
        data: String,
        size: usize,
    },
    Text {
        content: String,
    },
}

impl Attachment {
    pub fn image(media_type: &str, bytes: &[u8]) -> Self {
        Attachment::Image {
            media_type: media_type.to_string(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            size: bytes.len(),
        }
    }

    pub fn pasted_text(content: String) -> Self {
        Attachment::Text { content }
    }

    pub fn is_image(&self) -> bool {
        matches!(self, Attachment::Image { .. })
    }

    pub fn label(&self) -> String {
        match self {
            Attachment::Image { size, .. } => format!("Image · {} KB", size.div_ceil(1024)),
            Attachment::Text { content } => {
                format!("Pasted text · {} lines", content.lines().count().max(1))
            }
        }
    }
}

pub fn is_large_paste(text: &str) -> bool {
    text.len() > 4000 || text.lines().count() > 60
}

pub fn mime_of(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Webp => "image/webp",
        ImageFormat::Gif => "image/gif",
        _ => "image/png",
    }
}

pub fn image_mime_for_path(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// `@path` mention, relative to the working directory when inside it.
pub fn mention_for(path: &Path, root: &Path) -> String {
    let shown = path
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf());
    let text = shown.to_string_lossy();
    if text.contains(' ') {
        format!("@\"{text}\"")
    } else {
        format!("@{text}")
    }
}

/// Assemble the prompt: composer text, pasted text blocks and images.
pub fn build_prompt(text: &str, attachments: Vec<Attachment>) -> Prompt {
    let mut text = text.trim().to_string();
    let mut images = Vec::new();
    for attachment in attachments {
        match attachment {
            Attachment::Image {
                media_type, data, ..
            } => images.push(ImageAttachment { media_type, data }),
            Attachment::Text { content } => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str("```\n");
                text.push_str(content.trim_end());
                text.push_str("\n```");
            }
        }
    }
    Prompt { text, images }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKind {
    Slash,
    Mention,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PopupItem {
    pub label: String,
    pub detail: String,
    /// Text replacing the token, including a trailing space.
    pub insert: String,
}

pub struct Popup {
    pub kind: PopupKind,
    /// Byte range of the token being completed.
    pub start: usize,
    pub end: usize,
    pub items: Vec<PopupItem>,
    pub selected: usize,
}

/// The completion token ending at `cursor`: `/command` at the very start of
/// the input, or `@path` anywhere. Returns kind, token start and query.
pub fn token_at(value: &str, cursor: usize) -> Option<(PopupKind, usize, String)> {
    let cursor = cursor.min(value.len());
    if !value.is_char_boundary(cursor) {
        return None;
    }
    let before = &value[..cursor];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let token = &before[start..];
    if let Some(query) = token.strip_prefix('/') {
        (start == 0).then(|| (PopupKind::Slash, start, query.to_string()))
    } else {
        token
            .strip_prefix('@')
            .map(|query| (PopupKind::Mention, start, query.to_string()))
    }
}

/// Replace `value[start..end]` with `insert`; returns new text and cursor.
pub fn apply_insert(value: &str, start: usize, end: usize, insert: &str) -> (String, usize) {
    let end = end.min(value.len());
    let start = start.min(end);
    let mut text = String::with_capacity(value.len() + insert.len());
    text.push_str(&value[..start]);
    text.push_str(insert);
    let cursor = text.len();
    text.push_str(&value[end..]);
    (text, cursor)
}

const BUILTINS: &[(&str, &str)] = &[
    ("clear", "Start a fresh conversation in this thread"),
    ("compact", "Summarize the conversation to free context"),
    ("rename", "Rename this thread"),
    ("plan", "Switch to plan mode"),
    ("default", "Switch back to asking for approval"),
    ("status", "Show model, context and cost"),
];

/// Subsequence match score (higher is better); `None` when not matching.
pub fn fuzzy_score(candidate: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let candidate_lower = candidate.to_lowercase();
    let query = query.to_lowercase();
    let mut score = 0i64;
    let mut last: Option<usize> = None;
    let mut chars = candidate_lower.char_indices();
    for q in query.chars() {
        let (index, _) = chars.by_ref().find(|(_, c)| *c == q)?;
        score += match last {
            Some(prev) if index == prev + 1 => 8, // consecutive
            _ => 1,
        };
        let boundary = index == 0
            || candidate_lower[..index]
                .chars()
                .last()
                .is_some_and(|c| matches!(c, '/' | '-' | '_' | '.' | ' ' | ':'));
        if boundary {
            score += 6;
        }
        last = Some(index);
    }
    // Prefer matches in the file name and shorter candidates.
    let name_start = candidate_lower.rfind('/').map(|i| i + 1).unwrap_or(0);
    if candidate_lower[name_start..].contains(&query) {
        score += 20;
    }
    if candidate_lower.starts_with(&query) {
        score += 15;
    }
    Some(score * 100 - candidate.len() as i64)
}

pub fn slash_items(commands: &[SlashCommand], query: &str) -> Vec<PopupItem> {
    let builtin = BUILTINS
        .iter()
        .map(|(name, detail)| (name.to_string(), detail.to_string()));
    let provider = commands
        .iter()
        .filter(|c| !BUILTINS.iter().any(|(name, _)| *name == c.name))
        .map(|c| {
            let detail = if c.argument_hint.is_empty() {
                c.description.clone()
            } else {
                format!("{} — {}", c.argument_hint, c.description)
            };
            (c.name.clone(), detail)
        });
    let mut scored: Vec<(i64, PopupItem)> = builtin
        .chain(provider)
        .filter_map(|(name, detail)| {
            let score = fuzzy_score(&name, query)?;
            Some((
                score,
                PopupItem {
                    label: format!("/{name}"),
                    detail,
                    insert: format!("/{name} "),
                },
            ))
        })
        .collect();
    if !query.is_empty() {
        scored.sort_by_key(|item| std::cmp::Reverse(item.0));
    }
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, item)| item)
        .collect()
}

pub fn mention_items(files: &[String], query: &str) -> Vec<PopupItem> {
    let mut scored: Vec<(i64, &String)> = files
        .iter()
        .filter_map(|path| Some((fuzzy_score(path, query)?, path)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, path)| {
            let (dir, name) = match path.rsplit_once('/') {
                Some((dir, name)) => (dir.to_string(), name.to_string()),
                None => (String::new(), path.clone()),
            };
            let insert = if path.contains(' ') {
                format!("@\"{path}\" ")
            } else {
                format!("@{path} ")
            };
            PopupItem {
                label: name,
                detail: dir,
                insert,
            }
        })
        .collect()
}

/// Subagents matching an `@` query, inserted as `@agent-<name>`.
pub fn agent_items(agents: &[elyra_provider::SlashCommand], query: &str) -> Vec<PopupItem> {
    let query = query.strip_prefix("agent-").unwrap_or(query);
    let mut scored: Vec<(i64, &elyra_provider::SlashCommand)> = agents
        .iter()
        .filter_map(|agent| Some((fuzzy_score(&agent.name, query)?, agent)))
        .collect();
    scored.sort_by_key(|scored| std::cmp::Reverse(scored.0));
    scored
        .into_iter()
        .take(5)
        .map(|(_, agent)| PopupItem {
            label: format!("agent-{}", agent.name),
            detail: agent
                .description
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect(),
            insert: format!("@agent-{} ", agent.name),
        })
        .collect()
}

/// Files and directories under `root`, respecting .gitignore.
pub fn index_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for entry in walker.flatten() {
        if files.len() >= MAX_INDEXED_FILES {
            break;
        }
        let path: PathBuf = entry.path().to_path_buf();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        let mut text = relative.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            text.push('/');
        }
        files.push(text);
    }
    files
}

pub fn status_text(session: &ThreadSession) -> String {
    let thread = &session.thread;
    let mut lines = vec![format!(
        "Provider: {} · Model: {} · Effort: {}",
        thread.provider.label(),
        thread.model.as_deref().unwrap_or("default"),
        thread.effort.as_deref().unwrap_or("default"),
    )];
    if let (Some(used), Some(window)) = (session.usage.context_tokens, session.usage.context_window)
    {
        lines.push(format!(
            "Context: {used} / {window} tokens ({:.0}%)",
            used as f64 / window.max(1) as f64 * 100.
        ));
    }
    if let Some(cost) = session.usage.cost_usd {
        lines.push(format!("Session cost: ${cost:.3}"));
    }
    if let Some(session_id) = &thread.provider_session_id {
        lines.push(format!("Session: {session_id}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_completion_tokens() {
        assert_eq!(
            token_at("/com", 4),
            Some((PopupKind::Slash, 0, "com".into()))
        );
        assert_eq!(token_at("fix /com", 8), None, "slash only at the start");
        assert_eq!(
            token_at("look at @src/ma", 15),
            Some((PopupKind::Mention, 8, "src/ma".into()))
        );
        assert_eq!(token_at("plain text", 10), None);
        assert_eq!(
            token_at("æøå @", 8),
            Some((PopupKind::Mention, 7, String::new()))
        );
    }

    #[test]
    fn inserts_completion() {
        assert_eq!(
            apply_insert("look at @src/ma now", 8, 15, "@src/main.rs "),
            ("look at @src/main.rs  now".into(), 21)
        );
    }

    #[test]
    fn fuzzy_prefers_file_names_and_boundaries() {
        let files = vec![
            "src/composer.rs".to_string(),
            "docs/compose.md".to_string(),
            "crates/core/src/model.rs".to_string(),
        ];
        let items = mention_items(&files, "comp");
        assert_eq!(items[0].label, "compose.md");
        assert!(mention_items(&files, "zzz").is_empty());
        assert_eq!(
            mention_items(&files, "mod")[0].insert,
            "@crates/core/src/model.rs "
        );
    }

    #[test]
    fn slash_items_merge_builtins_and_provider_commands() {
        let commands = vec![
            SlashCommand {
                name: "review".into(),
                description: "Review code".into(),
                argument_hint: String::new(),
            },
            SlashCommand {
                name: "compact".into(),
                description: "dup".into(),
                argument_hint: String::new(),
            },
        ];
        let all = slash_items(&commands, "");
        assert_eq!(all.iter().filter(|i| i.label == "/compact").count(), 1);
        assert_eq!(slash_items(&commands, "rev")[0].label, "/review");
    }

    #[test]
    fn builds_prompt_with_attachments() {
        let prompt = build_prompt(
            "  explain  ",
            vec![
                Attachment::pasted_text("a\nb".into()),
                Attachment::image("image/png", b"png"),
            ],
        );
        assert_eq!(prompt.text, "explain\n\n```\na\nb\n```");
        assert_eq!(prompt.images[0].data, "cG5n");
    }

    #[test]
    fn indexes_files_respecting_gitignore() {
        let dir = std::env::temp_dir().join(format!("elyra-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("target/out"), "").unwrap();
        std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&dir)
            .status()
            .unwrap();
        let files = index_files(&dir);
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(!files.iter().any(|f| f.starts_with("target")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
