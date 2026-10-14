//! Search every thread (⌥⌘F): messages, tool calls and the start of tool
//! output, from the full-text index in the store. Hits are grouped by thread,
//! best match first; clicking one opens the thread at that entry.

use crate::app_state::AppState;
use crate::workspace::Workspace;
use elyra_core::{ItemId, ProjectId, SearchFilter, SearchHit, ThreadId};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::ops::Range;
use std::time::Duration;

/// Wait this long after the last key before searching.
const DEBOUNCE: Duration = Duration::from_millis(150);
const MAX_HITS: usize = 200;

pub struct ThreadSearch {
    app: Entity<AppState>,
    workspace: WeakEntity<Workspace>,
    query: Entity<InputState>,
    project: Option<ProjectId>,
    archived: bool,
    tools: bool,
    hits: Vec<SearchHit>,
    error: Option<String>,
    /// Bumped per search, so an older result doesn't replace a newer one.
    generation: u64,
    _search: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ThreadSearch {
    pub fn new(
        app: Entity<AppState>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search every thread — words, or \"an exact phrase\"")
        });
        let subscriptions = vec![cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this._search = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(DEBOUNCE).await;
                    let _ = this.update(cx, |this, cx| this.search(cx));
                }));
            }
        })];
        query.update(cx, |input, cx| input.focus(window, cx));
        Self {
            app,
            workspace,
            query,
            project: None,
            archived: false,
            tools: true,
            hits: Vec::new(),
            error: None,
            generation: 0,
            _search: None,
            _subscriptions: subscriptions,
        }
    }

    /// Focus the search field (the panel is opened again).
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn search(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        let filter = SearchFilter {
            project: self.project,
            archived: self.archived,
            messages_only: !self.tools,
        };
        self.generation += 1;
        let generation = self.generation;
        let job = search_in_background(query, filter, MAX_HITS, cx);
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return; // a newer search is running
                }
                match result {
                    Ok(hits) => {
                        this.hits = hits;
                        this.error = None;
                    }
                    Err(err) => {
                        this.hits.clear();
                        this.error = Some(format!("{err:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_project(&mut self, project: Option<ProjectId>, cx: &mut Context<Self>) {
        self.project = project;
        self.search(cx);
    }

    fn open(
        &mut self,
        thread: ThreadId,
        item: ItemId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.workspace.update(cx, |workspace, cx| {
            workspace.open_thread_at(thread, item, window, cx)
        });
    }

    /// Hits grouped by thread, in the order of each thread's best hit.
    fn groups(&self) -> Vec<(ThreadId, Vec<&SearchHit>)> {
        let mut groups: Vec<(ThreadId, Vec<&SearchHit>)> = Vec::new();
        for hit in &self.hits {
            match groups.iter_mut().find(|(thread, _)| *thread == hit.thread) {
                Some((_, hits)) => hits.push(hit),
                None => groups.push((hit.thread, vec![hit])),
            }
        }
        groups
    }

    fn hit_row(&self, hit: &SearchHit, index: usize, cx: &Context<Self>) -> AnyElement {
        let (text, marks) = marked(&hit.snippet);
        let highlight = HighlightStyle {
            background_color: Some(cx.theme().warning.opacity(0.3)),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let (thread, item) = (hit.thread, hit.item);
        h_flex()
            .id(("search-hit", index))
            .items_start()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_sm()
            .cursor_pointer()
            .hover(|this| this.bg(cx.theme().muted))
            .child(
                div()
                    .w(px(52.))
                    .flex_none()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(kind_label(&hit.kind)),
            )
            .child(
                div().flex_1().min_w_0().text_sm().child(
                    StyledText::new(text)
                        .with_highlights(marks.into_iter().map(|range| (range, highlight))),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        hit.at
                            .with_timezone(&chrono::Local)
                            .format("%Y-%m-%d")
                            .to_string(),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| this.open(thread, item, window, cx)))
            .into_any_element()
    }
}

/// Search the transcripts on a read-only connection of its own, off the
/// main thread (a common word in a long history takes tens of milliseconds).
pub fn search_in_background(
    query: String,
    filter: SearchFilter,
    limit: usize,
    cx: &App,
) -> Task<anyhow::Result<Vec<SearchHit>>> {
    cx.background_executor().spawn(async move {
        elyra_core::Store::open_read_only(&elyra_core::paths::database_path())?
            .search_transcripts(&query, &filter, limit)
    })
}

/// What kind of entry a hit is in, for its row.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        "user" => "You",
        "assistant" => "Agent",
        "tool_use" => "Tool",
        "tool_result" => "Output",
        _ => "",
    }
}

/// A snippet without its match marks (`\u{2}`…`\u{3}`), on one line, and the
/// byte ranges of the matches in it.
pub fn marked(snippet: &str) -> (String, Vec<Range<usize>>) {
    let mut text = String::with_capacity(snippet.len());
    let mut marks = Vec::new();
    let mut start = None;
    for ch in snippet.chars() {
        match ch {
            '\u{2}' => start = Some(text.len()),
            '\u{3}' => {
                if let Some(start) = start.take() {
                    marks.push(start..text.len());
                }
            }
            '\n' | '\r' | '\t' => text.push(' '),
            _ => text.push(ch),
        }
    }
    (text, marks)
}

impl Render for ThreadSearch {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.app.read(cx);
        let project_label: SharedString = self
            .project
            .and_then(|id| state.project(id))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "All projects".into())
            .into();
        let projects: Vec<(Option<ProjectId>, String)> =
            std::iter::once((None, "All projects".to_string()))
                .chain(state.projects.iter().map(|p| (Some(p.id), p.name.clone())))
                .collect();
        let view = cx.weak_entity();
        let query_empty = self.query.read(cx).value().trim().is_empty();
        let groups = self.groups();
        let threads = groups.len();
        let mut index = 0;
        let results = groups.into_iter().map(|(thread_id, hits)| {
            let thread = state.thread(thread_id);
            let title = thread
                .map(|t| t.title.clone())
                .unwrap_or_else(|| "Thread".into());
            let detail = thread
                .and_then(|t| state.project(t.project_id))
                .map(|p| p.name.clone())
                .unwrap_or_default();
            let archived = thread.is_none_or(|t| t.archived);
            let count = hits.len();
            let rows: Vec<AnyElement> = hits
                .into_iter()
                .map(|hit| {
                    index += 1;
                    self.hit_row(hit, index, cx)
                })
                .collect();
            v_flex()
                .gap_0p5()
                .child(
                    h_flex()
                        .gap_2()
                        .px_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{detail}{} · {count} match{}",
                                    if archived { " · archived" } else { "" },
                                    if count == 1 { "" } else { "es" }
                                )),
                        ),
                )
                .children(rows)
        });
        let results: Vec<_> = results.collect();
        let status = if let Some(error) = &self.error {
            Some(error.clone())
        } else if query_empty {
            Some("Type to search your messages, the agents' replies, tool calls and their output in every thread.".into())
        } else if threads == 0 {
            Some("Nothing found.".into())
        } else {
            None
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Icon::new(IconName::TextSearch).small())
                    .child(
                        div()
                            .flex_none()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Search threads"),
                    )
                    .child(div().flex_1().child(Input::new(&self.query).small()))
                    .child(
                        Button::new("search-tools")
                            .xsmall()
                            .when(self.tools, |b| b.primary())
                            .when(!self.tools, |b| b.ghost())
                            .label("Tool output")
                            .tooltip("Also search tool calls and what they printed")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tools = !this.tools;
                                this.search(cx);
                            })),
                    )
                    .child(
                        Button::new("search-archived")
                            .xsmall()
                            .when(self.archived, |b| b.primary())
                            .when(!self.archived, |b| b.ghost())
                            .label("Archived")
                            .tooltip("Also search archived threads")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.archived = !this.archived;
                                this.search(cx);
                            })),
                    )
                    .child(
                        Button::new("search-project")
                            .small()
                            .ghost()
                            .label(project_label)
                            .dropdown_caret(true)
                            .dropdown_menu(move |mut menu, _, _| {
                                for (id, name) in projects.clone() {
                                    let view = view.clone();
                                    menu = menu.item(PopupMenuItem::new(name).on_click(
                                        move |_, _, cx| {
                                            let _ = view.update(cx, |v, cx| v.set_project(id, cx));
                                        },
                                    ));
                                }
                                menu
                            }),
                    ),
            )
            .child(
                v_flex()
                    .id("search-results")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .gap_3()
                    .when_some(status, |this, status| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(status),
                        )
                    })
                    .when(threads > 0, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} in {threads} thread{}{}",
                                    self.hits.len(),
                                    if threads == 1 { "" } else { "s" },
                                    if self.hits.len() == MAX_HITS {
                                        " (the best matches)"
                                    } else {
                                        ""
                                    }
                                )),
                        )
                    })
                    .children(results),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::marked;

    #[test]
    fn finds_the_marked_matches() {
        let (text, marks) = marked("…the \u{2}cart\u{3} total\nfor \u{2}rabatt\u{3}");
        assert_eq!(text, "…the cart total for rabatt");
        assert_eq!(
            marks.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
            ["cart", "rabatt"]
        );
        assert_eq!(marked("plain"), ("plain".to_string(), vec![]));
    }
}
