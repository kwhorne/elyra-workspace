//! Threads made from other threads: export, fork and hand off, best-of-N
//! races, duels, second opinions, reviews and side chats.

use super::*;

impl Workspace {
    /// Save a thread as a ZIP chosen by the user.
    pub(crate) fn export_thread(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(title) = self.app.read(cx).thread(id).map(|t| t.title.clone()) else {
            return;
        };
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let path = cx.prompt_for_new_path(&dir, Some(&crate::export::file_name(&title)));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                match crate::export::export_thread(this.app.read(cx), id, &path) {
                    Ok(()) => window.push_notification(
                        Notification::success(format!("Exported to {}", path.display()))
                            .on_click(move |_, _, cx| cx.reveal_path(&path)),
                        cx,
                    ),
                    Err(err) => window.push_notification(
                        Notification::error(format!("Export failed: {err:#}")),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    pub(super) fn on_export_thread(
        &mut self,
        _: &ExportThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.active {
            self.export_thread(id, window, cx);
        }
    }

    /// New thread in a project with a prepared (unsent) message.
    pub(crate) fn start_thread_with_draft(
        &mut self,
        project_id: ProjectId,
        draft: String,
        use_worktree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_thread_in(project_id, window, cx);
        let Some(id) = self.active else {
            return;
        };
        if let Some(session) = self.app.read(cx).existing_session(id) {
            session.update(cx, |session, cx| {
                session.use_worktree = use_worktree;
                cx.notify();
            });
        }
        if let Some(view) = self.thread_views.get(&id) {
            view.update(cx, |view, cx| view.append_to_composer(&draft, window, cx));
        }
    }

    pub(super) fn on_commit_and_push(
        &mut self,
        _: &CommitAndPush,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.active else {
            return;
        };
        self.show_right_tab(RightTab::Changes, window, cx);
        if let Some(changes) = self.changes_views.get(&id) {
            changes.update(cx, |changes, cx| changes.commit_and_push(window, cx));
        }
    }

    pub(super) fn on_new_chat(&mut self, _: &NewChat, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat(window, cx);
    }

    pub(crate) fn fork_thread(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.app.update(cx, |app, cx| app.fork_thread(id, cx)) {
            Ok(fork) => self.activate(fork, window, cx),
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }

    /// Continue a thread with another provider: a new thread with a recap
    /// ready to send.
    pub(crate) fn handoff_thread(
        &mut self,
        id: ThreadId,
        provider: elyra_core::ProviderKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self
            .app
            .update(cx, |app, cx| app.handoff_thread(id, provider, cx))
        {
            Ok((new_id, draft)) => {
                self.activate(new_id, window, cx);
                if let Some(view) = self.thread_views.get(&new_id) {
                    view.update(cx, |view, cx| view.append_to_composer(&draft, window, cx));
                }
            }
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }

    pub(super) fn on_import_thread(
        &mut self,
        _: &ImportThreads,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::dialogs::import_claude_sessions(cx.weak_entity(), self.app.clone(), window, cx);
    }

    pub(super) fn on_fork_thread(
        &mut self,
        _: &ForkThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.active {
            self.fork_thread(id, window, cx);
        }
    }

    pub(super) fn on_todays_work(
        &mut self,
        _: &TodaysWork,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let summary = cx.new(|cx| crate::day_summary::DaySummary::new(app, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Today's work")
                .w(px(640.))
                .child(summary.clone())
        });
    }

    // ---- best of N --------------------------------------------------------

    pub(super) fn on_best_of_n(
        &mut self,
        _: &BestOfN,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self
            .active
            .and_then(|id| self.app.read(cx).thread(id).map(|t| t.project_id))
            .or(self.current_project);
        match project {
            Some(project) => crate::race::start_dialog(cx.weak_entity(), project, window, cx),
            None => window.push_notification(
                Notification::info("Open a project first: best of N runs in a project's folder."),
                cx,
            ),
        }
    }

    pub(super) fn on_new_duel(&mut self, _: &NewDuel, window: &mut Window, cx: &mut Context<Self>) {
        let project = self
            .active
            .and_then(|id| self.app.read(cx).thread(id).map(|t| t.project_id))
            .or(self.current_project);
        match project {
            Some(project) => crate::duel::start_dialog(self.app.clone(), project, window, cx),
            None => window.push_notification(
                Notification::info("Open a project first: a duel runs in a project's folder."),
                cx,
            ),
        }
    }

    /// Give `prompt` to each provider in a new thread with its own worktree.
    pub(crate) fn start_race(
        &mut self,
        project: ProjectId,
        prompt: String,
        providers: Vec<elyra_core::ProviderKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let race = uuid::Uuid::new_v4();
        let short: String = prompt
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(40)
            .collect();
        for provider in providers {
            let thread = match self
                .app
                .update(cx, |app, cx| app.create_thread(project, cx))
            {
                Ok(thread) => thread,
                Err(err) => {
                    window.push_notification(Notification::error(format!("{err:#}")), cx);
                    continue;
                }
            };
            let Some(session) = self.app.update(cx, |app, cx| app.session(thread.id, cx)) else {
                continue;
            };
            let prompt = prompt.clone();
            session.update(cx, |session, cx| {
                session.set_provider(provider, cx);
                session.thread.race_id = Some(race);
                session.use_worktree = true;
                session.rename(format!("{short} · {}", provider.label()), cx);
                session.submit(elyra_provider::Prompt::text(prompt), cx);
            });
        }
        self.show_race(race, window, cx);
    }

    /// Open the comparison for a best-of-N run.
    pub(crate) fn show_race(
        &mut self,
        race: uuid::Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = match &self.race {
            Some(view) => view.clone(),
            None => {
                let (app, workspace) = (self.app.clone(), cx.weak_entity());
                let view = cx.new(|cx| crate::race::RaceView::new(app, workspace, cx));
                self.race = Some(view.clone());
                view
            }
        };
        view.update(cx, |view, cx| view.set_race(race, cx));
        if self.panel != Some(Panel::Race) {
            self.toggle_panel(Panel::Race, window, cx);
        }
    }

    // ---- second opinion --------------------------------------------------

    /// Have another provider review what the thread's last turn changed, in a
    /// read-only side chat.
    pub(crate) fn second_opinion(
        &mut self,
        id: ThreadId,
        provider: elyra_core::ProviderKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.app.update(cx, |app, cx| app.session(id, cx)) else {
            return;
        };
        let (root, from, request, author) = {
            let session = session.read(cx);
            (
                session.working_dir(),
                session
                    .turn_checkpoints()
                    .last()
                    .map(|(_, sha)| sha.clone()),
                session.last_request().unwrap_or_default(),
                session.thread.provider,
            )
        };
        let Some(from) = from else {
            window.push_notification(
                Notification::info(
                    "Nothing to review yet: the thread has no turn with a checkpoint.",
                ),
                cx,
            );
            return;
        };
        let job = cx.background_executor().spawn(async move {
            let to = elyra_git::checkpoint::snapshot(&root, "elyra: second opinion")?;
            elyra_git::diff_text(&root, &from, &to)
        });
        cx.spawn_in(window, async move |this, cx| {
            let diff = job.await;
            let _ = this.update_in(cx, |this, window, cx| match diff {
                Ok(diff) if diff.trim().is_empty() => window.push_notification(
                    Notification::info(
                        "The last turn changed no files, so there is nothing to review.",
                    ),
                    cx,
                ),
                Ok(diff) => this.start_review(id, provider, author, &request, &diff, window, cx),
                Err(err) => window.push_notification(
                    Notification::error(format!("Could not read the changes: {err:#}")),
                    cx,
                ),
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_review(
        &mut self,
        id: ThreadId,
        provider: elyra_core::ProviderKind,
        author: elyra_core::ProviderKind,
        request: &str,
        diff: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = match self
            .app
            .update(cx, |app, cx| app.create_review_chat(id, provider, cx))
        {
            Ok(session) => session,
            Err(err) => {
                window.push_notification(Notification::error(format!("{err:#}")), cx);
                return;
            }
        };
        let mut prompt = review_prompt(author.label(), request, diff);
        let (project, dir) = {
            let session = session.read(cx);
            (session.project.id, session.project.path.clone())
        };
        if let Some(hint) =
            crate::stack::tools_hint(&self.app.read(cx).store, project, &dir, provider)
        {
            prompt.push_str(&format!("\n\n{hint}"));
        }
        session.update(cx, |session, cx| {
            session.submit(elyra_provider::Prompt::text(prompt), cx)
        });
        let review = session.read(cx).thread.id;
        if self.thread_view(review, window, cx).is_some() {
            self.show_right_tab(RightTab::SideChat, window, cx);
        }
    }

    /// Put a side chat's last reply in its parent thread's composer.
    pub(super) fn send_side_reply(
        &mut self,
        side: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.app.update(cx, |app, cx| app.session(side, cx)) else {
            return;
        };
        let (reply, parent, title) = {
            let session = session.read(cx);
            (
                session.last_reply(),
                session.thread.parent_id,
                session.thread.title.clone(),
            )
        };
        let (Some(parent), false) = (parent, reply.trim().is_empty()) else {
            return;
        };
        let text = if title.starts_with("Second opinion") {
            format!(
                "Another agent reviewed your last changes. Fix what you agree with and say why you disagree with the rest:\n\n{}",
                reply.trim()
            )
        } else {
            reply.trim().to_string()
        };
        if let Some(view) = self.thread_view(parent, window, cx) {
            view.update(cx, |view, cx| {
                view.append_to_composer(&text, window, cx);
                view.focus_composer(window, cx);
            });
        }
    }

    // ---- side chats -----------------------------------------------------

    pub(crate) fn side_chat_of(&self, parent: ThreadId, cx: &App) -> Option<ThreadId> {
        self.app
            .read(cx)
            .threads
            .iter()
            .filter(|t| t.parent_id == Some(parent))
            .max_by_key(|t| t.created_at)
            .map(|t| t.id)
    }

    /// Open a side chat that was just made in the side chat tab.
    pub(crate) fn show_side_chat(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thread_view(id, window, cx).is_some() {
            self.show_right_tab(RightTab::SideChat, window, cx);
        }
    }

    pub(super) fn on_new_side_chat(
        &mut self,
        _: &NewSideChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self.active else {
            return;
        };
        match self
            .app
            .update(cx, |app, cx| app.create_side_chat(parent, cx))
        {
            Ok(session) => {
                let id = session.read(cx).thread.id;
                if let Some(view) = self.thread_view(id, window, cx) {
                    self.show_right_tab(RightTab::SideChat, window, cx);
                    view.update(cx, |view, cx| view.focus_composer(window, cx));
                }
            }
            Err(err) => window.push_notification(Notification::error(format!("{err:#}")), cx),
        }
    }

    /// Turn a side chat into a regular thread with its own tab.
    pub(super) fn promote_side_chat(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.app.update(cx, |app, cx| {
            app.update_thread(id, |t| t.parent_id = None, cx)
        });
        self.activate(id, window, cx);
    }

    pub(super) fn close_side_chat(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.thread_views.remove(&id);
        if let Err(err) = self
            .app
            .update(cx, |app, cx| app.delete_thread(id, false, cx))
        {
            window.push_notification(Notification::error(format!("{err:#}")), cx);
        }
        cx.notify();
    }

    // ---- rendering ------------------------------------------------------
}
