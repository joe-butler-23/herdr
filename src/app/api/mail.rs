use crate::api::schema::{
    AgentStatus, MailKind, MailListParams, MailReadParams, MailSendParams, ResponseResult,
};
use crate::app::mail_store::NewMailMessage;
use crate::app::App;
use crate::terminal::TerminalId;

use super::super::api_helpers::{encode_api_keys, encode_api_text};
use super::responses::{encode_error, encode_success};

impl App {
    /// Resolve a mail `to`/`from` string to a live `TerminalId`. Tries, in
    /// order: (1) `resolve_terminal_target` (raw terminal id | pane id via
    /// `parse_pane_id` | agent name/label — already handles ambiguity),
    /// falling back to (2) a raw `TerminalId` string match against
    /// `self.state.terminals` directly, in case the caller already has a
    /// terminal id from a prior `mail_envelope.from_terminal_id` and wants
    /// to reply to it after its pane compacted to a different public id.
    fn resolve_mail_recipient(&self, target: &str) -> Result<TerminalId, String> {
        match self.resolve_terminal_target(target) {
            Ok(resolved) => Ok(TerminalId::from_raw(resolved.terminal_id)),
            Err(_) => {
                let candidate = TerminalId::from_raw(target.to_string());
                if self.state.terminals.contains_key(&candidate) {
                    Ok(candidate)
                } else {
                    Err(target.to_string())
                }
            }
        }
    }

    pub(super) fn handle_mail_send(&mut self, id: String, params: MailSendParams) -> String {
        let to = match self.resolve_mail_recipient(&params.to) {
            Ok(terminal_id) => terminal_id,
            Err(target) => {
                return encode_error(
                    id,
                    "recipient_not_found",
                    format!("no pane/agent/terminal matches '{target}'"),
                )
            }
        };
        // from_terminal_id is NOT inferable server-side from the socket
        // alone (no calling-pane identity on this API) — it is only ever
        // populated by independently resolving the caller's self-reported
        // from_pane_id, never trusted as authoritative identity. A stale or
        // absent self-report must never fail the send.
        let from_terminal_id = params
            .from_pane_id
            .as_deref()
            .and_then(|hint| self.resolve_mail_recipient(hint).ok());

        let stored = self.state.store_mail(NewMailMessage {
            from_terminal_id: from_terminal_id.clone(),
            from_pane_id: params.from_pane_id,
            from_agent: params.from_agent,
            to_terminal_id: to.clone(),
            kind: params.kind,
            subject: params.subject,
            body: params.body,
        });
        let envelope = stored.to_envelope();

        self.emit_event(crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::MailReceived,
            data: crate::api::schema::EventData::MailReceived {
                envelope: envelope.clone(),
            },
        });

        self.maybe_nudge_after_send(&to, from_terminal_id.as_ref(), params.no_nudge);

