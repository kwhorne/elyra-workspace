//! What the transcript's cards do: approvals, answers to questions, plans,
//! proposals, saved rules, restoring and undoing.

use super::*;

impl ThreadView {
    pub(super) fn interrupt(&mut self, _: &Interrupt, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| session.interrupt(cx));
    }

    // ---- transcript actions ---------------------------------------------------

    pub fn toggle_expanded(&mut self, id: ItemId, cx: &mut Context<Self>) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        cx.notify();
    }

    pub fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        self.session
            .update(cx, |session, cx| session.respond(request_id, decision, cx));
    }

    /// "Review the changes" on a pull request card.
    pub fn review_pr_changes(&mut self, item: ItemId, cx: &mut Context<Self>) {
        let (app, thread) = {
            let session = self.session.read(cx);
            (session.app_entity(), session.thread.id)
        };
        if let Some(app) = app {
            crate::review_follow::review_changes(&app, thread, item, cx);
        }
    }

    pub fn stop_following_pr(&mut self, item: ItemId, cx: &mut Context<Self>) {
        let (app, thread) = {
            let session = self.session.read(cx);
            (session.app_entity(), session.thread.id)
        };
        if let Some(app) = app {
            crate::review_follow::stop(app.read(cx), thread);
        }
        self.session
            .update(cx, |session, cx| session.set_pr_update_handled(item, cx));
    }

    /// Keep a proposed rule: in the repository's AGENTS.md (or CLAUDE.md), or
    /// in the project's instructions in Elyra.
    pub fn keep_rule(
        &mut self,
        item: ItemId,
        in_file: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (rule, app, dir, project) = {
            let session = self.session.read(cx);
            (
                session.proposed_rule(item),
                session.app_entity(),
                session.working_dir(),
                session.project.clone(),
            )
        };
        let (Some((rule, paths)), Some(app)) = (rule, app) else {
            return;
        };
        let note = if in_file {
            match crate::rules::add_to_file(&dir, &rule, paths.as_deref()) {
                Ok(path) => format!(
                    "Added to {}; commit it with your changes.",
                    path.strip_prefix(&dir).unwrap_or(&path).display()
                ),
                Err(err) => {
                    window.push_notification(
                        gpui_kit::component::notification::Notification::error(format!(
                            "Couldn't add the rule: {err}"
                        )),
                        cx,
                    );
                    return;
                }
            }
        } else {
            let mut project = project;
            project.instructions = Some(crate::rules::with_instruction(
                project.instructions.as_deref(),
                &rule,
            ));
            app.update(cx, |app, cx| app.update_project(project, cx));
            "Added to the project's instructions; agents get it from their next message."
                .to_string()
        };
        let outcome = if in_file {
            elyra_core::RuleOutcome::File
        } else {
            elyra_core::RuleOutcome::Instructions
        };
        self.session.update(cx, |session, cx| {
            session.set_rule_outcome(item, outcome, cx)
        });
        window.push_notification(
            gpui_kit::component::notification::Notification::success(note),
            cx,
        );
    }

    pub fn dismiss_rule(&mut self, item: ItemId, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.set_rule_outcome(item, elyra_core::RuleOutcome::Dismissed, cx)
        });
    }

    /// Create the automation the agent proposed, as it is.
    pub fn create_proposal(&mut self, item: ItemId, window: &mut Window, cx: &mut Context<Self>) {
        let (automation, app) = {
            let session = self.session.read(cx);
            (session.proposal(item), session.app_entity())
        };
        let (Some(automation), Some(app)) = (automation, app) else {
            return;
        };
        crate::automations::create(&app, &automation, cx);
        self.session.update(cx, |session, cx| {
            session.set_proposal_outcome(item, elyra_core::ProposalOutcome::Created, cx)
        });
        window.push_notification(
            gpui_kit::component::notification::Notification::success(format!(
                "\u{201c}{}\u{201d} created; it is in Automations.",
                automation.name
            )),
            cx,
        );
    }

    /// Open the proposed automation in the editor; saving creates it.
    pub fn edit_proposal(&mut self, item: ItemId, window: &mut Window, cx: &mut Context<Self>) {
        let (automation, app) = {
            let session = self.session.read(cx);
            (session.proposal(item), session.app_entity())
        };
        let (Some(automation), Some(app)) = (automation, app) else {
            return;
        };
        let session = self.session.downgrade();
        crate::automations::edit_proposal(
            app,
            automation,
            move |cx| {
                let _ = session.update(cx, |session, cx| {
                    session.set_proposal_outcome(item, elyra_core::ProposalOutcome::Created, cx)
                });
            },
            window,
            cx,
        );
    }

    pub fn dismiss_proposal(&mut self, item: ItemId, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.set_proposal_outcome(item, elyra_core::ProposalOutcome::Dismissed, cx)
        });
    }

    pub fn approve_plan(
        &mut self,
        request_id: &str,
        mode: Option<PermissionMode>,
        cx: &mut Context<Self>,
    ) {
        self.session
            .update(cx, |session, cx| session.approve_plan(request_id, mode, cx));
    }

    pub fn answer(&mut self, request_id: &str, answer: QuestionAnswer, cx: &mut Context<Self>) {
        self.forms.remove(request_id);
        self.session
            .update(cx, |session, cx| session.answer(request_id, answer, cx));
    }

    pub fn toggle_option(
        &mut self,
        request_id: &str,
        question: usize,
        option: usize,
        multi: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(selected) = self
            .forms
            .get_mut(request_id)
            .and_then(|form| form.selected.get_mut(question))
        {
            if multi {
                if !selected.remove(&option) {
                    selected.insert(option);
                }
            } else {
                selected.clear();
                selected.insert(option);
            }
        }
        cx.notify();
    }

    pub fn submit_question(&mut self, request_id: &str, cx: &mut Context<Self>) {
        let request = self
            .session
            .read(cx)
            .items
            .iter()
            .find_map(|item| match &item.content {
                ItemContent::Question {
                    request,
                    answer: None,
                } if request.request_id == request_id => Some(request.clone()),
                _ => None,
            });
        let (Some(request), Some(form)) = (request, self.forms.get(request_id)) else {
            return;
        };
        let mut answers = Vec::new();
        for (qi, question) in request.questions.iter().enumerate() {
            let mut parts: Vec<String> = question
                .options
                .iter()
                .enumerate()
                .filter(|(oi, _)| form.selected.get(qi).is_some_and(|s| s.contains(oi)))
                .map(|(_, option)| option.label.clone())
                .collect();
            if let Some(text) = form.texts.get(qi) {
                let text = text.read(cx).value().trim().to_string();
                if !text.is_empty() {
                    parts.push(text);
                }
            }
            if parts.is_empty() {
                return; // every question needs an answer
            }
            answers.push(parts.join(", "));
        }
        self.answer(request_id, QuestionAnswer::Answers { answers }, cx);
    }

    pub fn confirm_restore(&mut self, sha: String, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (session, sha) = (session.clone(), sha.clone());
            alert
                .title("Restore files?")
                .description(
                    "Files in this thread's folder go back to how they were before this message: \
                     edits since then are undone and files created since are deleted. \
                     The conversation and commits are not changed.",
                )
                .confirm()
                .ok_text("Restore files")
                .on_ok(move |_, _, cx| {
                    session.update(cx, |s, cx| s.restore_checkpoint(sha.clone(), cx));
                    true
                })
        });
    }

    pub fn confirm_undo(&mut self, item: ItemId, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let session = session.clone();
            alert
                .title("Undo this instruction?")
                .description(
                    "What this message's turn (and its automatic fixes) changed in the folder is \
                     taken back; what came after it stays. Files that changed again since are left \
                     as they are, for the agent to finish. You can put it back afterwards.",
                )
                .confirm()
                .ok_text("Undo")
                .on_ok(move |_, _, cx| {
                    session.update(cx, |s, cx| s.undo_instruction(item, cx));
                    true
                })
        });
    }

    /// Create answer state for questions that just arrived.
    pub(super) fn ensure_forms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open: Vec<_> = self
            .session
            .read(cx)
            .items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::Question {
                    request,
                    answer: None,
                } => Some(request.clone()),
                _ => None,
            })
            .collect();
        for request in open {
            if self.forms.contains_key(&request.request_id) || request.kind == QuestionKind::Confirm
            {
                continue;
            }
            let texts = request
                .questions
                .iter()
                .map(|question| {
                    let placeholder = if request.kind == QuestionKind::Text {
                        "Type your answer"
                    } else {
                        "Other…"
                    };
                    let prefill = question.prefill.clone();
                    cx.new(|cx| {
                        let mut state = InputState::new(window, cx).placeholder(placeholder);
                        if !prefill.is_empty() {
                            state.set_value(prefill, window, cx);
                        }
                        state
                    })
                })
                .collect();
            self.forms.insert(
                request.request_id.clone(),
                QuestionForm {
                    selected: vec![HashSet::new(); request.questions.len()],
                    texts,
                },
            );
        }
    }

    // ---- find in thread -------------------------------------------------------
}
