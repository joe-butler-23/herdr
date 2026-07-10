//! In-memory mail inboxes: per-recipient `TerminalId`-keyed message queues.
//!
//! Transient, not persisted across server restarts — same lifetime
//! guarantee as `EventHub` (`src/api/event_hub.rs`). See
//! `docs/next/api` mail spec for the full design; the short version:
//! - `id` is per-inbox, monotonic, 1-based, never reused (including across
//!   FIFO trims), so a stale remembered id always resolves to either the
//!   right message or a clean `mail_not_found`.
//! - `read` is a per-message flag, not a shared cursor, so `mail.read` can
//!   mark an arbitrary (not necessarily oldest) message read without
//!   silently un-hiding or re-hiding neighbors.
//! - FIFO trim past `MAIL_INBOX_MAX_MESSAGES` drops the oldest message(s),
//!   identical to `EventHub::push`'s overflow handling.

use std::collections::VecDeque;

use crate::api::schema::{MailEnvelope, MailKind, MailMessage};
use crate::app::state::AppState;
use crate::terminal::TerminalId;

pub(crate) const MAIL_SUBJECT_MAX_CHARS: usize = 120;
pub(crate) const MAIL_BODY_MAX_BYTES: usize = 64 * 1024;
pub(crate) const MAIL_INBOX_MAX_MESSAGES: usize = 200;
const MAIL_TRUNCATION_MARKER: &str = "\n…[truncated]";

/// Per-recipient mail inbox.
#[derive(Debug, Default)]
pub(crate) struct MailInbox {
    /// FIFO-ordered; oldest trimmed first past `MAIL_INBOX_MAX_MESSAGES`.
    messages: VecDeque<StoredMailMessage>,
    /// Monotonic id counter, per-inbox, never reused even across trims.
    next_id: u64,
}

/// Fields needed to store a new message; assembled by the `mail.send`
/// handler after recipient/sender resolution.
pub(crate) struct NewMailMessage {
    pub from_terminal_id: Option<TerminalId>,
    pub from_pane_id: Option<String>,
    pub from_agent: Option<String>,
    pub to_terminal_id: TerminalId,
    pub kind: MailKind,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredMailMessage {
    pub id: u64,
    pub from_terminal_id: Option<TerminalId>,
    pub from_pane_id: Option<String>,
    pub from_agent: Option<String>,
    pub to_terminal_id: TerminalId,
    pub kind: MailKind,
    pub subject: String,
    pub body: String,
    pub body_truncated: bool,
    pub seq: u64,
    pub read: bool,
}

impl StoredMailMessage {
    pub(crate) fn to_envelope(&self) -> MailEnvelope {
        MailEnvelope {
            id: self.id,
            from_terminal_id: self.from_terminal_id.as_ref().map(TerminalId::to_string),
            from_pane_id: self.from_pane_id.clone(),
            from_agent: self.from_agent.clone(),
            to_terminal_id: self.to_terminal_id.to_string(),
            kind: self.kind,
            subject: self.subject.clone(),
            body_bytes: self.body.len() as u64,
            body_truncated: self.body_truncated,
            seq: self.seq,
            read: self.read,
        }
    }

    pub(crate) fn to_message(&self) -> MailMessage {
        MailMessage {
            envelope: self.to_envelope(),
            body: self.body.clone(),
        }
    }
}

impl MailInbox {
    /// Insert a new message, assigning the next monotonic id, truncating
    /// subject/body to their caps, and trimming the oldest message(s) past
    /// `MAIL_INBOX_MAX_MESSAGES`. `seq` is caller-supplied (ns-resolution
    /// wall clock at store time) so every inbox shares one clock source.
    pub(crate) fn insert(&mut self, new: NewMailMessage, seq: u64) -> StoredMailMessage {
        self.next_id += 1;
        let id = self.next_id;
        let subject = truncate_subject(&new.subject);
        let (body, body_truncated) = truncate_body(new.body);
        let stored = StoredMailMessage {
            id,
            from_terminal_id: new.from_terminal_id,
            from_pane_id: new.from_pane_id,
            from_agent: new.from_agent,
            to_terminal_id: new.to_terminal_id,
            kind: new.kind,
            subject,
            body,
            body_truncated,
            seq,
            read: false,
        };
        self.messages.push_back(stored.clone());
        let overflow = self.messages.len().saturating_sub(MAIL_INBOX_MAX_MESSAGES);
        if overflow > 0 {
            self.messages.drain(0..overflow);
        }
        stored
    }

    /// Envelopes in FIFO (send) order, optionally filtered to unread-only
    /// and/or to a single sender's `TerminalId`. A message with no resolved
    /// `from_terminal_id` (self-report failed) never matches a `sender`
    /// filter.
    pub(crate) fn list(&self, unread_only: bool, sender: Option<&TerminalId>) -> Vec<MailEnvelope> {
        self.messages
            .iter()
            .filter(|message| !unread_only || !message.read)
            .filter(|message| match sender {
                Some(sender) => message.from_terminal_id.as_ref() == Some(sender),
                None => true,
            })
            .map(StoredMailMessage::to_envelope)
            .collect()
    }

