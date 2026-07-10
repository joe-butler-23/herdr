use crate::api::schema::{MailListParams, MailReadParams, MailSendParams, ResponseResult};
use crate::app::mail_store::NewMailMessage;
use crate::app::App;
use crate::terminal::TerminalId;

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
            from_terminal_id,
            from_pane_id: params.from_pane_id,
            from_agent: params.from_agent,
            to_terminal_id: to,
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

        encode_success(id, ResponseResult::MailSent { envelope })
    }

    pub(super) fn handle_mail_list(&mut self, id: String, params: MailListParams) -> String {
        let from = match self.resolve_mail_recipient(&params.from) {
            Ok(terminal_id) => terminal_id,
            Err(target) => {
                return encode_error(
                    id,
                    "recipient_not_found",
                    format!("no pane/agent/terminal matches '{target}'"),
                )
            }
        };
        let messages = self
            .state
            .mail_inboxes
            .get(&from)
            .map(|inbox| inbox.list(params.unread_only))
            .unwrap_or_default();
        encode_success(id, ResponseResult::MailList { messages })
    }

    pub(super) fn handle_mail_read(&mut self, id: String, params: MailReadParams) -> String {
        let from = match self.resolve_mail_recipient(&params.from) {
            Ok(terminal_id) => terminal_id,
            Err(target) => {
                return encode_error(
                    id,
                    "recipient_not_found",
                    format!("no pane/agent/terminal matches '{target}'"),
                )
            }
        };
        let Some(inbox) = self.state.mail_inboxes.get_mut(&from) else {
            return mail_not_found(id, params.id);
        };
        let Some(message) = inbox.mark_read(params.id) else {
            return mail_not_found(id, params.id);
        };
        encode_success(
            id,
            ResponseResult::MailRead {
                message: message.to_message(),
            },
        )
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
