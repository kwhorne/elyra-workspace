//! The line record: which lines a turn wrote, the instructions a turn took
//! lines from (the intent guard), and what parallel threads are changing
//! (the intent board).

use super::*;

impl ThreadSession {
    /// Remember which lines this turn added, so `why` and `history` (and the
    /// blame in Changes) can trace them back to its message.
    /// The turn took out lines written for other instructions: say which, and
    /// tell the agent with its next message.
    pub(super) fn guard_intents(
        &mut self,
        item: ItemId,
        before: String,
        after: String,
        removed: &[crate::intent_guard::Removed],
        cx: &mut Context<Self>,
    ) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let intents = crate::intent_guard::find(
            app.read(cx),
            self.project.id,
            (self.thread.id, item),
            removed,
        );
        if intents.is_empty() {
            return;
        }
        self.rewrite_note = Some(crate::intent_guard::note(&intents, self.thread.id));
        self.append(
            ItemContent::Rewrote {
                before,
                after,
                instructions: intents,
            },
            cx,
        );
        cx.notify();
    }

    /// Ask the author of a rewritten instruction whether the new code still
    /// does what it was asked to do (in a side chat); returns the chat.
    pub fn rewrite_request(&self, item: ItemId, index: usize) -> Option<crate::ask_line::Request> {
        let entry = self.items.iter().find(|i| i.id == item)?;
        let ItemContent::Rewrote {
            before,
            after,
            instructions,
        } = &entry.content
        else {
            return None;
        };
        let intent = instructions.get(index)?;
        let root = elyra_git::repo_root(&self.working_dir()).ok()?;
        let since: String = intent
            .files
            .iter()
            .filter_map(|path| elyra_git::diff_text_of(&root, before, after, path).ok())
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .take(10_000)
            .collect();
        Some(crate::ask_line::Request {
            parent: Some(self.thread.id),
            root,
            path: intent.files.first()?.clone(),
            line: None,
            text: intent.lines.first()?.clone(),
            question: crate::intent_guard::question(intent),
            author: Some((intent.thread, intent.item)),
            since: Some(since).filter(|s| !s.trim().is_empty()),
            title: Some(format!(
                "Still holds? · {}",
                intent.instruction.chars().take(40).collect::<String>()
            )),
        })
    }

    /// Remember the chat that asked an instruction's author.
    pub fn set_rewrite_asked(
        &mut self,
        item: ItemId,
        index: usize,
        chat: ThreadId,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        if let ItemContent::Rewrote { instructions, .. } = &mut self.items[position].content
            && let Some(intent) = instructions.get_mut(index)
        {
            intent.asked = Some(chat);
            self.persist_item(position, cx);
            cx.notify();
        }
    }

    pub(super) fn record_line_origins(&mut self, cx: &mut Context<Self>) {
        let Some((item, Some(checkpoint))) =
            self.items
                .iter()
                .rev()
                .find_map(|item| match &item.content {
                    ItemContent::User { checkpoint, .. } => Some((item.id, checkpoint.clone())),
                    _ => None,
                })
        else {
            return;
        };
        let (dir, project, thread) = (self.working_dir(), self.project.id, self.thread.id);
        // The folder as the turn left it, kept (a ref) for taking it back.
        let after_ref = format!("refs/elyra/{}/{}-after", thread.simple(), self.items.len());
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir).ok()?;
            let after =
                elyra_git::checkpoint::create(&root, &after_ref, "elyra: after turn").ok()?;
            let added = elyra_git::checkpoint::added_between(&root, &checkpoint, &after).ok()?;
            // Every file the turn added to, also from the shell, for the board.
            let mut files: Vec<String> = added
                .iter()
                .map(|(path, _)| root.join(path).display().to_string())
                .collect();
            files.dedup();
            let lines = added
                .into_iter()
                .filter_map(|(path, text)| Some((path, elyra_core::line_hash(&text)?)))
                .collect::<Vec<_>>();
            // Lines it took out, to see whose instructions they were for.
            let removed = crate::intent_guard::removed(&root, &checkpoint, &after);
            Some((files, lines, after, removed, checkpoint))
        });
        cx.spawn(async move |this, cx| {
            let Some((files, lines, after, removed, before)) = job.await else {
                return;
            };
            if !removed.is_empty() {
                let _ = this.update(cx, |this, cx| {
                    this.guard_intents(item, before, after.clone(), &removed, cx)
                });
            }
            let _ = this.update(cx, |this, cx| {
                // The turn's summary keeps where it left the folder.
                if let Some(index) = this
                    .items
                    .iter()
                    .rposition(|i| matches!(i.content, ItemContent::TurnSummary { .. }))
                    && let ItemContent::TurnSummary { after: slot, .. } =
                        &mut this.items[index].content
                {
                    *slot = Some(after);
                    this.persist_item(index, cx);
                }
                this.publish_changes(
                    files.into_iter().map(|file| (file, Vec::new())).collect(),
                    cx,
                );
            });
            if lines.is_empty() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(app) = this.app.upgrade()
                    && let Err(err) = app.read(cx).store.record_line_origins(
                        project,
                        thread,
                        item,
                        Utc::now(),
                        &lines,
                    )
                {
                    log::warn!("recording line origins: {err:#}");
                }
                this.lines_recorded += 1;
                cx.notify();
            });
        })
        .detach();
    }

    /// Put files this thread's agent changed on the intent board — (path as
    /// the tool named it, the text it wrote) — and tell this thread and any
    /// other that changed the same file lately.
    pub(super) fn publish_changes(
        &mut self,
        files: Vec<(String, Vec<String>)>,
        cx: &mut Context<Self>,
    ) {
        let dir = self.working_dir();
        let thread = self.thread.id;
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir).ok()?;
            let root = root.canonicalize().unwrap_or(root);
            let repo = elyra_git::common_dir(&dir).ok()?;
            let branch = elyra_git::current_branch(&root);
            let now = Utc::now();
            let intents: Vec<crate::intent_board::Intent> = files
                .into_iter()
                .filter_map(|(path, snippets)| {
                    let file = dir.join(&path);
                    let file = file.canonicalize().unwrap_or(file);
                    let path = file.strip_prefix(&root).ok()?.display().to_string();
                    let symbols = if snippets.is_empty() {
                        Vec::new()
                    } else {
                        let source = std::fs::read_to_string(&file).unwrap_or_default();
                        crate::intent_board::symbols_in(&path, &source, &snippets)
                    };
                    Some(crate::intent_board::Intent {
                        repo: repo.clone(),
                        root: root.clone(),
                        branch: branch.clone(),
                        thread,
                        path,
                        symbols,
                        at: now,
                    })
                })
                .collect();
            Some(intents)
        });
        cx.spawn(async move |this, cx| {
            let Some(intents) = job.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.on_changes(intents, cx));
        })
        .detach();
    }

    pub(super) fn on_changes(
        &mut self,
        intents: Vec<crate::intent_board::Intent>,
        cx: &mut Context<Self>,
    ) {
        use crate::intent_board::{Overlap, label, overlap_notice, related_threads};
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let related = related_threads(app.read(cx), self.thread.id);
        let overlaps: Vec<(crate::intent_board::Intent, Overlap)> = app.update(cx, |app, _| {
            intents
                .into_iter()
                .flat_map(|intent| {
                    app.intents
                        .publish(intent.clone(), |t| related.contains(&t))
                        .into_iter()
                        .map(move |overlap| (intent.clone(), overlap))
                })
                .collect()
        });
        let now = Utc::now();
        for (mine, overlap) in overlaps {
            let (title, agent) = label(app.read(cx), overlap.other.thread);
            let text = overlap_notice(&overlap, &title, &agent, &mine.root, now);
            self.notice(text, false, cx);
            let Some(other) = app.read(cx).existing_session(overlap.other.thread) else {
                continue;
            };
            let viewer = overlap.other.root.clone();
            let mirrored = Overlap {
                other: mine,
                shared: overlap.shared,
                differing: overlap.differing,
            };
            let text = overlap_notice(
                &mirrored,
                &self.thread.title,
                self.thread.provider.label(),
                &viewer,
                now,
            );
            other.update(cx, |other, cx| other.notice(text, false, cx));
        }
    }

    /// What other agents changed in this repository lately, for the prompt.
    pub(super) fn other_agents(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let dir = self.working_dir();
        if self
            .intent_repo
            .as_ref()
            .is_none_or(|(known, _, _)| *known != dir)
        {
            let repo = elyra_git::common_dir(&dir).ok()?;
            let root = elyra_git::repo_root(&dir).ok()?;
            let root = root.canonicalize().unwrap_or(root);
            self.intent_repo = Some((dir, repo, root));
        }
        let (_, repo, root) = self.intent_repo.as_ref()?;
        let app = self.app.upgrade()?;
        let rows = crate::intent_board::rows_for(
            app.read(cx),
            repo,
            root,
            Some(self.thread.id),
            Utc::now(),
        );
        crate::intent_board::context(&rows)
    }
}
