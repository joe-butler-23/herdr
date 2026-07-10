use serde::{Deserialize, Serialize};

/// Message classification. Additive, non-exhaustive from the wire's
/// perspective (new variants are additive; do not remove/rename existing
/// ones without a protocol version bump).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MailKind {
    Done,
    Blocked,
    Question,
    Info,
}

/// Envelope-only view of a mail message: everything EXCEPT the body.
/// Returned by `mail.wait` and `mail.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MailEnvelope {
    /// Monotonic, per-recipient-inbox message id (1-based, never reused).
    pub id: u64,
    /// Sender's durable terminal id, stringified (`TerminalId::to_string()`).
    /// Best-effort: `None` when the sender's self-reported `from_pane_id`
    /// didn't resolve to a live terminal (sending mail must never fail just
    /// because the sender's own identity is stale).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_terminal_id: Option<String>,
    /// Sender's best-known public pane id at send time (may go stale if the
    /// sender pane later closes/compacts; kept for human-readable display
    /// only, never used to resolve identity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_pane_id: Option<String>,
    /// Sender agent label, e.g. "claude", "codex", "opencode", or a hook's
    /// `source` string when not one of those. Best-effort, may be absent for
    /// sends via `herdr mail send` with no agent context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_agent: Option<String>,
    /// Recipient terminal id, stringified. Always resolvable (it's the
    /// actual recipient the message was stored under).
    pub to_terminal_id: String,
    pub kind: MailKind,
    /// Capped to 120 chars server-side at send time.
    pub subject: String,
    /// Byte length of the (possibly truncated) stored body — lets a caller
    /// decide whether `mail.read` is worth the token cost before calling it.
    pub body_bytes: u64,
    /// True if the stored body was truncated at the 64KB cap.
    pub body_truncated: bool,
    /// Server-assigned monotonic delivery sequence (ns-resolution wall clock
    /// at store time, ties broken by `id`). Use for display ordering; `id`
    /// is the authoritative per-inbox ordering key.
    pub seq: u64,
    pub read: bool,
}

/// Full message including body. Returned only by `mail.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MailMessage {
    #[serde(flatten)]
    pub envelope: MailEnvelope,
    /// Capped to 64KB (`MAIL_BODY_MAX_BYTES`); truncated bodies get a
    /// trailing marker appended and `envelope.body_truncated = true`.
    pub body: String,
}

// ---- Params structs (one per Method variant) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MailSendParams {
    /// Recipient: pane id (`p_...`/`w-N`/`ws:pN`), agent name/label (via
    /// `resolve_terminal_target`), a raw terminal id, or the literal string
    /// `"parent"`. `"parent"` MUST already be resolved to a real pane id by
    /// the CLI before this reaches the wire — the server has no notion of
    /// "the caller's parent"; if a client sends the literal string
    /// `"parent"` here the server treats it as a normal (and almost
    /// certainly unresolvable) target string and returns
    /// `recipient_not_found`.
    pub to: String,
    pub kind: MailKind,
    /// Truncated server-side to 120 chars (UTF-8 char boundary safe).
    pub subject: String,
    /// Truncated server-side to `MAIL_BODY_MAX_BYTES` (64KB).
    #[serde(default)]
    pub body: String,
    /// Sender's self-reported public pane id (e.g. from the sender's own
    /// `HERDR_PANE_ID`). Never trusted as authoritative identity — the
    /// server independently resolves it to a `from_terminal_id`, and a
    /// failed resolution never fails the send, only leaves provenance
    /// fields absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_pane_id: Option<String>,
    /// Sender agent label (e.g. `"claude"`/`"codex"`/`"opencode"`), set by
    /// hook callers. `None` for the plain CLI unless explicitly passed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct MailListParams {
    /// Whose inbox to list. Required — there is no calling-pane identity on
    /// this socket API, so the CLI defaults it from `HERDR_PANE_ID`.
    pub inbox: String,
    /// Optional sender filter: same target grammar as `mail.send`'s `to`
    /// (pane id, agent name/label, or raw terminal id), resolved via the
    /// same `resolve_mail_recipient` helper. When present, only envelopes
    /// whose `from_terminal_id` equals the resolved sender are returned;
    /// unresolvable senders fail with `sender_not_found`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(default)]
    pub unread_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MailReadParams {
    /// Whose inbox to read from. `mail.read` looks up an exact message id,
    /// so it has no sender filter (unlike `mail.list`/`mail.wait`).
    pub inbox: String,
    pub id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MailWaitParams {
    /// Whose inbox to wait on. Required on the wire; the CLI defaults it
    /// from `HERDR_PANE_ID` when `--inbox` is omitted. Not defaulted
    /// server-side because the server has no calling-pane identity.
    pub inbox: String,
    /// Optional sender filter (same grammar/resolution as
    /// `MailListParams.sender`). When present, the wait blocks until the
    /// oldest UNREAD message FROM that sender arrives; unread mail from any
    /// other sender does not satisfy the wait and is left untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}
