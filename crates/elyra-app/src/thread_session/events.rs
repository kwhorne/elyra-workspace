//! What the agent sends while a turn runs, what it cost, and the end of the
//! turn.

use super::*;

impl ThreadSession {
    pub(super) fn handle_event(&mut self, event: ProviderEvent, cx: &mut Context<Self>) {
        match event {
            ProviderEvent::SessionStarted { session_id, .. } => {
                self.fork_pending = false;
                if self.thread.provider_session_id.as_deref() != Some(session_id.as_str()) {
                    self.thread.provider_session_id = Some(session_id);
                    self.save_thread(cx);
                }
            }
            ProviderEvent::TextDelta(text) => {
                self.activity = None;
                self.streaming_text.push_str(&text);
                cx.emit(SessionEvent::Streamed);
            }
            ProviderEvent::ThinkingDelta(text) => {
                self.streaming_thinking.push_str(&text);
                cx.emit(SessionEvent::Streamed);
            }
            ProviderEvent::AssistantText {
                text,
                parent_tool_use_id,
            } => {
                if parent_tool_use_id.is_none() {
                    self.streaming_text.clear();
                }
                self.append(
                    ItemContent::Assistant {
                        text,
                        parent_tool_use_id,
                    },
                    cx,
                );
            }
            ProviderEvent::Thinking { text } => {
                let text = if text.is_empty() {
                    std::mem::take(&mut self.streaming_thinking)
                } else {
                    self.streaming_thinking.clear();
                    text
                };
                if !text.trim().is_empty() {
                    self.append(ItemContent::Thinking { text }, cx);
                }
            }
            ProviderEvent::ToolUse {
                tool_use_id,
                name,
                input,
                parent_tool_use_id,
            } => {
                if parent_tool_use_id.is_none() {
                    self.streaming_text.clear();
                }
                self.append(
                    ItemContent::ToolUse {
                        tool_use_id,
                        name,
                        input,
                        parent_tool_use_id,
                    },
                    cx,
                );
            }
            ProviderEvent::ToolProgress {
                tool_use_id,
                output,
            } => {
                self.tool_progress.insert(tool_use_id, output);
                cx.emit(SessionEvent::Streamed);
            }
            ProviderEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                self.tool_progress.remove(&tool_use_id);
                let (mut edit, mut stuck) = (None, None);
                if let Some((name, input, parent)) =
                    self.items
                        .iter()
                        .rev()
                        .find_map(|item| match &item.content {
                            ItemContent::ToolUse {
                                tool_use_id: id,
                                name,
                                input,
                                parent_tool_use_id,
                            } if *id == tool_use_id => Some((name, input, parent_tool_use_id)),
                            _ => None,
                        })
                {
                    if !is_error {
                        edit = crate::intent_board::edited_file(name, input);
                    }
                    // Red in a duel runs tests that fail on purpose.
                    if !self.skip_checks {
                        stuck = self.thrash.observe(&crate::thrash_guard::Call {
                            name,
                            input,
                            parent: parent.as_deref(),
                            is_error,
                            output: &content,
                        });
                    }
                }
                if let Some(edit) = edit {
                    self.publish_changes(vec![edit], cx);
                }
                self.append(
                    ItemContent::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    },
                    cx,
                );
                if let Some(stuck) = stuck {
                    self.on_stuck(stuck, cx);
                }
            }
            ProviderEvent::PermissionRequest(request) => {
                let plan =
                    (request.tool_name == "ExitPlanMode").then(|| request.request_id.clone());
                self.append(
                    ItemContent::Approval {
                        request_id: request.request_id,
                        tool_name: request.tool_name,
                        description: request.description,
                        input: request.input,
                        decision: None,
                    },
                    cx,
                );
                self.thread.last_activity_at = Some(Utc::now());
                self.set_status(ThreadStatus::NeedsApproval, cx);
                self.save_thread(cx);
                cx.emit(SessionEvent::NeedsAttention);
                // Settings → Cross-examine every plan with.
                let examiner = ProviderKind::parse(
                    &crate::preferences::Preferences::global(cx).cross_examine_with,
                );
                if let (Some(request_id), Some(examiner)) = (plan, examiner)
                    && examiner != self.thread.provider
                    && self.thread.parent_id.is_none()
                {
                    self.cross_examine(&request_id, examiner, cx);
                }
            }
            ProviderEvent::Question(request) => {
                self.append(
                    ItemContent::Question {
                        request,
                        answer: None,
                    },
                    cx,
                );
                self.thread.last_activity_at = Some(Utc::now());
                self.set_status(ThreadStatus::NeedsApproval, cx);
                self.save_thread(cx);
                cx.emit(SessionEvent::NeedsAttention);
            }
            ProviderEvent::Usage(usage) => {
                self.merge_usage(usage);
                cx.notify();
            }
            ProviderEvent::TurnCompleted {
                is_error,
                duration_ms,
                cost_usd,
                message,
            } => self.complete_turn(is_error, duration_ms, cost_usd, message, cx),
            ProviderEvent::Commands(commands) => {
                self.commands = commands;
                cx.notify();
            }
            ProviderEvent::Agents(agents) => {
                self.agents = agents;
                cx.notify();
            }
            ProviderEvent::Models(models) => {
                if let Ok(json) = serde_json::to_string(&models) {
                    let key = format!("models.{}", self.thread.provider.as_str());
                    self.set_setting(&key, &json, cx);
                }
                self.models = models;
                cx.notify();
            }
            ProviderEvent::QueueChanged(queue) => {
                self.provider_queue = queue;
                cx.notify();
            }
            ProviderEvent::Compacting(active) => {
                self.activity = active.then(|| "Compacting conversation…".to_string());
                cx.notify();
            }
            ProviderEvent::Retrying {
                attempt,
                max_attempts,
                error,
            } => {
                let reason = error
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>();
                self.activity = Some(format!("Retrying ({attempt}/{max_attempts}) — {reason}"));
                cx.notify();
            }
            ProviderEvent::RateLimited { resets_at } => {
                let when = resets_at
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format(" until %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                self.notice(format!("Usage limit reached{when}."), true, cx);
            }
            ProviderEvent::Notice { text, is_error } => self.notice(text, is_error, cx),
            ProviderEvent::Warning(text) => self.notice(text, false, cx),
            ProviderEvent::Exited { error } => {
                self.provider = None;
                self._events = None;
                self.needs_restart = false;
                let was_running = self.running;
                self.set_running(false, cx);
                self.activity = None;
                self.streaming_text.clear();
                self.streaming_thinking.clear();
                self.tool_progress.clear();
                if let Some(error) = error {
                    self.notice(error, true, cx);
                    self.set_status(ThreadStatus::Failed, cx);
                } else if was_running {
                    self.set_status(ThreadStatus::Idle, cx);
                }
                if was_running {
                    cx.emit(SessionEvent::TurnCompleted);
                }
                if let Some(reason) = self.start_over.take().filter(|_| was_running) {
                    self.start_over_now(reason, cx);
                }
                cx.notify();
            }
        }
    }

    pub(super) fn merge_usage(&mut self, usage: Usage) {
        if usage.context_tokens.is_some() {
            self.usage.context_tokens = usage.context_tokens;
        }
        if usage.context_window.is_some() {
            self.usage.context_window = usage.context_window;
        }
        if usage.cost_usd.is_some() {
            self.usage.cost_usd = usage.cost_usd;
        }
        if usage.input_tokens + usage.output_tokens > 0 {
            self.usage.input_tokens = usage.input_tokens;
            self.usage.output_tokens = usage.output_tokens;
            self.usage.cache_read_tokens = usage.cache_read_tokens;
            self.usage.cache_write_tokens = usage.cache_write_tokens;
        }
    }

    pub(super) fn complete_turn(
        &mut self,
        is_error: bool,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let spent_before = self.spent_usd();
        self.activity = None;
        self.streaming_text.clear();
        self.streaming_thinking.clear();
        self.tool_progress.clear();
        if let Some(message) = message.filter(|_| is_error) {
            self.notice(message, true, cx);
        }
        self.append(
            ItemContent::TurnSummary {
                duration_ms,
                cost_usd,
                is_error,
                context_tokens: self.usage.context_tokens,
                context_window: self.usage.context_window,
                after: None,
            },
            cx,
        );
        self.thread.last_activity_at = Some(Utc::now());
        self.record_line_origins(cx);
        // Done means green: the turn ends when the project's checks and
        // journeys pass.
        let command = self.check_command();
        // Once a thread has seen invariants, a turn that removes them is
        // still checked against them.
        self.invariants_seen |= crate::invariants::exist(&self.working_dir());
        let invariants = !self.skip_checks && self.invariants_seen;
        if !is_error
            && !self.stopped
            && (command.is_some() || invariants || self.journeys_after_turn(cx))
        {
            self.run_checks(command, spent_before, cx);
            return;
        }
        self.finish_turn(is_error, spent_before, cx);
    }

    /// The end of a turn (after its checks): idle, notify, next message.
    pub(super) fn finish_turn(
        &mut self,
        is_error: bool,
        spent_before: f64,
        cx: &mut Context<Self>,
    ) {
        if let Some((model, effort)) = self.escalated_from.take() {
            self.apply_model_effort(model, effort, cx);
        }
        // Stopped because it went in circles: try again, still running.
        if let Some(reason) = self.start_over.take() {
            if self.refuse_at_budget(cx) {
                self.notice("So the agent isn't started over.", false, cx);
            } else {
                self.start_over_now(reason, cx);
                return;
            }
        }
        self.set_running(false, cx);
        self.activity = None;
        self.set_status(
            if is_error {
                ThreadStatus::Failed
            } else {
                ThreadStatus::Idle
            },
            cx,
        );
        self.save_thread(cx);
        if let Some(provider) = &self.provider {
            let _ = provider.refresh_usage();
        }
        self.check_budget(spent_before, cx);
        // A failed turn keeps them for the next one.
        if !is_error {
            self.replay_requests(cx);
            self.check_kept_test(cx);
            self.offer_keep_as_test(cx);
            self.test_the_tests(cx);
        }
        cx.emit(SessionEvent::TurnCompleted);
        // Deliver the next queued message.
        if !self.queued.is_empty() {
            let prompt = self.queued.remove(0);
            self.send(prompt, cx);
            return;
        }
        self.continue_goal(is_error, cx);
    }
}
