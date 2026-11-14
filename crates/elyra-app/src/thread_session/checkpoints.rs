//! Going back: the checkpoints before each turn and the database
//! snapshots with them, restoring, undoing an instruction, and finding the turn
//! that broke something.

use super::*;

impl ThreadSession {
    /// Database snapshots taken with this thread's checkpoints, oldest first:
    /// (checkpoint commit, snapshot reference).
    pub(super) fn db_snapshots(&self, cx: &App) -> Vec<(String, String)> {
        let Some(app) = self.app.upgrade() else {
            return Vec::new();
        };
        db_snapshots(&app.read(cx).store, self.thread.id)
    }

    /// Keep a checkpoint's database snapshot; drop the oldest beyond the limit.
    pub(super) fn remember_db_snapshot(
        &mut self,
        sha: &str,
        reference: String,
        cx: &mut Context<Self>,
    ) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let mut snapshots = self.db_snapshots(cx);
        snapshots.push((sha.to_string(), reference));
        let excess = snapshots.len().saturating_sub(MAX_DB_SNAPSHOTS);
        let dropped: Vec<String> = snapshots.drain(..excess).map(|(_, r)| r).collect();
        let key = db_snapshots_key(self.thread.id);
        if let Ok(json) = serde_json::to_string(&snapshots)
            && let Err(err) = app.read(cx).store.set_setting(&key, &json)
        {
            log::warn!("saving database snapshots: {err:#}");
        }
        if !dropped.is_empty() {
            cx.background_executor()
                .spawn(async move {
                    for reference in dropped {
                        crate::grove::drop_snapshot(&reference);
                    }
                })
                .detach();
        }
    }

    /// Checkpoints of this thread's turns, oldest first: (label, commit).
    pub fn turn_checkpoints(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::User {
                    text,
                    checkpoint: Some(sha),
                    ..
                } => Some((
                    text.lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(60)
                        .collect::<String>(),
                    sha.clone(),
                )),
                _ => None,
            })
            .enumerate()
            .map(|(index, (text, sha))| (format!("Turn {} · {text}", index + 1), sha))
            .collect()
    }

    /// Put the working tree back to how it was before a message was sent.
    pub fn restore_checkpoint(&mut self, sha: String, cx: &mut Context<Self>) {
        if self.running {
            self.notice("Stop the agent before restoring files.", true, cx);
            return;
        }
        let job = self.restore_job(sha, cx);
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(None) => this.notice(
                        "Files restored to how they were before that message. The conversation is unchanged.",
                        false,
                        cx,
                    ),
                    Ok(Some(Ok(()))) => this.notice(
                        "Files and database restored to how they were before that message. The conversation is unchanged.",
                        false,
                        cx,
                    ),
                    Ok(Some(Err(err))) => this.notice(
                        format!("Files restored, but not the database: {err:#}"),
                        true,
                        cx,
                    ),
                    Err(err) => this.notice(format!("Could not restore files: {err:#}"), true, cx),
                }
                cx.emit(SessionEvent::TurnCompleted);
            });
        })
        .detach();
    }

    /// Put the files (and the database, when the checkpoint has it) back to
    /// a checkpoint, in the background: the database's result, if it had one.
    pub(super) fn restore_job(
        &self,
        sha: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Option<anyhow::Result<()>>>> {
        let cwd = self.working_dir();
        let snapshot = self
            .db_snapshots(cx)
            .into_iter()
            .find(|(s, _)| *s == sha)
            .map(|(_, r)| r);
        cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&cwd)?;
            elyra_git::checkpoint::restore(&root, &sha)?;
            let database = snapshot.and_then(|reference| {
                let database = crate::grove::database_of(&cwd)?;
                Some(crate::grove::restore_database(&database, &reference))
            });
            anyhow::Ok(database)
        })
    }

    /// Take back the instruction `item` (a message from the user): what its
    /// turn and automatic fixes changed is undone, later work stays. The
    /// result is added as an entry.
    pub fn undo_instruction(&mut self, item: ItemId, cx: &mut Context<Self>) {
        if self.running {
            self.notice("Wait for the turn to finish before undoing.", false, cx);
            return;
        }
        let Some(span) = crate::undo::span(&self.items, item) else {
            self.notice("That message has no checkpoint to undo from.", true, cx);
            return;
        };
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let dir = self.working_dir();
        let threads = crate::bisect::threads_in(app.read(cx), &dir);
        let n = self.items.len();
        let refs = (
            format!("refs/elyra/{}/{n}-undo-before", self.thread.id.simple()),
            format!("refs/elyra/{}/{n}-undo-after", self.thread.id.simple()),
        );
        self.activity = Some("Undoing…".into());
        cx.notify();
        let instruction = span.instruction.clone();
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir)?;
            // A turn from before its end was kept: up to the folder's next
            // instruction.
            let next = if span.after.is_none() {
                let store = elyra_core::Store::open_read_only(&elyra_core::paths::database_path())?;
                crate::bisect::timeline(&store, &threads)?
                    .into_iter()
                    .find(|step| step.at > span.at)
                    .map(|step| step.checkpoint)
            } else {
                None
            };
            crate::undo::undo(&root, &span, next, (&refs.0, &refs.1))
        });
        cx.spawn(async move |this, cx| {
            let done = job.await;
            let _ = this.update(cx, |this, cx| {
                this.activity = None;
                match done {
                    Ok(done) => this.append(
                        ItemContent::Undo {
                            item,
                            instruction,
                            reverted: done.reverted,
                            kept: done.kept,
                            before_undo: done.before_undo,
                            after_undo: done.after_undo,
                            put_back: false,
                        },
                        cx,
                    ),
                    Err(err) => this.notice(format!("Could not undo: {err:#}"), true, cx),
                }
                cx.emit(SessionEvent::FilesChanged);
                cx.notify();
            });
        })
        .detach();
    }

    /// Put back an undo (the entry `undo`): its changes return, what
    /// changed since stays.
    pub fn put_back_undo(&mut self, undo: ItemId, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|i| i.id == undo) else {
            return;
        };
        let ItemContent::Undo {
            before_undo,
            after_undo,
            put_back: false,
            ..
        } = &self.items[index].content
        else {
            return;
        };
        let (before, after) = (before_undo.clone(), after_undo.clone());
        let dir = self.working_dir();
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir)?;
            crate::undo::put_back(&root, &before, &after)
        });
        cx.spawn(async move |this, cx| {
            let applied = job.await;
            let _ = this.update(cx, |this, cx| {
                match applied {
                    Ok(applied) => {
                        if let Some(ItemContent::Undo { put_back, .. }) =
                            this.items.get_mut(index).map(|i| &mut i.content)
                        {
                            *put_back = true;
                        }
                        this.persist_item(index, cx);
                        if !applied.kept.is_empty() {
                            this.notice(
                                format!(
                                    "Put back, except {}: changed since in ways that don't merge.",
                                    applied.kept.join(", ")
                                ),
                                false,
                                cx,
                            );
                        }
                    }
                    Err(err) => this.notice(format!("Could not put it back: {err:#}"), true, cx),
                }
                cx.emit(SessionEvent::FilesChanged);
                cx.notify();
            });
        })
        .detach();
    }

    /// Find the instruction whose turn made the checks fail ("Who broke
    /// it?"): `command`, or the project's check command, runs against the
    /// earlier states of the folder in a temporary checkout. The result is
    /// added as an entry; Stop ends it.
    pub fn find_breaking_change(&mut self, command: Option<String>, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let Some(command) = command
            .or_else(|| self.project.check_command.clone())
            .filter(|c| !c.trim().is_empty())
        else {
            self.notice(
                "Set the project's check command (Context tab) to find what broke it.",
                true,
                cx,
            );
            return;
        };
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let dir = self.working_dir();
        let threads = crate::bisect::threads_in(app.read(cx), &dir);
        let cancel = Arc::new(AtomicBool::new(false));
        self.check_cancel = Some(cancel.clone());
        self.stopped = false;
        self.set_running(true, cx);
        self.activity = Some("Finding the turn that broke the checks…".into());
        cx.notify();
        let (progress, updates) = async_channel::unbounded::<String>();
        cx.spawn(async move |this, cx| {
            while let Ok(update) = updates.recv().await {
                let alive = this
                    .update(cx, |this, cx| {
                        this.activity = Some(update);
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        let run_command = command.clone();
        let job = cx.background_executor().spawn(async move {
            let found = (|| {
                let store = elyra_core::Store::open_read_only(&elyra_core::paths::database_path())?;
                let steps = crate::bisect::timeline(&store, &threads)?;
                let root = elyra_git::repo_root(&dir)?;
                anyhow::Ok(crate::bisect::find(
                    &root,
                    &dir,
                    &run_command,
                    &steps,
                    &cancel,
                    |text| {
                        let _ = progress.try_send(text);
                    },
                ))
            })();
            found.unwrap_or_else(|err| elyra_core::BisectVerdict::Failed {
                error: format!("{err:#}"),
            })
        });
        cx.spawn(async move |this, cx| {
            let verdict = job.await;
            let _ = this.update(cx, |this, cx| {
                this.check_cancel = None;
                this.activity = None;
                this.set_running(false, cx);
                if this.stopped {
                    this.notice("Stopped looking for what broke the checks.", false, cx);
                } else {
                    this.append(ItemContent::Bisect { command, verdict }, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}
