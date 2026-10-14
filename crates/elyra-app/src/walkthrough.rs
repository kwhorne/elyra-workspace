//! Reviewing a branch one instruction at a time: what was asked, what the
//! agent said it did, and the lines it wrote (the ones nobody has read
//! marked), oldest first. Marking a step as read marks its lines, as Changes
//! does; the lines no agent wrote come last.

use crate::app_state::AppState;
use crate::instructions::{Group, Overview};
use crate::workspace::Workspace;
use elyra_core::ProjectId;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;

/// The most lines listed per file in a step (with their numbers; a gap in
/// the numbers shows as ⋯).
const MAX_LINES_PER_FILE: usize = 200;

pub struct Walkthrough {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
    dir: PathBuf,
    project: ProjectId,
    overview: Option<Result<Overview, String>>,
    step: usize,
}

impl Walkthrough {
    pub fn new(
        app: Entity<AppState>,
        workspace: WeakEntity<Workspace>,
        dir: PathBuf,
        project: ProjectId,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            app,
            workspace,
            dir,
            project,
            overview: None,
            step: 0,
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.overview = None;
        let (dir, project) = (self.dir.clone(), self.project);
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir)?;
            crate::instructions::load(&root, project, &elyra_core::paths::database_path())
        });
        cx.spawn(async move |this, cx| {
            let overview = job.await.map_err(|err| format!("{err:#}"));
            let _ = this.update(cx, |this, cx| {
                // Start at the first instruction with something unread.
                this.step = overview
                    .as_ref()
                    .ok()
                    .and_then(|o| {
                        o.groups
                            .iter()
                            .position(|g| g.turn.is_some() && g.unread > 0)
                    })
                    .unwrap_or(0);
                this.overview = Some(overview);
                cx.notify();
            });
        })
        .detach();
    }

    fn groups(&self) -> &[Group] {
        match &self.overview {
            Some(Ok(overview)) => &overview.groups,
            _ => &[],
        }
    }

    fn go(&mut self, step: usize, cx: &mut Context<Self>) {
        if step < self.groups().len() {
            self.step = step;
            cx.notify();
        }
    }

    /// Mark the lines of the current step as read, and go on.
    fn mark_read_and_next(&mut self, cx: &mut Context<Self>) {
        let project = self.project;
        let Some(Ok(overview)) = &mut self.overview else {
            return;
        };
        let Some(group) = overview.groups.get_mut(self.step) else {
            return;
        };
        let store = &self.app.read(cx).store;
        for (path, lines) in &mut group.detail {
            let hashes: Vec<i64> = lines.iter().filter(|l| !l.read).map(|l| l.hash).collect();
            if hashes.is_empty() {
                continue;
            }
            match store.mark_lines_reviewed(project, path, &hashes, chrono::Utc::now()) {
                Ok(_) => lines.iter_mut().for_each(|l| l.read = true),
                Err(err) => log::warn!("marking {path} as read: {err:#}"),
            }
        }
        group.unread = 0;
        let next = (self.step + 1).min(overview.groups.len().saturating_sub(1));
        self.step = next;
        cx.notify();
    }

    fn open_thread(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((thread, _)) = self.groups().get(self.step).and_then(|g| g.turn) else {
            return;
        };
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.open_thread_from_panel(thread, window, cx)
        });
    }

    fn render_step(&self, group: &Group, cx: &Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let mono = cx.theme().mono_font_family.clone();
        let by_agent = group.turn.is_some();
        let headline = if by_agent {
            group
                .instruction
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("(an empty message)")
                .to_string()
        } else {
            "Not written by an agent in Elyra Workspace".to_string()
        };
        let mut meta: Vec<String> = Vec::new();
        if by_agent {
            meta.extend([group.thread.clone(), group.agent.clone()]);
            if let Some(at) = group.at {
                meta.push(
                    at.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string(),
                );
            }
            if let Some(cost) = group.cost {
                meta.push(crate::thread_session::format_usd(cost));
            }
        }
        meta.push(format!(
            "{} line{}{}",
            group.lines,
            if group.lines == 1 { "" } else { "s" },
            if by_agent {
                format!(", {} unread", group.unread)
            } else {
                String::new()
            }
        ));
        meta.retain(|m| !m.is_empty());
        let section = |title: &'static str| {
            div()
                .pt_2()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(title)
        };
        let clip = |text: &str, max: usize| {
            if text.chars().count() > max {
                format!("{}…", text.chars().take(max).collect::<String>())
            } else {
                text.to_string()
            }
        };
        let files =
            group.detail.iter().map(|(path, lines)| {
                let mut rows: Vec<AnyElement> = Vec::new();
                let mut previous: Option<usize> = None;
                for line in lines.iter().take(MAX_LINES_PER_FILE) {
                    if previous.is_some_and(|p| line.number > p + 1) {
                        rows.push(
                            div()
                                .pl(px(52.))
                                .text_color(muted)
                                .child("⋯")
                                .into_any_element(),
                        );
                    }
                    previous = Some(line.number);
                    rows.push(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(40.))
                                    .flex_none()
                                    .text_right()
                                    .text_color(muted)
                                    .child(line.number.to_string()),
                            )
                            .child(div().w(px(8.)).flex_none().when(
                                by_agent && !line.read,
                                |this| {
                                    this.child(
                                        div()
                                            .mt(px(5.))
                                            .size(px(6.))
                                            .rounded_full()
                                            .bg(cx.theme().warning),
                                    )
                                },
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(line.text.replace('\t', "    ")),
                            )
                            .into_any_element(),
                    );
                }
                if lines.len() > MAX_LINES_PER_FILE {
                    rows.push(
                        div()
                            .pl(px(52.))
                            .text_color(muted)
                            .child(format!(
                                "and {} more lines",
                                lines.len() - MAX_LINES_PER_FILE
                            ))
                            .into_any_element(),
                    );
                }
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(path.clone()),
                    )
                    .child(
                        v_flex()
                            .p_2()
                            .rounded_md()
                            .bg(cx.theme().muted.opacity(0.5))
                            .font_family(mono.clone())
                            .text_size(cx.theme().mono_font_size)
                            .children(rows),
                    )
            });
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(headline),
            )
            .child(div().text_xs().text_color(muted).child(meta.join(" · ")))
            .when_some(group.checks, |this, passed| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(if passed {
                            cx.theme().success
                        } else {
                            cx.theme().danger
                        })
                        .child(if passed {
                            "The checks passed after it."
                        } else {
                            "The checks failed after it."
                        }),
                )
            })
            .when(by_agent && group.instruction.lines().count() > 1, |this| {
                this.child(section("WHAT WAS ASKED"))
                    .child(div().text_sm().child(clip(&group.instruction, 1500)))
            })
            .when(!group.reply.is_empty(), |this| {
                this.child(section("WHAT THE AGENT SAID"))
                    .child(div().text_sm().child(TextView::markdown(
                        SharedString::from(format!("walkthrough-reply-{}", self.step)),
                        clip(&group.reply, 2000),
                    )))
            })
            .child(section("THE LINES IT WROTE"))
            .when(group.detail.is_empty(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Its lines have changed since, or only removed something."),
                )
            })
            .children(files)
            .into_any_element()
    }
}

