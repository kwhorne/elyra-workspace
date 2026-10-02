//! The Files tab (⇧⌘E): a .gitignore-aware explorer and a code editor with
//! autosave, external-change detection and Markdown/image previews.

use crate::actions::SaveFile;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState, InputEvent, Position};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MAX_EDITABLE: u64 = 2 * 1024 * 1024;
const AUTOSAVE_DELAY: Duration = Duration::from_millis(400);
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// One directory level, .gitignore-aware, directories first. `.git` is
/// always hidden; other dotfiles are shown.
pub fn list_dir(root: &Path, rel: &str) -> Vec<DirEntry> {
    let dir = root.join(rel);
    let walker = ignore::WalkBuilder::new(&dir)
        .max_depth(Some(1))
        .hidden(false)
        .parents(true)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    let mut entries: Vec<DirEntry> = walker
        .flatten()
        .filter(|entry| entry.depth() == 1)
        .map(|entry| DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: entry.file_type().is_some_and(|t| t.is_dir()),
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Map a file name to a highlighter language name.
pub fn language_for(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if name == "cargo.lock" || name == "uv.lock" || name == "poetry.lock" {
        return "toml";
    }
    if name == "makefile" || name == "gnumakefile" {
        return "make";
    }
    if name == "dockerfile"
        || name.ends_with(".sh")
        || name.ends_with(".zsh")
        || name == ".zshrc"
        || name == ".bashrc"
    {
        return "bash";
    }
    let ext = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    match ext {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "json" | "jsonc" | "json5" => "json",
        "toml" => "toml",
        "md" | "mdx" | "markdown" => "markdown",
        "py" | "pyi" => "python",
        "go" => "go",
        "rb" | "rake" | "gemspec" => "ruby",
        "php" => "php",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "cs" => "csharp",
        "css" | "scss" | "less" => "css",
        "html" | "htm" | "vue" => "html",
        "svelte" => "svelte",
        "astro" => "astro",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        "lua" => "lua",
        "zig" => "zig",
        "ex" | "exs" => "elixir",
        "erb" => "erb",
        "ejs" => "ejs",
        "graphql" | "gql" => "graphql",
        "proto" => "proto",
        "scala" => "scala",
        "cmake" => "cmake",
        "diff" | "patch" => "diff",
        "bash" | "sh" | "zsh" | "fish" => "bash",
        _ => "text",
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

enum Content {
    Text {
        editor: Entity<EditorState>,
        /// What is on disk as far as we know.
        saved: String,
        mtime: Option<SystemTime>,
        dirty: bool,
        /// The file changed on disk while there were unsaved edits.
        conflict: bool,
        preview: bool,
        _save: Option<Task<()>>,
    },
    Image,
    Unsupported(String),
}

struct OpenFile {
    rel: String,
    content: Content,
}

pub enum FilesEvent {
    /// Mention a file in the composer.
    Mention(String),
}

pub struct FilesView {
    root: PathBuf,
    children: HashMap<String, Vec<DirEntry>>,
    expanded: HashSet<String>,
    open: Option<OpenFile>,
    show_tree: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FilesEvent> for FilesView {}

impl FilesView {
    pub fn new(root: PathBuf, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            root,
            children: HashMap::new(),
            expanded: HashSet::new(),
            open: None,
            show_tree: true,
            focus: cx.focus_handle(),
            _subscriptions: Vec::new(),
        };
        // One directory level is cheap; listing it now avoids an empty flash.
        let top = list_dir(&this.root, "");
        this.children.insert(String::new(), top);
        this
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        self.root = root;
        self.children.clear();
        self.expanded.clear();
        self.open = None;
        let top = list_dir(&self.root, "");
        self.children.insert(String::new(), top);
        cx.notify();
    }

    /// Re-list expanded directories and pick up external edits.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirs: Vec<String> = std::iter::once(String::new())
            .chain(self.expanded.iter().cloned())
            .collect();
        for dir in dirs {
            self.load_dir(dir, cx);
        }
        self.check_disk(window, cx);
    }

    fn load_dir(&mut self, rel: String, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let job = cx.background_executor().spawn({
            let rel = rel.clone();
            async move { list_dir(&root, &rel) }
        });
        cx.spawn(async move |this, cx| {
            let entries = job.await;
            let _ = this.update(cx, |this, cx| {
                this.children.insert(rel, entries);
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_dir(&mut self, rel: String, cx: &mut Context<Self>) {
        if !self.expanded.remove(&rel) {
            self.expanded.insert(rel.clone());
            self.load_dir(rel, cx);
        }
        cx.notify();
    }

    pub fn open_file(
        &mut self,
        rel: &str,
        line: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open.as_ref().is_some_and(|open| open.rel == rel) {
            if let Some(line) = line {
                self.go_to_line(line, window, cx);
            }
            return;
        }
        self.flush(cx);
        // Reveal the file in the tree.
        let mut prefix = String::new();
        for part in rel
            .split('/')
            .collect::<Vec<_>>()
            .iter()
            .rev()
            .skip(1)
            .rev()
        {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if self.expanded.insert(prefix.clone()) {
                self.load_dir(prefix.clone(), cx);
            }
        }
        let path = self.root.join(rel);
        let ext = rel
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default();
        let content = if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            Content::Image
        } else {
            match std::fs::metadata(&path) {
                Err(err) => Content::Unsupported(format!("Can't open: {err}")),
                Ok(meta) if meta.len() > MAX_EDITABLE => {
                    Content::Unsupported("This file is too large to edit here.".into())
                }
                Ok(_) => match std::fs::read(&path) {
                    Err(err) => Content::Unsupported(format!("Can't open: {err}")),
                    Ok(bytes) if bytes.iter().take(8000).any(|b| *b == 0) => {
                        Content::Unsupported("Binary file".into())
                    }
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        let editor = cx.new(|cx| {
                            EditorState::new(window, cx)
                                .language(language_for(rel))
                                .line_number(true)
                                .default_value(text.clone())
                        });
                        self._subscriptions.push(cx.subscribe_in(
                            &editor,
                            window,
                            |this, _, event: &InputEvent, _, cx| {
                                if let InputEvent::Change = event {
                                    this.edited(cx);
                                }
                            },
                        ));
                        Content::Text {
                            editor,
                            saved: text,
                            mtime: modified(&path),
                            dirty: false,
                            conflict: false,
                            preview: false,
                            _save: None,
                        }
                    }
                },
            }
        };
        self.open = Some(OpenFile {
            rel: rel.to_string(),
            content,
        });
        if let Some(line) = line {
            self.go_to_line(line, window, cx);
        }
        cx.notify();
    }

    /// Move the cursor to a 1-based line.
    pub fn go_to_line(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(OpenFile {
            content: Content::Text {
                editor, preview, ..
            },
            ..
        }) = &mut self.open
        {
            *preview = false;
            editor.update(cx, |editor, cx| {
                editor.set_cursor_position(
                    Position::new(line.saturating_sub(1) as u32, 0),
                    window,
                    cx,
                )
            });
        }
    }

    fn edited(&mut self, cx: &mut Context<Self>) {
        let Some(OpenFile {
            content:
                Content::Text {
                    editor,
                    saved,
                    dirty,
                    conflict,
                    _save,
                    ..
                },
            ..
        }) = &mut self.open
        else {
            return;
        };
        *dirty = editor.read(cx).value().as_ref() != saved.as_str();
        if *dirty && !*conflict {
            *_save = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AUTOSAVE_DELAY).await;
                let _ = this.update(cx, |this, cx| this.save(false, cx));
            }));
        }
        cx.notify();
    }

    /// Write the buffer. Unless `force`, refuses when the file changed on
    /// disk since it was loaded and flags a conflict instead.
    fn save(&mut self, force: bool, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let Some(OpenFile {
            rel,
            content:
                Content::Text {
                    editor,
                    saved,
                    mtime,
                    dirty,
                    conflict,
                    ..
                },
        }) = &mut self.open
        else {
            return;
        };
        if !*dirty && !force {
            return;
        }
        let path = root.join(&*rel);
        if !force && modified(&path) != *mtime {
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            if on_disk != *saved {
                *conflict = true;
                cx.notify();
                return;
            }
        }
        let text = editor.read(cx).value().to_string();
        match std::fs::write(&path, &text) {
            Ok(()) => {
                *saved = text;
                *mtime = modified(&path);
                *dirty = false;
                *conflict = false;
            }
            Err(err) => log::error!("saving {}: {err}", path.display()),
        }
        cx.notify();
    }

    /// Save pending edits now (before switching files or closing).
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self.save(false, cx);
    }

    fn on_save(&mut self, _: &SaveFile, _: &mut Window, cx: &mut Context<Self>) {
        self.save(false, cx);
    }

    /// Reload a clean buffer whose file changed on disk; flag a conflict for
    /// a dirty one.
    fn check_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let Some(OpenFile {
            rel,
            content:
                Content::Text {
                    editor,
                    saved,
                    mtime,
                    dirty,
                    conflict,
                    ..
                },
        }) = &mut self.open
        else {
            return;
        };
        let path = root.join(&*rel);
        let disk_time = modified(&path);
        if disk_time == *mtime {
            return;
        }
        let Ok(on_disk) = std::fs::read_to_string(&path) else {
            return;
        };
        *mtime = disk_time;
        if on_disk == *saved {
            return;
        }
        if *dirty {
            *conflict = true;
        } else {
            *saved = on_disk.clone();
            editor.update(cx, |editor, cx| editor.set_value(on_disk, window, cx));
        }
        cx.notify();
    }

    fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        if let Some(OpenFile {
            rel,
            content:
                Content::Text {
                    editor,
                    saved,
                    mtime,
                    dirty,
                    conflict,
                    ..
                },
        }) = &mut self.open
        {
            let path = root.join(&*rel);
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            *saved = text.clone();
            *mtime = modified(&path);
            *dirty = false;
            *conflict = false;
            editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
            cx.notify();
        }
    }

    fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(OpenFile {
            content: Content::Text { preview, .. },
            ..
        }) = &mut self.open
        {
            *preview = !*preview;
            cx.notify();
        }
    }

    pub fn open_path(&self) -> Option<PathBuf> {
        self.open.as_ref().map(|open| self.root.join(&open.rel))
    }

    // ---- rendering ------------------------------------------------------

    fn render_tree(&self, cx: &Context<Self>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        self.push_rows("", 0, &mut rows, cx);
        if rows.is_empty() && self.children.contains_key("") {
            rows.push(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("This folder is empty.")
                    .into_any_element(),
            );
        }
        div()
            .id("files-tree")
            .size_full()
            .overflow_y_scroll()
            .py_1()
            .children(rows)
            .into_any_element()
    }

    fn push_rows(&self, dir: &str, depth: usize, rows: &mut Vec<AnyElement>, cx: &Context<Self>) {
        let Some(entries) = self.children.get(dir) else {
            return;
        };
        let open = self.open.as_ref().map(|o| o.rel.as_str());
        for entry in entries {
            let rel = if dir.is_empty() {
                entry.name.clone()
            } else {
                format!("{dir}/{}", entry.name)
            };
            let expanded = entry.is_dir && self.expanded.contains(&rel);
            let selected = open == Some(rel.as_str());
            let icon = match (entry.is_dir, expanded) {
                (true, true) => IconName::FolderOpen,
                (true, false) => IconName::Folder,
                _ => IconName::File,
            };
            let is_dir = entry.is_dir;
            let click_rel = rel.clone();
            rows.push(
                h_flex()
                    .id(SharedString::from(format!("file-{rel}")))
                    .w_full()
                    .h(px(24.))
                    .pl(px(8. + depth as f32 * 12.))
                    .pr_2()
                    .gap_1p5()
                    .text_sm()
                    .cursor_pointer()
                    .when(selected, |this| this.bg(cx.theme().accent))
                    .hover(|this| this.bg(cx.theme().accent.opacity(0.5)))
                    .child(
                        Icon::new(icon)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(entry.name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if is_dir {
                            this.toggle_dir(click_rel.clone(), cx);
                        } else {
                            this.open_file(&click_rel.clone(), None, window, cx);
                        }
                    }))
                    .into_any_element(),
            );
            if expanded {
                self.push_rows(&rel, depth + 1, rows, cx);
            }
        }
    }

    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let open = self.open.as_ref();
        let status = match open.map(|o| &o.content) {
            Some(Content::Text { conflict: true, .. }) => "Changed on disk",
            Some(Content::Text { dirty: true, .. }) => "Unsaved",
            Some(Content::Text { .. }) => "Saved",
            _ => "",
        };
        let is_markdown = open.is_some_and(|o| language_for(&o.rel) == "markdown");
        let previewing = matches!(
            open.map(|o| &o.content),
            Some(Content::Text { preview: true, .. })
        );
        h_flex()
            .h(px(32.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("files-toggle-tree")
                    .ghost()
                    .xsmall()
                    .icon(IconName::PanelLeft)
                    .tooltip("Toggle file tree")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_tree = !this.show_tree;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(open.map(|o| o.rel.clone()).unwrap_or_default()),
            )
            .when(!status.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            })
            .when(is_markdown, |this| {
                this.child(
                    Button::new("files-preview")
                        .ghost()
                        .xsmall()
                        .icon(if previewing {
                            IconName::Code
                        } else {
                            IconName::Eye
                        })
                        .tooltip(if previewing { "Edit" } else { "Preview" })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_preview(cx))),
                )
            })
            .when_some(open.map(|o| o.rel.clone()), |this, rel| {
                this.child(
                    Button::new("files-mention")
                        .ghost()
                        .xsmall()
                        .icon(IconName::AtSign)
                        .tooltip("Mention in chat")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(FilesEvent::Mention(rel.clone()))
                        })),
                )
            })
            .child(
                Button::new("files-refresh")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh")
                    .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
            )
            .into_any_element()
    }

    fn render_content(&self, cx: &Context<Self>) -> AnyElement {
        let muted = |text: String| {
            div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text)
                .into_any_element()
        };
        let Some(open) = &self.open else {
            return muted("Pick a file to view or edit it.".into());
        };
        match &open.content {
            Content::Unsupported(message) => muted(message.clone()),
            Content::Image => div()
                .size_full()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(self.root.join(&open.rel))
                        .max_w_full()
                        .max_h_full()
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            Content::Text {
                editor,
                conflict,
                preview,
                ..
            } => v_flex()
                .size_full()
                .when(*conflict, |this| {
                    this.child(
                        h_flex()
                            .px_3()
                            .py_1p5()
                            .gap_2()
                            .bg(cx.theme().warning.opacity(0.15))
                            .text_sm()
                            .child(
                                div()
                                    .flex_1()
                                    .child("This file changed on disk and you have unsaved edits."),
                            )
                            .child(
                                Button::new("files-reload")
                                    .xsmall()
                                    .label("Reload")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reload_from_disk(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("files-overwrite")
                                    .xsmall()
                                    .warning()
                                    .label("Overwrite")
                                    .on_click(cx.listener(|this, _, _, cx| this.save(true, cx))),
                            ),
                    )
                })
                .child(if *preview {
                    div()
                        .id("files-md-preview")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p_4()
                        .child(TextView::markdown(
                            "files-markdown",
                            editor.read(cx).value().to_string(),
                        ))
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(
                            Editor::new(editor)
                                .h_full()
                                .appearance(false)
                                .text_size(cx.theme().mono_font_size)
                                .font_family(cx.theme().mono_font_family.clone()),
                        )
                        .into_any_element()
                })
                .into_any_element(),
        }
    }
}

impl Focusable for FilesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FilesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .key_context("FilesView")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_save))
            .child(self.render_header(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.show_tree, |this| {
                        this.child(
                            div()
                                .w(px(220.))
                                .flex_none()
                                .h_full()
                                .border_r_1()
                                .border_color(cx.theme().border)
                                .child(self.render_tree(cx)),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(self.render_content(cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{language_for, list_dir};

    #[test]
    fn maps_languages() {
        assert_eq!(language_for("src/main.rs"), "rust");
        assert_eq!(language_for("web/App.tsx"), "tsx");
        assert_eq!(language_for("Makefile"), "make");
        assert_eq!(language_for("README.md"), "markdown");
        assert_eq!(language_for("Cargo.lock"), "toml");
        assert_eq!(language_for("notes.txt"), "text");
    }

    #[test]
    fn lists_directories_first_and_respects_gitignore() {
        let dir = std::env::temp_dir().join(format!("elyra-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();
        std::fs::write(dir.join("A.md"), "").unwrap();
        let names: Vec<String> = list_dir(&dir, "").into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["src", ".gitignore", "A.md", "b.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
