//! The user's part in a turn: approvals, plans and answers, interrupting,
//! compacting or clearing the context, and turns cut short by quitting.

use super::*;

impl ThreadSession {
    pub fn interrupt(&mut self, cx: &mut Context<Self>) {
        self.stopped = true;
        if let Some(cancel) = &self.check_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(provider) = &self.provider
            && let Err(err) = provider.interrupt()
        {
            log::warn!("interrupt failed: {err:#}");
        }
        cx.notify();
    }

    pub fn compact(&mut self, cx: &mut Context<Self>) {
        if self.running {
            self.notice(
                "Wait for the current turn to finish before compacting.",
                false,
                cx,
            );
            return;
        }
        let result = self
            .ensure_provider(cx)
            .and_then(|()| self.provider.as_ref().unwrap().compact());
        match result {
            Ok(()) => {
                if self.thread.provider == ProviderKind::Claude {
                    // Claude runs /compact as a turn.
                    self.set_running(true, cx);
                }
                self.activity = Some("Compacting conversation…".into());
            }
            Err(err) => self.notice(format!("Could not compact: {err:#}"), true, cx),
        }
        cx.notify();
    }

    /// Forget the provider conversation and start fresh in this thread.
    pub fn clear_context(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.stop_provider();
        self.thread.provider_session_id = None;
        self.usage = Usage::default();
        self.save_thread(cx);
        self.notice(crate::context_report::CLEARED, false, cx);
    }

    // ---- decisions --------------------------------------------------------

    pub fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|item| {
            matches!(&item.content, ItemContent::Approval { request_id: id, decision: None, .. } if id == request_id)
        }) else {
            return;
        };
        let response = match decision {
            ApprovalDecision::Allowed => PermissionResponse::Allow,
            ApprovalDecision::AllowedForSession => PermissionResponse::AllowForSession,
            ApprovalDecision::Denied => PermissionResponse::Deny {
                message: "The user denied this action.".into(),
            },
        };
        self.respond_with(index, response, decision, cx);
    }

    /// Answer a plan approval (ExitPlanMode) and pick the mode to continue in.
    pub fn approve_plan(
        &mut self,
        request_id: &str,
        continue_in: Option<PermissionMode>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|item| {
            matches!(&item.content, ItemContent::Approval { request_id: id, decision: None, .. } if id == request_id)
        }) else {
            return;
        };
        match continue_in {
            Some(mode) => {
                self.respond_with(
                    index,
                    PermissionResponse::Allow,
                    ApprovalDecision::Allowed,
                    cx,
                );
                self.set_permission_mode(mode, cx);
            }
            None => self.respond_with(
                index,
                PermissionResponse::Deny {
                    message: "The user wants to keep planning. Refine the plan.".into(),
                },
                ApprovalDecision::Denied,
                cx,
            ),
        }
    }

    pub(super) fn respond_with(
        &mut self,
        index: usize,
        response: PermissionResponse,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        let ItemContent::Approval { request_id, .. } = &self.items[index].content else {
            return;
        };
        let request_id = request_id.clone();
        match self.provider.as_ref() {
            Some(provider) => {
                if let Err(err) = provider.respond_permission(&request_id, response) {
                    self.notice(format!("Could not send approval: {err:#}"), true, cx);
                    return;
                }
            }
            None => {
                self.notice(
                    "The provider session has ended; this request can no longer be answered.",
                    true,
                    cx,
                );
            }
        }
        if let ItemContent::Approval { decision: slot, .. } = &mut self.items[index].content {
            *slot = Some(decision);
        }
        self.persist_item(index, cx);
        self.after_decision(cx);
    }

    pub fn answer(&mut self, request_id: &str, answer: QuestionAnswer, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| {
            matches!(&item.content, ItemContent::Question { request, answer: None } if request.request_id == request_id)
        }) else {
            return;
        };
        match self.provider.as_ref() {
            Some(provider) => {
                if let Err(err) = provider.answer_question(request_id, answer.clone()) {
                    self.notice(format!("Could not send answer: {err:#}"), true, cx);
                    return;
                }
            }
            None => self.notice(
                "The provider session has ended; this question can no longer be answered.",
                true,
                cx,
            ),
        }
        if let ItemContent::Question { answer: slot, .. } = &mut self.items[index].content {
            *slot = Some(answer);
        }
        self.persist_item(index, cx);
        self.after_decision(cx);
    }

    pub(super) fn after_decision(&mut self, cx: &mut Context<Self>) {
        if !self.pending_approval() && self.running {
            self.set_status(ThreadStatus::Running, cx);
        }
        cx.notify();
    }

    // ---- options ----------------------------------------------------------

    /// Stop the provider before the app quits. Returns whether a turn was
    /// cut off (the caller persists the interrupted status).
    pub fn stop_for_quit(&mut self) -> bool {
        let was_running = self.running;
        if let Some(cancel) = &self.check_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        self.stop_provider();
        if was_running {
            // The app is quitting: nothing to tell it.
            self.running = false;
            self.thread.status = ThreadStatus::Interrupted;
        }
        was_running
    }

    /// Continue a turn that was cut off when the app quit.
    pub fn resume_interrupted(&mut self, cx: &mut Context<Self>) {
        if self.thread.status != ThreadStatus::Interrupted || self.running {
            return;
        }
        self.set_status(ThreadStatus::Idle, cx);
        self.submit(
            Prompt::text(
                "Your previous turn was interrupted because the app quit. First check what was already done (files changed, commands that ran) so nothing is done twice, then continue where you left off.",
            ),
            cx,
        );
    }

    pub fn dismiss_interrupted(&mut self, cx: &mut Context<Self>) {
        if self.thread.status == ThreadStatus::Interrupted {
            self.set_status(ThreadStatus::Idle, cx);
            cx.notify();
        }
    }
}