impl Render for Walkthrough {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let groups = self.groups();
        let count = groups.len();
        let current = groups.get(self.step);
        let unread_left: usize = groups.iter().map(|g| g.unread).sum();
        let can_mark = current.is_some_and(|g| g.turn.is_some() && g.unread > 0);
        let base = match &self.overview {
            Some(Ok(overview)) => format!("compared with {}", overview.base),
            _ => String::new(),
        };
        let body: AnyElement = match &self.overview {
            None => h_flex()
                .gap_2()
                .text_sm()
                .text_color(muted)
                .child(Spinner::new().small())
                .child("Reading which instructions wrote the branch…")
                .into_any_element(),
            Some(Err(err)) => div()
                .text_sm()
                .text_color(muted)
                .child(err.clone())
                .into_any_element(),
            Some(Ok(_)) if count == 0 => div()
                .text_sm()
                .text_color(muted)
                .child("The branch adds no lines.")
                .into_any_element(),
            Some(Ok(_)) => current
                .map(|group| self.render_step(group, cx))
                .unwrap_or_else(|| div().into_any_element()),
        };
        let steps = groups.iter().enumerate().map(|(index, group)| {
            let selected = index == self.step;
            let label = match group.turn {
                Some(_) => group
                    .instruction
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("")
                    .to_string(),
                None => "Not by an agent".to_string(),
            };
            h_flex()
                .id(("walkthrough-step", index))
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .when(selected, |this| this.bg(cx.theme().accent))
                .hover(|this| this.bg(cx.theme().muted))
                .child(
                    div()
                        .w(px(18.))
                        .flex_none()
                        .text_xs()
                        .text_color(muted)
                        .child(format!("{}", index + 1)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(label),
                )
                .when(group.turn.is_some() && group.unread == 0, |this| {
                    this.child(
                        Icon::new(IconName::Check)
                            .xsmall()
                            .text_color(cx.theme().success),
                    )
                })
                .when(group.unread > 0, |this| {
                    this.child(div().size(px(6.)).rounded_full().bg(cx.theme().warning))
                })
                .on_click(cx.listener(move |this, _, _, cx| this.go(index, cx)))
        });
        let steps: Vec<_> = steps.collect();
        let step = self.step;
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Icon::new(IconName::ListChecks).small())
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Review this branch"),
                    )
                    .child(div().flex_1().text_xs().text_color(muted).child(base))
                    .when(count > 0, |this| {
                        this.child(div().text_xs().text_color(muted).child(format!(
                            "{} of {count} · {unread_left} unread lines",
                            step + 1
                        )))
                    })
                    .child(
                        Button::new("walkthrough-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(
                        v_flex()
                            .id("walkthrough-steps")
                            .w(px(260.))
                            .flex_none()
                            .h_full()
                            .p_2()
                            .gap_0p5()
                            .overflow_y_scroll()
                            .border_r_1()
                            .border_color(cx.theme().border)
                            .children(steps),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                div()
                                    .id("walkthrough-body")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .p_4()
                                    .child(div().max_w(px(900.)).child(body)),
                            )
                            .when(count > 0, |this| {
                                this.child(
                                    h_flex()
                                        .px_4()
                                        .py_2()
                                        .gap_2()
                                        .border_t_1()
                                        .border_color(cx.theme().border)
                                        .child(
                                            Button::new("walkthrough-previous")
                                                .small()
                                                .ghost()
                                                .icon(IconName::ChevronLeft)
                                                .label("Previous")
                                                .disabled(step == 0)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.go(step.saturating_sub(1), cx)
                                                })),
                                        )
                                        .child(
                                            Button::new("walkthrough-open")
                                                .small()
                                                .ghost()
                                                .label("Open thread")
                                                .disabled(current.is_none_or(|g| g.turn.is_none()))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.open_thread(window, cx)
                                                })),
                                        )
                                        .child(div().flex_1())
                                        .child(
                                            Button::new("walkthrough-next")
                                                .small()
                                                .ghost()
                                                .label("Next")
                                                .disabled(step + 1 >= count)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.go(step + 1, cx)
                                                })),
                                        )
                                        .child(
                                            Button::new("walkthrough-read")
                                                .small()
                                                .primary()
                                                .icon(IconName::Check)
                                                .label("Mark as read and next")
                                                .disabled(!can_mark)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.mark_read_and_next(cx)
                                                })),
                                        ),
                                )
                            }),
                    ),
            )
    }
}
