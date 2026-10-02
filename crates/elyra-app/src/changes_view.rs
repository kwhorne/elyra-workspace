use elyra_git::{ChangeKind, DiffLineKind, FileChange, FileDiff};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;

/// Working-tree changes for one thread's directory: changed files, the
/// selected file's diff, and commit/push.
pub struct ChangesView {
    cwd: PathBuf,
    is_repo: bool,
    branch: Option<String>,
    changes: Vec<FileChange>,
    selected: Option<usize>,
    diff: Option<FileDiff>,
    loading: bool,
    busy: Option<&'static str>,
    message: Option<(String, bool)>,
    commit_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ChangesView {
    pub fn new(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let commit_input = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        let subscriptions = vec![cx.subscribe_in(
            &commit_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.commit(window, cx);
                }
            },
        )];
        let mut this = Self {
            is_repo: true,
            cwd,
            branch: None,
            changes: Vec::new(),
            selected: None,
            diff: None,
            loading: false,
            busy: None,
            message: None,
            commit_input,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
    }

    pub fn set_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        if self.cwd != cwd {
            self.cwd = cwd;
            self.selected = None;
            self.diff = None;
            self.refresh(cx);
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        self.loading = true;
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            if !elyra_git::is_repo(&cwd) {
                return (false, None, Ok(Vec::new()));
            }
            (
                true,
                elyra_git::current_branch(&cwd),
                elyra_git::status(&cwd),
            )
        });
        cx.spawn(async move |this, cx| {
            let (is_repo, branch, changes) = job.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.is_repo = is_repo;
                this.branch = branch;
                match changes {
                    Ok(changes) => {
                        let previous = this
                            .selected
                            .and_then(|i| this.changes.get(i))
                            .map(|c| c.path.clone());
                        this.changes = changes;
                        this.selected = previous
                            .and_then(|path| this.changes.iter().position(|c| c.path == path))
                            .or(if this.changes.is_empty() {
                                None
                            } else {
                                Some(0)
                            });
                        this.load_diff(cx);
                    }
                    Err(err) => this.message = Some((format!("{err:#}"), true)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(index);
        self.load_diff(cx);
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        let Some(change) = self.selected.and_then(|i| self.changes.get(i)).cloned() else {
            self.diff = None;
            cx.notify();
            return;
        };
        let cwd = self.cwd.clone();
        let job = cx
            .background_executor()
            .spawn(async move { elyra_git::diff_file(&cwd, &change) });
        cx.spawn(async move |this, cx| {
            let diff = job.await;
            let _ = this.update(cx, |this, cx| {
                match diff {
                    Ok(diff) => this.diff = Some(diff),
                    Err(err) => {
                        this.diff = None;
                        this.message = Some((format!("{err:#}"), true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_git(
        &mut self,
        label: &'static str,
        job: impl FnOnce(PathBuf) -> anyhow::Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label);
        self.message = None;
        cx.notify();
        let cwd = self.cwd.clone();
        let task = cx.background_executor().spawn(async move { job(cwd) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = None;
                this.message = Some(match result {
                    Ok(text) => (text, false),
                    Err(err) => (format!("{err:#}"), true),
                });
                this.refresh(cx);
            });
        })
        .detach();
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.commit_input.read(cx).value().trim().to_string();
        if message.is_empty() || self.changes.is_empty() {
            return;
        }
        self.commit_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.run_git(
            "Committing…",
            move |cwd| elyra_git::commit_all(&cwd, &message).map(|sha| format!("Committed {sha}")),
            cx,
        );
    }

    fn push(&mut self, cx: &mut Context<Self>) {
        self.run_git(
            "Pushing…",
            |cwd| elyra_git::push(&cwd).map(|_| "Pushed to origin".to_string()),
            cx,
        );
    }

    fn discard_selected(&mut self, cx: &mut Context<Self>) {
        let Some(change) = self.selected.and_then(|i| self.changes.get(i)).cloned() else {
            return;
        };
        self.run_git(
            "Discarding…",
            move |cwd| {
                elyra_git::discard_file(&cwd, &change)
                    .map(|()| format!("Discarded {}", change.path))
            },
            cx,
        );
    }

    fn kind_color(kind: ChangeKind, cx: &App) -> Hsla {
        match kind {
            ChangeKind::Added | ChangeKind::Untracked => cx.theme().success,
            ChangeKind::Deleted | ChangeKind::Conflicted => cx.theme().danger,
            ChangeKind::Renamed => cx.theme().info,
            ChangeKind::Modified => cx.theme().warning,
        }
    }

    fn render_diff(&self, cx: &App) -> AnyElement {
        let Some(diff) = &self.diff else {
            return div()
                .p_4()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if self.changes.is_empty() {
                    "No changes"
                } else {
                    "Select a file"
                })
                .into_any_element();
        };
        if diff.binary {
            return div()
                .p_4()
                .text_sm()
                .child("Binary file")
                .into_any_element();
        }
        let added_bg = cx.theme().success.opacity(0.12);
        let removed_bg = cx.theme().danger.opacity(0.12);
        let gutter = cx.theme().muted_foreground.opacity(0.7);
        let mut rows: Vec<AnyElement> = Vec::new();
        for (h, hunk) in diff.hunks.iter().enumerate() {
            rows.push(
                div()
                    .id(("hunk", h))
                    .px_2()
                    .py_0p5()
                    .bg(cx.theme().muted)
                    .text_color(cx.theme().muted_foreground)
                    .child(hunk.header.clone())
                    .into_any_element(),
            );
            for line in &hunk.lines {
                let (bg, sign) = match line.kind {
                    DiffLineKind::Added => (Some(added_bg), "+"),
                    DiffLineKind::Removed => (Some(removed_bg), "-"),
                    DiffLineKind::Context => (None, " "),
                };
                let number = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
                rows.push(
                    h_flex()
                        .w_full()
                        .when_some(bg, |this, bg| this.bg(bg))
                        .child(
                            div()
                                .w(px(40.))
                                .flex_none()
                                .pr_1()
                                .text_right()
                                .text_color(gutter)
                                .child(number(line.old_line)),
                        )
                        .child(
                            div()
                                .w(px(40.))
                                .flex_none()
                                .pr_1()
                                .text_right()
                                .text_color(gutter)
                                .child(number(line.new_line)),
                        )
                        .child(div().w(px(14.)).flex_none().child(sign))
                        .child(
                            div()
                                .flex_1()
                                .whitespace_nowrap()
                                .child(if line.text.is_empty() {
                                    " ".to_string()
                                } else {
                                    line.text.replace('\t', "    ")
                                }),
                        )
                        .into_any_element(),
                );
            }
        }
        v_flex()
            .min_w_full()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(cx.theme().mono_font_size)
            .children(rows)
            .into_any_element()
    }
}

impl Render for ChangesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_repo {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::GitBranch).large())
                .child("This folder is not a Git repository")
                .into_any_element();
        }
        let (insertions, deletions) = self
            .diff
            .as_ref()
            .map(|d| (d.additions(), d.deletions()))
            .unwrap_or_default();
        let selected = self.selected;
        let files = self.changes.iter().enumerate().map(|(i, change)| {
            let (dir, name) = match change.path.rsplit_once('/') {
                Some((dir, name)) => (format!("{dir}/"), name.to_string()),
                None => (String::new(), change.path.clone()),
            };
            h_flex()
                .id(("file", i))
                .w_full()
                .px_2()
                .py_0p5()
                .gap_2()
                .rounded_sm()
                .cursor_pointer()
                .text_sm()
                .when(selected == Some(i), |this| this.bg(cx.theme().accent))
                .hover(|this| this.bg(cx.theme().muted))
                .child(
                    div()
                        .w(px(14.))
                        .flex_none()
                        .font_weight(FontWeight::BOLD)
                        .text_xs()
                        .text_color(Self::kind_color(change.kind, cx))
                        .child(change.kind.letter()),
                )
                .child(div().flex_none().child(name))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(dir),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select(i, cx)))
        });

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .child(
                        Icon::new(IconName::GitBranch)
                            .small()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.branch.clone().unwrap_or_else(|| "—".into())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} changed", self.changes.len())),
                    )
                    .child(div().flex_1())
                    .when(self.loading, |this| this.child(Spinner::new().xsmall()))
                    .child(
                        Button::new("refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .tooltip("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .id("files")
                    .max_h(relative(0.35))
                    .flex_none()
                    .overflow_y_scroll()
                    .p_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(files),
            )
            .when(self.diff.is_some(), |this| {
                this.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .gap_2()
                        .text_xs()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(
                                    self.diff
                                        .as_ref()
                                        .map(|d| d.path.clone())
                                        .unwrap_or_default(),
                                ),
                        )
                        .child(
                            div()
                                .text_color(cx.theme().success)
                                .child(format!("+{insertions}")),
                        )
                        .child(
                            div()
                                .text_color(cx.theme().danger)
                                .child(format!("−{deletions}")),
                        )
                        .child(
                            Button::new("discard")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Undo2)
                                .tooltip("Discard changes to this file")
                                .on_click(cx.listener(|this, _, _, cx| this.discard_selected(cx))),
                        ),
                )
            })
            .child(
                div()
                    .id("diff")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .child(self.render_diff(cx)),
            )
            .child(
                v_flex()
                    .p_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when_some(self.message.clone(), |this, (message, is_error)| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(if is_error {
                                    cx.theme().danger
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(message),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.commit_input).small()))
                            .child(
                                Button::new("commit")
                                    .small()
                                    .primary()
                                    .icon(IconName::GitCommitHorizontal)
                                    .label(self.busy.unwrap_or("Commit"))
                                    .loading(self.busy.is_some())
                                    .disabled(self.changes.is_empty() || self.busy.is_some())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.commit(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("push")
                                    .small()
                                    .icon(IconName::Upload)
                                    .tooltip("Push to origin")
                                    .disabled(self.busy.is_some())
                                    .on_click(cx.listener(|this, _, _, cx| this.push(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }
}