    /// Mark the given id read and return the full stored message. `None` if
    /// the id was never assigned in this inbox or was trimmed out.
    pub(crate) fn mark_read(&mut self, id: u64) -> Option<&StoredMailMessage> {
        let message = self.messages.iter_mut().find(|message| message.id == id)?;
        message.read = true;
        Some(&*message)
    }
}

/// Truncate on a char boundary (never a raw byte cut) to a max character
/// count, matching the discipline `normalize_custom_status`
/// (`src/app/api_helpers.rs`) already applies at a smaller cap.
fn truncate_subject(subject: &str) -> String {
    subject.chars().take(MAIL_SUBJECT_MAX_CHARS).collect()
}

/// Truncate to a max byte count on a UTF-8 char boundary (never splitting a
/// multi-byte scalar), appending a marker when truncation happened.
fn truncate_body(body: String) -> (String, bool) {
    if body.len() <= MAIL_BODY_MAX_BYTES {
        return (body, false);
    }
    let boundary = floor_char_boundary(&body, MAIL_BODY_MAX_BYTES);
    let mut truncated = body[..boundary].to_string();
    truncated.push_str(MAIL_TRUNCATION_MARKER);
    (truncated, true)
}

/// Largest byte index `<= index` that lands on a UTF-8 char boundary.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut idx = index;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

impl AppState {
    /// Store a new mail message in the recipient's inbox, creating the
    /// inbox on first use. `seq` is assigned here (ns-resolution wall clock)
    /// so every inbox shares one monotonic-enough clock source.
    pub(crate) fn store_mail(&mut self, new: NewMailMessage) -> StoredMailMessage {
        let seq = mail_seq_now();
        self.mail_inboxes
            .entry(new.to_terminal_id.clone())
            .or_default()
            .insert(new, seq)
    }
}

fn mail_seq_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(id: &str) -> TerminalId {
        TerminalId::from_raw(id.to_string())
    }

    fn new_message(to: &str, subject: &str, body: &str) -> NewMailMessage {
        NewMailMessage {
            from_terminal_id: Some(terminal("term_from")),
            from_pane_id: Some("p_1_2".to_string()),
            from_agent: Some("claude".to_string()),
            to_terminal_id: terminal(to),
            kind: MailKind::Done,
            subject: subject.to_string(),
            body: body.to_string(),
        }
    }

    #[test]
    fn send_then_list_then_read_marks_only_that_message_read() {
        let mut inbox = MailInbox::default();
        let first = inbox.insert(new_message("term_to", "first", "body one"), 1);
        let second = inbox.insert(new_message("term_to", "second", "body two"), 2);
        assert_eq!(first.id, 1);
        assert_eq!(second.id, 2);

        let unread = inbox.list(true, None);
        assert_eq!(unread.len(), 2);
        assert!(unread.iter().all(|envelope| !envelope.read));

        let read_message = inbox.mark_read(first.id).expect("message exists");
        assert!(read_message.read);
        assert_eq!(read_message.id, 1);

        // Marking one message read doesn't affect the other's unread status.
        let unread_after = inbox.list(true, None);
        assert_eq!(unread_after.len(), 1);
        assert_eq!(unread_after[0].id, 2);

        let all = inbox.list(false, None);
        assert_eq!(all.len(), 2);
        assert!(all[0].read);
        assert!(!all[1].read);
    }

    #[test]
    fn unread_scan_via_list_never_mutates_read_state() {
        // `mail.wait`'s poll loop takes list(true)'s first (lowest-id)
        // entry and must never itself mark anything read.
        let mut inbox = MailInbox::default();
        inbox.insert(new_message("term_to", "a", "a"), 1);
        inbox.insert(new_message("term_to", "b", "b"), 2);

        let lowest = inbox
            .list(true, None)
            .into_iter()
            .next()
            .expect("unread present");
        assert_eq!(lowest.id, 1);
        // Scanning repeatedly must not mark anything read.
        let lowest_again = inbox
            .list(true, None)
            .into_iter()
            .next()
            .expect("still unread");
        assert_eq!(lowest_again.id, 1);
        assert!(!lowest_again.read);

        inbox.mark_read(1);
        let next_lowest = inbox
            .list(true, None)
            .into_iter()
            .next()
            .expect("second still unread");
        assert_eq!(next_lowest.id, 2);
    }

    #[test]
    fn mark_read_unknown_id_returns_none() {
        let mut inbox = MailInbox::default();
        inbox.insert(new_message("term_to", "a", "a"), 1);
        assert!(inbox.mark_read(999).is_none());
    }