        encode_success(id, ResponseResult::MailSent { envelope })
    }

    pub(super) fn handle_mail_list(&mut self, id: String, params: MailListParams) -> String {
        let inbox_terminal_id = match self.resolve_mail_recipient(&params.inbox) {
            Ok(terminal_id) => terminal_id,
            Err(target) => {
                return encode_error(
                    id,
                    "recipient_not_found",
                    format!("no pane/agent/terminal matches '{target}'"),
                )
            }
        };
        let sender_filter = match params.sender.as_deref() {
            Some(sender) => match self.resolve_mail_recipient(sender) {
                Ok(terminal_id) => Some(terminal_id),
                Err(target) => {
                    return encode_error(
                        id,
                        "sender_not_found",
                        format!("no pane/agent/terminal matches sender '{target}'"),
                    )
                }
            },
            None => None,
        };
        let messages = self
            .state
            .mail_inboxes
            .get(&inbox_terminal_id)
            .map(|inbox| inbox.list(params.unread_only, sender_filter.as_ref()))
            .unwrap_or_default();
        encode_success(id, ResponseResult::MailList { messages })
    }

    pub(super) fn handle_mail_read(&mut self, id: String, params: MailReadParams) -> String {
        let inbox_terminal_id = match self.resolve_mail_recipient(&params.inbox) {
            Ok(terminal_id) => terminal_id,
            Err(target) => {
                return encode_error(
                    id,
                    "recipient_not_found",
                    format!("no pane/agent/terminal matches '{target}'"),
                )
            }
        };
        let Some(inbox) = self.state.mail_inboxes.get_mut(&inbox_terminal_id) else {
            return mail_not_found(id, params.id);
        };
        let Some(message) = inbox.mark_read(params.id) else {
            return mail_not_found(id, params.id);
        };
        let message = message.to_message();

        // If the agent checked its mail on its own and nothing unread
        // remains, cancel any pending nudge — there's nothing left to
        // announce, and typing a stale nudge line into a pane that just
        // drained its own inbox would be confusing noise.
        let inbox_now_fully_read = self
            .state
            .mail_inboxes
            .get(&inbox_terminal_id)
            .is_some_and(|inbox| inbox.list(true, None).is_empty());
        if inbox_now_fully_read {
            self.state.mail_pending_nudges.remove(&inbox_terminal_id);
        }

        encode_success(id, ResponseResult::MailRead { message })
    }
}

impl App {
    /// After a successful `mail.send`, decide whether to deliver a one-line
    /// doorbell keystroke nudge to the recipient pane immediately, queue it
    /// for the recipient's next transition into idle/done, or skip it
    /// entirely. Never sent to panes without a detected/reported agent
    /// (plain shells), never to the sender's own pane, and gated by the
    /// `[mail] nudge` config plus the per-message `no_nudge` flag.
    fn maybe_nudge_after_send(
        &mut self,
        recipient: &TerminalId,
        sender: Option<&TerminalId>,
        no_nudge: bool,
    ) {
        if no_nudge || !self.state.mail_nudge_enabled {
            return;
        }
        if sender == Some(recipient) {
            return; // self-send must never nudge
        }
        let Ok(target) = self.resolve_terminal_target(&recipient.to_string()) else {
            return; // recipient pane no longer exists; nothing to nudge
        };
        let Some(pane) = self.pane_info(target.ws_idx, target.pane_id) else {
            return;
        };
        if pane.agent.is_none() {
            return; // never nudge plain shells — only detected/reported agents
        }
        match pane.agent_status {
            AgentStatus::Idle | AgentStatus::Done => {
                self.deliver_nudge(recipient, target.ws_idx, target.pane_id);
            }
            AgentStatus::Working | AgentStatus::Blocked | AgentStatus::Unknown => {
                self.state.mail_pending_nudges.insert(recipient.clone());
            }
        }
    }

