//! Cross-examining a plan: another agent's objections, in a read-only side
//! chat, before any code is written (see `cross_examine`).

use super::*;

impl ThreadSession {
    /// Have `examiner` cross-examine the pending plan `request_id` in a
    /// read-only side chat; its objections come back as a card.
    pub fn cross_examine(
        &mut self,
        request_id: &str,
        examiner: ProviderKind,
        cx: &mut Context<Self>,
    ) {
        let Some(plan) = self.items.iter().find_map(|item| match &item.content {
            ItemContent::Approval {
                request_id: id,
                tool_name,
                input,
                decision: None,
                ..
            } if id == request_id && tool_name == "ExitPlanMode" => {
                input["plan"].as_str().map(str::to_string)
            }
            _ => None,
        }) else {
            self.notice("There is no plan waiting to be cross-examined.", true, cx);
            return;
        };
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let parent = self.thread.id;
        let title = format!("Cross-examination · {}", examiner.label());
        let chat = match app.update(cx, |app, cx| {
            app.create_read_only_chat(parent, examiner, title, cx)
        }) {
            Ok(chat) => chat,
            Err(err) => {
                self.notice(
                    format!("Could not start the cross-examination: {err:#}"),
                    true,
                    cx,
                );
                return;
            }
        };
        self.append(
            ItemContent::CrossExamination {
                request_id: request_id.to_string(),
                examiner,
                chat: chat.read(cx).thread.id,
                objections: Vec::new(),
                state: ExamState::Running,
            },
            cx,
        );
        let Some(card) = self.items.last().map(|item| item.id) else {
            return;
        };
        let subscription = cx.subscribe(&chat, move |this, chat, event: &SessionEvent, cx| {
            if matches!(event, SessionEvent::TurnCompleted) {
                this.examined(card, &chat, cx);
            }
        });
        self.examinations.insert(card, subscription);
        let request = self.last_request().unwrap_or_default();
        let author = self.thread.provider.label();
        let (dir, project) = (self.project.path.clone(), self.project.id);
        let hint = crate::stack::tools_hint(&app.read(cx).store, project, &dir, examiner);
        let lookup = format!("{request}\n{plan}");
        let job = cx.background_executor().spawn(async move {
            crate::warm_start::build(&dir, project, parent, &lookup).map(|pack| pack.text)
        });
        cx.spawn(async move |_, cx| {
            let orientation = job.await;
            let mut prompt =
                crate::cross_examine::prompt(author, &request, &plan, orientation.as_deref());
            if let Some(hint) = hint {
                prompt.push_str(&format!("\n\n{hint}"));
            }
            chat.update(cx, |chat, cx| chat.submit(Prompt::text(prompt), cx));
        })
        .detach();
    }

    /// The examiner answered: read its objections into the card.
    pub(super) fn examined(
        &mut self,
        card: ItemId,
        chat: &gpui_kit::Entity<ThreadSession>,
        cx: &mut Context<Self>,
    ) {
        self.examinations.remove(&card);
        let (reply, failed) = {
            let chat = chat.read(cx);
            (
                chat.last_reply(),
                chat.thread.status == ThreadStatus::Failed,
            )
        };
        let Some(index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        let ItemContent::CrossExamination {
            objections, state, ..
        } = &mut self.items[index].content
        else {
            return;
        };
        match crate::cross_examine::parse(&reply) {
            Some(found) => {
                *objections = found;
                *state = ExamState::Done;
            }
            None => {
                *state = ExamState::Failed {
                    reason: if failed {
                        "The examiner stopped with an error.".into()
                    } else {
                        "The examiner's answer had no list of objections; open its chat to read it."
                            .into()
                    },
                };
            }
        }
        self.persist_item(index, cx);
        cx.emit(SessionEvent::NeedsAttention);
        cx.notify();
    }

    /// Take an objection into the plan, or keep the plan on it (again: undecided).
    pub fn set_objection(&mut self, card: ItemId, at: usize, taken: bool, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        if let ItemContent::CrossExamination {
            objections,
            state: ExamState::Done,
            ..
        } = &mut self.items[index].content
            && let Some(objection) = objections.get_mut(at)
        {
            objection.taken = (objection.taken != Some(taken)).then_some(taken);
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// Whether the plan a cross-examination is about still waits for an answer.
    pub fn plan_pending(&self, request_id: &str) -> bool {
        self.items.iter().any(|item| {
            matches!(&item.content, ItemContent::Approval { request_id: id, decision: None, .. } if id == request_id)
        })
    }

    /// Send the planning agent back with the objections taken.
    pub fn revise_plan(&mut self, card: ItemId, cx: &mut Context<Self>) {
        let Some(card_index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        let ItemContent::CrossExamination {
            request_id,
            examiner,
            objections,
            state: ExamState::Done,
            ..
        } = &self.items[card_index].content
        else {
            return;
        };
        let Some(message) = crate::cross_examine::revision(examiner.label(), objections) else {
            return;
        };
        let request_id = request_id.clone();
        let Some(approval) = self.items.iter().position(|item| {
            matches!(&item.content, ItemContent::Approval { request_id: id, decision: None, .. } if *id == request_id)
        }) else {
            self.notice("The plan was already answered.", true, cx);
            return;
        };
        // Claude Code reads the reason with the refusal; other agents get it
        // as the next message.
        let inline = self.thread.provider == ProviderKind::Claude;
        self.respond_with(
            approval,
            PermissionResponse::Deny {
                message: message.clone(),
            },
            ApprovalDecision::Denied,
            cx,
        );
        if let ItemContent::CrossExamination { state, .. } = &mut self.items[card_index].content {
            *state = ExamState::Sent;
        }
        self.persist_item(card_index, cx);
        if !inline {
            self.submit(Prompt::text(message), cx);
        }
        cx.notify();
    }
}