    #[test]
    fn list_sender_filter_only_returns_that_senders_messages() {
        let mut inbox = MailInbox::default();
        let mut from_b = new_message("term_to", "from b", "b body");
        from_b.from_terminal_id = Some(terminal("term_b"));
        inbox.insert(new_message("term_to", "from a", "a body"), 1); // from_terminal_id = term_from
        inbox.insert(from_b, 2);

        let only_a = inbox.list(false, Some(&terminal("term_from")));
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].subject, "from a");

        let only_b = inbox.list(true, Some(&terminal("term_b")));
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].subject, "from b");

        // A sender with no messages in this inbox yields an empty list, not
        // an error — filtering is purely a view over existing messages.
        let none_from_unknown = inbox.list(false, Some(&terminal("term_nobody")));
        assert!(none_from_unknown.is_empty());
    }

    #[test]
    fn fifo_trim_drops_oldest_past_cap_and_ids_stay_monotonic() {
        let mut inbox = MailInbox::default();
        let total = MAIL_INBOX_MAX_MESSAGES + 1;
        for i in 0..total {
            inbox.insert(new_message("term_to", "s", "b"), i as u64);
        }
        let all = inbox.list(false, None);
        assert_eq!(all.len(), MAIL_INBOX_MAX_MESSAGES);
        // The oldest (id 1) was trimmed; the surviving oldest is id 2, and
        // the newest carries the true total-sent id, not a re-based count.
        assert_eq!(all.first().unwrap().id, 2);
        assert_eq!(all.last().unwrap().id, total as u64);
    }

    #[test]
    fn trimmed_id_is_permanently_not_found_never_reassigned() {
        let mut inbox = MailInbox::default();
        for i in 0..(MAIL_INBOX_MAX_MESSAGES + 5) {
            inbox.insert(new_message("term_to", "s", "b"), i as u64);
        }
        // id 1 was trimmed out; it must never come back as a hit.
        assert!(inbox.mark_read(1).is_none());
        // A live, still-present id continues to resolve.
        assert!(inbox
            .mark_read((MAIL_INBOX_MAX_MESSAGES + 5) as u64)
            .is_some());
    }

    #[test]
    fn subject_truncates_at_char_boundary_not_byte_count() {
        // Multi-byte scalars (é = 2 bytes) so a byte-count truncation would
        // panic; char-count truncation must not.
        let subject: String = "é".repeat(200);
        let mut inbox = MailInbox::default();
        let stored = inbox.insert(new_message("term_to", &subject, "body"), 1);
        assert_eq!(stored.subject.chars().count(), MAIL_SUBJECT_MAX_CHARS);
    }

    #[test]
    fn body_truncates_at_64kb_on_utf8_boundary_and_flags_truncated() {
        // Build a body whose midpoint (right around the 64KB cap) lands
        // mid-character if sliced by raw byte count.
        let mut body = String::new();
        while body.len() < MAIL_BODY_MAX_BYTES + 100 {
            body.push('â'); // 2-byte UTF-8 scalar
        }
        let body_len_before = body.len();
        assert!(body_len_before > MAIL_BODY_MAX_BYTES);

        let mut inbox = MailInbox::default();
        let stored = inbox.insert(new_message("term_to", "subject", &body), 1);
        assert!(stored.body_truncated);
        assert!(stored.body.is_char_boundary(stored.body.len()));
        // Truncated body length is bounded by the cap plus the marker, and
        // strictly smaller than the original.
        assert!(stored.body.len() <= MAIL_BODY_MAX_BYTES + MAIL_TRUNCATION_MARKER.len());
        assert!(stored.body.len() < body_len_before);
        assert!(stored.body.ends_with(MAIL_TRUNCATION_MARKER));
    }

    #[test]
    fn body_under_cap_is_not_truncated() {
        let mut inbox = MailInbox::default();
        let stored = inbox.insert(new_message("term_to", "subject", "short body"), 1);
        assert!(!stored.body_truncated);
        assert_eq!(stored.body, "short body");
    }

    #[test]
    fn to_envelope_reports_byte_length_of_stored_body() {
        let mut inbox = MailInbox::default();
        let stored = inbox.insert(new_message("term_to", "subject", "hello"), 1);
        let envelope = stored.to_envelope();
        assert_eq!(envelope.body_bytes, 5);
        assert_eq!(envelope.to_terminal_id, "term_to");
        assert_eq!(envelope.from_terminal_id.as_deref(), Some("term_from"));
    }

    #[test]
    fn to_message_includes_body_envelope_does_not() {
        let mut inbox = MailInbox::default();
        let stored = inbox.insert(new_message("term_to", "subject", "the body text"), 1);
        let message = stored.to_message();
        assert_eq!(message.body, "the body text");
        assert_eq!(message.envelope.id, stored.id);
    }
}