    /// Deliver any nudge queued for `recipient` if it just transitioned into
    /// idle/done. Called from `emit_pane_state_update` on every pane state
    /// update; cheap no-op when nothing is pending.
    pub(super) fn deliver_queued_nudge_on_transition(
        &mut self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
        previous_agent_status: AgentStatus,
        agent_status: AgentStatus,
    ) {
        let now_idle_or_done = matches!(agent_status, AgentStatus::Idle | AgentStatus::Done);
        let was_idle_or_done =
            matches!(previous_agent_status, AgentStatus::Idle | AgentStatus::Done);
        if !now_idle_or_done || was_idle_or_done {
            return; // only fires on a fresh transition INTO idle/done
        }
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id))
            .cloned()
        else {
            return;
        };
        if !self.state.mail_pending_nudges.contains(&terminal_id) {
            return;
        }
        self.deliver_nudge(&terminal_id, ws_idx, pane_id);
    }

    /// Type the nudge line + Enter into the recipient pane via the same
    /// internal send-input path the API's `pane.send_text`/`pane.send_input`
    /// handlers use (never shells out). Clears the pending flag on success
    /// or when there's nothing left to say (e.g. the recipient drained its
    /// own inbox between queueing and delivery).
    fn deliver_nudge(
        &mut self,
        recipient: &TerminalId,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
    ) {
        let Some(line) = self.mail_nudge_line(recipient) else {
            self.state.mail_pending_nudges.remove(recipient);
            return;
        };
        let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
            return; // no live runtime to type into; leave any pending flag as-is
        };
        let text_bytes = encode_api_text(runtime, &line);
        let _ = runtime.try_send_bytes(bytes::Bytes::from(text_bytes));
        let enter_key = ["enter".to_string()];
        if let Ok(enter_keys) = encode_api_keys(runtime, &enter_key) {
            for bytes in enter_keys {
                let _ = runtime.try_send_bytes(bytes::Bytes::from(bytes));
            }
        }
        self.state.mail_pending_nudges.remove(recipient);
    }

    /// Compute the nudge line from the recipient's CURRENT unread mail
    /// (coalescing: content is computed fresh at delivery time, not stored
    /// at enqueue time, so a burst of mail while pending collapses into one
    /// line). `None` when nothing is unread (nudge was made moot by a
    /// `mail.read` in the meantime).
    fn mail_nudge_line(&self, recipient: &TerminalId) -> Option<String> {
        let unread = self
            .state
            .mail_inboxes
            .get(recipient)
            .map(|inbox| inbox.list(true, None))
            .unwrap_or_default();
        match unread.len() {
            0 => None,
            1 => {
                let envelope = &unread[0];
                let from = envelope
                    .from_agent
                    .clone()
                    .or_else(|| envelope.from_pane_id.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                Some(format!(
                    "[herdr mail] {} from {} \"{}\" — check: herdr mail read {}",
                    mail_kind_label(envelope.kind),
                    from,
                    envelope.subject,
                    envelope.id,
                ))
            }
            n => Some(format!(
                "[herdr mail] {n} unread — check: herdr mail list --unread-only"
            )),
        }
    }
}

fn mail_kind_label(kind: MailKind) -> &'static str {
    match kind {
        MailKind::Done => "done",
        MailKind::Blocked => "blocked",
        MailKind::Question => "question",
        MailKind::Info => "info",
    }
}

impl App {
    /// Resolve a caller-supplied parent-pane hint (`AgentStartParams`/
    /// `PaneSplitParams.parent_pane_id`) into the `HERDR_PARENT_TERMINAL_ID`/
    /// `HERDR_PARENT_PANE_ID` env pairs to stamp on a freshly spawned child
    /// pane (SPEC-AMENDMENTS A1: the durable terminal id is authoritative
    /// for mail routing, the public pane id is display/debug convenience
    /// only). Resolution failure is non-fatal — spawn must never fail on a
    /// bad parent hint, so this returns an empty vec and logs at debug
    /// level rather than erroring.
    pub(super) fn resolve_parent_pane_env(
        &self,
        parent_pane_id: Option<&str>,
    ) -> Vec<(String, String)> {
        let Some(parent_pane_id) = parent_pane_id else {
            return Vec::new();
        };
        let Some((ws_idx, pane_id)) = self.parse_pane_id(parent_pane_id) else {
            tracing::debug!(
                event = "mail.parent_pane_unresolved",
                parent_pane_id,
                "parent pane hint did not resolve to a live pane; skipping HERDR_PARENT_* env"
            );
            return Vec::new();
        };
        let Some(terminal_id) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id))
        else {
            tracing::debug!(
                event = "mail.parent_pane_unresolved",
                parent_pane_id,
                "parent pane hint has no attached terminal; skipping HERDR_PARENT_* env"
            );
            return Vec::new();
        };
        vec![
            (
                crate::integration::HERDR_PARENT_TERMINAL_ID_ENV_VAR.to_string(),
                terminal_id.to_string(),
            ),
            (
                crate::integration::HERDR_PARENT_PANE_ID_ENV_VAR.to_string(),
                parent_pane_id.to_string(),
            ),
        ]
    }
}

fn mail_not_found(id: String, mail_id: u64) -> String {
    encode_error(
        id,
        "mail_not_found",
        format!("no message with id {mail_id} in this inbox"),
    )
}
