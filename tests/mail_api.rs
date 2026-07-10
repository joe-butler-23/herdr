//! Acceptance tests for the herdr `mail.*` subsystem (SPEC.md §8, W1-W4 +
//! semantics tests; W5's live smoke script lives at `scripts/mail-smoke.sh`).
//!
//! Harness copied 1:1 from `tests/api_ping.rs` (unique_test_dir, SpawnedHerdr,
//! wait_for_socket, JsonLineReader, test_lock, cleanup_spawned_herdr) — a
//! real `herdr server` binary is spawned in a PTY and talked to over raw
//! newline-delimited JSON on its unix socket, exactly like every other
//! integration test in this crate. No typed client: assertions read
//! `response["result"]["type"]` / `response["error"]["code"]` directly.

mod support;

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::{json, Value};
use support::{
    cleanup_test_base, register_runtime_dir, register_spawned_herdr_pid,
    unregister_spawned_herdr_pid,
};

// ================= Harness (copied from tests/api_ping.rs) =================

fn unique_test_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    PathBuf::from(format!("/tmp/hmail-{}-{nanos}", std::process::id()))
}

struct SpawnedHerdr {
    _master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Drop for SpawnedHerdr {
    fn drop(&mut self) {
        let pid = self.child.process_id();
        let _ = self.child.kill();

        if let Some(pid) = pid {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                let mut status = 0;
                let result =
                    unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
                if result == pid as libc::pid_t || result == -1 {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }

            unregister_spawned_herdr_pid(Some(pid));
        }
    }
}

fn cleanup_spawned_herdr(spawned: SpawnedHerdr, base: PathBuf) {
    drop(spawned);
    cleanup_test_base(&base);
}

fn test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn wait_for_socket(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() && UnixStream::connect(path).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("socket did not appear at {}", path.display());
}

fn spawn_herdr(config_home: &Path, runtime_dir: &Path, socket_path: &Path) -> SpawnedHerdr {
    fs::create_dir_all(config_home.join("herdr")).unwrap();
    fs::create_dir_all(runtime_dir).unwrap();
    register_runtime_dir(runtime_dir);
    fs::write(
        config_home.join("herdr/config.toml"),
        "onboarding = false\n",
    )
    .unwrap();

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_herdr"));
    cmd.arg("server");
    cmd.env("XDG_CONFIG_HOME", config_home);
    cmd.env("XDG_RUNTIME_DIR", runtime_dir);
    cmd.env("HERDR_SOCKET_PATH", socket_path);
    cmd.env_remove("HERDR_CLIENT_SOCKET_PATH");
    cmd.env("SHELL", "/bin/sh");
    cmd.env_remove("HERDR_ENV");

    let child = pair.slave.spawn_command(cmd).unwrap();
    register_spawned_herdr_pid(child.process_id());

    SpawnedHerdr {
        _master: pair.master,
        child,
    }
}

struct JsonLineReader {
    stream: UnixStream,
    buf: Vec<u8>,
}

impl JsonLineReader {
    fn connect(socket_path: &Path) -> Self {
        Self {
            stream: UnixStream::connect(socket_path).unwrap(),
            buf: Vec::new(),
        }
    }

    fn send_line(&mut self, json: &str) {
        self.stream.write_all(json.as_bytes()).unwrap();
        self.stream.write_all(b"\n").unwrap();
        self.stream.flush().unwrap();
    }

    fn read_json_line(&mut self, timeout: Duration) -> Value {
        self.try_read_json_line(timeout)
            .unwrap_or_else(|| panic!("timed out waiting for json line"))
    }

    fn try_read_json_line(&mut self, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        self.stream.set_nonblocking(true).unwrap();

        loop {
            if Instant::now() >= deadline {
                self.stream.set_nonblocking(false).unwrap();
                return None;
            }

            if let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                let line = String::from_utf8(self.buf.drain(..=pos).collect()).unwrap();
                self.stream.set_nonblocking(false).unwrap();
                return Some(serde_json::from_str(&line).unwrap());
            }

            let mut bytes = [0u8; 4096];
            match self.stream.read(&mut bytes) {
                Ok(0) => panic!("stream closed while waiting for json line"),
                Ok(n) => self.buf.extend_from_slice(&bytes[..n]),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("failed to read json line: {err}"),
            }
        }
    }
}

fn send_request(socket_path: &Path, request: &Value) -> Value {
    send_request_with_timeout(socket_path, request, Duration::from_secs(5))
}

fn send_request_with_timeout(socket_path: &Path, request: &Value, timeout: Duration) -> Value {
    let mut reader = JsonLineReader::connect(socket_path);
    reader.send_line(&request.to_string());
    reader.read_json_line(timeout)
}

// ================= Mail-specific request helpers =================

struct Pane {
    pane_id: String,
    terminal_id: String,
}

fn workspace_create(socket_path: &Path, cwd: &Path, id: &str) -> Value {
    send_request(
        socket_path,
        &json!({
            "id": id,
            "method": "workspace.create",
            "params": {"cwd": cwd.display().to_string(), "focus": true},
        }),
    )
}

fn root_pane_of(created: &Value) -> Pane {
    Pane {
        pane_id: created["result"]["root_pane"]["pane_id"]
            .as_str()
            .unwrap_or_else(|| panic!("workspace.create did not return a root pane: {created}"))
            .to_string(),
        terminal_id: created["result"]["root_pane"]["terminal_id"]
            .as_str()
            .unwrap()
            .to_string(),
    }
}

fn split_pane(socket_path: &Path, id: &str, target_pane_id: &str, focus: bool) -> Pane {
    let response = send_request(
        socket_path,
        &json!({
            "id": id,
            "method": "pane.split",
            "params": {
                "target_pane_id": target_pane_id,
                "direction": "right",
                "focus": focus,
            },
        }),
    );
    let pane = &response["result"]["pane"];
    Pane {
        pane_id: pane["pane_id"]
            .as_str()
            .unwrap_or_else(|| panic!("pane.split failed: {response}"))
            .to_string(),
        terminal_id: pane["terminal_id"].as_str().unwrap().to_string(),
    }
}

fn mail_send(
    socket_path: &Path,
    id: &str,
    to: &str,
    kind: &str,
    subject: &str,
    body: &str,
    from_pane_id: Option<&str>,
) -> Value {
    mail_send_ex(
        socket_path,
        id,
        to,
        kind,
        subject,
        body,
        from_pane_id,
        false,
    )
}

/// `mail.send` with the `--no-nudge` flag under caller control (the plain
/// `mail_send` above always leaves it at the default `false`).
#[allow(clippy::too_many_arguments)]
fn mail_send_ex(
    socket_path: &Path,
    id: &str,
    to: &str,
    kind: &str,
    subject: &str,
    body: &str,
    from_pane_id: Option<&str>,
    no_nudge: bool,
) -> Value {
    let mut params =
        json!({"to": to, "kind": kind, "subject": subject, "body": body, "no_nudge": no_nudge});
    if let Some(from_pane_id) = from_pane_id {
        params["from_pane_id"] = json!(from_pane_id);
    }
    send_request(
        socket_path,
        &json!({"id": id, "method": "mail.send", "params": params}),
    )
}

/// `pane.report_agent` — establishes (or transitions) a detected/reported
/// agent's state on a pane. Nudge delivery gates on this: only panes with a
/// reported agent label are ever nudged.
fn pane_report_agent(
    socket_path: &Path,
    id: &str,
    pane_id: &str,
    source: &str,
    agent: &str,
    state: &str,
) -> Value {
    send_request(
        socket_path,
        &json!({
            "id": id,
            "method": "pane.report_agent",
            "params": {"pane_id": pane_id, "source": source, "agent": agent, "state": state},
        }),
    )
}

/// `pane.read` (visible source) — used to observe the nudge line typed into
/// a recipient pane. Real PTYs echo typed-but-unsubmitted keystrokes at the
/// tty layer, so this shows the nudge even before/without pressing Enter.
fn pane_read_visible(socket_path: &Path, id: &str, pane_id: &str) -> String {
    let response = send_request(
        socket_path,
        &json!({"id": id, "method": "pane.read", "params": {"pane_id": pane_id, "source": "visible"}}),
    );
    response["result"]["read"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Poll `pane.read` until `needle` appears in the pane's visible text or the
/// deadline elapses. Delivery is asynchronous relative to the request that
/// triggers it (the nudge bytes are written to the pane's PTY master
/// synchronously, but the terminal emulator's screen buffer update runs on a
/// separate reader task), so a bare one-shot read can race the render.
fn wait_for_pane_text(
    socket_path: &Path,
    pane_id: &str,
    needle: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let text = pane_read_visible(socket_path, "wait_for_pane_text", pane_id);
        if text.contains(needle) {
            return Some(text);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Blocking `mail.wait`. The client-side read timeout is padded well past
/// `timeout_ms` so we always observe the server's own `timeout`/match
/// response rather than a spurious client-side read timeout.
fn mail_wait(socket_path: &Path, id: &str, inbox: &str, timeout_ms: Option<u64>) -> Value {
    mail_wait_from(socket_path, id, inbox, None, timeout_ms)
}

/// `mail.wait` with an explicit sender filter (`--from` on the CLI). Other
/// senders' unread mail in `inbox` must not satisfy this wait and must be
/// left untouched.
fn mail_wait_from(
    socket_path: &Path,
    id: &str,
    inbox: &str,
    sender: Option<&str>,
    timeout_ms: Option<u64>,
) -> Value {
    mail_wait_ex(socket_path, id, inbox, sender, timeout_ms, false)
}

/// `mail.wait` with the `--consume` flag under caller control.
#[allow(clippy::too_many_arguments)]
fn mail_wait_ex(
    socket_path: &Path,
    id: &str,
    inbox: &str,
    sender: Option<&str>,
    timeout_ms: Option<u64>,
    consume: bool,
) -> Value {
    let mut params = json!({"inbox": inbox, "consume": consume});
    if let Some(sender) = sender {
        params["sender"] = json!(sender);
    }
    if let Some(timeout_ms) = timeout_ms {
        params["timeout_ms"] = json!(timeout_ms);
    }
    let client_timeout =
        Duration::from_millis(timeout_ms.unwrap_or(4_000)) + Duration::from_secs(5);
    send_request_with_timeout(
        socket_path,
        &json!({"id": id, "method": "mail.wait", "params": params}),
        client_timeout,
    )
}

fn mail_list(socket_path: &Path, id: &str, inbox: &str, unread_only: bool) -> Value {
    mail_list_from(socket_path, id, inbox, None, unread_only)
}

/// `mail.list` with an explicit sender filter (`--from` on the CLI).
fn mail_list_from(
    socket_path: &Path,
    id: &str,
    inbox: &str,
    sender: Option<&str>,
    unread_only: bool,
) -> Value {
    let mut params = json!({"inbox": inbox, "unread_only": unread_only});
    if let Some(sender) = sender {
        params["sender"] = json!(sender);
    }
    send_request(
        socket_path,
        &json!({"id": id, "method": "mail.list", "params": params}),
    )
}

fn mail_read(socket_path: &Path, id: &str, inbox: &str, mail_id: u64) -> Value {
    send_request(
        socket_path,
        &json!({
            "id": id,
            "method": "mail.read",
            "params": {"inbox": inbox, "id": mail_id},
        }),
    )
}

fn new_harness() -> (SpawnedHerdr, PathBuf, PathBuf) {
    let base = unique_test_dir();
    let config_home = base.join("config");
    let runtime_dir = base.join("runtime");
    let socket_path = runtime_dir.join("herdr.sock");
    let child = spawn_herdr(&config_home, &runtime_dir, &socket_path);
    wait_for_socket(&socket_path, Duration::from_secs(5));
    (child, base, socket_path)
}

// ================= W1: visibility-independent delivery =================

/// Proves the reason `mail` exists: `wait agent-status --status done` never
/// fired for a pane that was on-screen (focused/visible in its tab). Mail
/// delivery and `mail.wait` must not depend on focus/seen/active state at
/// all — this creates two panes in the SAME focused tab, sends `done`-mail
/// to the pane that is NOT the currently focused one, and proves `mail.wait`
/// still delivers it without ever calling `pane.read`/`pane.get` on the
/// recipient to "notice" anything.
#[test]
fn mail_delivery_independent_of_pane_visibility() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "w1:ws");
    let pane1 = root_pane_of(&created);
    // Splitting with focus:true makes pane2 the focused pane in the tab;
    // pane1 stays a plain, unfocused (but still on-screen) sibling pane —
    // exactly the "visible, not focused" condition the old mechanism missed.
    let _pane2 = split_pane(&socket_path, "w1:split", &pane1.pane_id, true);

    let pane_get = send_request(
        &socket_path,
        &json!({"id": "w1:get1", "method": "pane.get", "params": {"pane_id": pane1.pane_id}}),
    );
    assert_eq!(
        pane_get["result"]["pane"]["focused"], false,
        "pane1 must have lost focus to the split pane for this test to be meaningful: {pane_get}"
    );

    let sent = mail_send(
        &socket_path,
        "w1:send",
        &pane1.pane_id,
        "done",
        "worker finished",
        "all good",
        None,
    );
    assert_eq!(
        sent["result"]["type"], "mail_sent",
        "mail.send failed: {sent}"
    );

    // No pane.read/pane.get on pane1 happens between send and wait — mail
    // delivery must be entirely independent of that.
    let waited = mail_wait(&socket_path, "w1:wait", &pane1.pane_id, Some(2_000));
    assert_eq!(
        waited["result"]["type"], "mail_wait_matched",
        "mail.wait for an unfocused-but-visible pane failed: {waited}"
    );
    assert_eq!(waited["result"]["envelope"]["kind"], "done");
    assert_eq!(waited["result"]["envelope"]["subject"], "worker finished");

    cleanup_spawned_herdr(child, base);
}

// ================= W2: losslessness / queued-before-armed =================

/// Sends 3 mails to a pane before any `mail.wait` is armed, then proves
/// nothing is dropped and ordering is preserved across repeated
/// wait/read/re-arm cycles, and that a wait against a drained inbox
/// actually blocks (times out) rather than matching stale state.
#[test]
fn mail_queued_before_wait_is_not_lost() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "w2:ws");
    let pane1 = root_pane_of(&created);

    for n in 1..=3u64 {
        let sent = mail_send(
            &socket_path,
            &format!("w2:send{n}"),
            &pane1.pane_id,
            "info",
            &format!("mail #{n}"),
            &format!("body #{n}"),
            None,
        );
        assert_eq!(
            sent["result"]["type"], "mail_sent",
            "send {n} failed: {sent}"
        );
        assert_eq!(
            sent["result"]["envelope"]["id"], n,
            "ids must be assigned in send order"
        );
    }

    for expected_id in 1..=3u64 {
        let start = Instant::now();
        let waited = mail_wait(&socket_path, "w2:wait", &pane1.pane_id, Some(2_000));
        let elapsed = start.elapsed();
        assert_eq!(
            waited["result"]["type"], "mail_wait_matched",
            "wait for id {expected_id} failed: {waited}"
        );
        assert_eq!(
            waited["result"]["envelope"]["id"], expected_id,
            "mail.wait must return the LOWEST-id unread envelope, not most recent"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "wait for already-queued mail took {elapsed:?}; should return on the first poll"
        );

        let read = mail_read(&socket_path, "w2:read", &pane1.pane_id, expected_id);
        assert_eq!(
            read["result"]["type"], "mail_read",
            "read {expected_id} failed: {read}"
        );
        assert_eq!(read["result"]["message"]["read"], true);
    }

    // Backlog is fully drained now — a fresh wait must genuinely block and
    // time out, not spuriously match a re-hidden/stale entry.
    let start = Instant::now();
    let timed_out = mail_wait(&socket_path, "w2:wait_empty", &pane1.pane_id, Some(300));
    let elapsed = start.elapsed();
    assert_eq!(
        timed_out["error"]["code"], "timeout",
        "expected a real timeout once the inbox is drained: {timed_out}"
    );
    assert!(
        elapsed >= Duration::from_millis(300),
        "timeout fired too early ({elapsed:?}); wait must actually block until the deadline"
    );

    cleanup_spawned_herdr(child, base);
}

// ================= W3: one waiter, many senders =================

/// Two distinct sender panes deliver to one recipient inbox. A single
/// armed waiter sees the first sender's mail the instant it lands; after
/// read + re-arm it sees the second sender's mail next — never the first
/// one again, never dropped. A stricter variant has two more senders race
/// concurrently (both dispatch before any wait is re-armed) and proves the
/// single-threaded App event loop still serializes them into a strict,
/// gapless id order with no loss/duplication.
#[test]
fn one_waiter_multiplexes_many_senders() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "w3:ws");
    let recipient = root_pane_of(&created);
    let sender_a = split_pane(&socket_path, "w3:splitA", &recipient.pane_id, false);
    let sender_b = split_pane(&socket_path, "w3:splitB", &recipient.pane_id, false);

    // Arm the wait on a background connection BEFORE A sends, so the
    // waiter genuinely wakes on delivery rather than observing a backlog.
    let socket_path_bg = socket_path.clone();
    let recipient_pane_bg = recipient.pane_id.clone();
    let waiter = thread::spawn(move || {
        mail_wait(
            &socket_path_bg,
            "w3:wait_a",
            &recipient_pane_bg,
            Some(5_000),
        )
    });
    thread::sleep(Duration::from_millis(200)); // let the wait connection arm
    let sent_a = mail_send(
        &socket_path,
        "w3:sendA",
        &recipient.pane_id,
        "done",
        "A finished",
        "from A",
        Some(&sender_a.pane_id),
    );
    assert_eq!(sent_a["result"]["type"], "mail_sent", "{sent_a}");

    let waited_a = waiter.join().unwrap();
    assert_eq!(
        waited_a["result"]["type"], "mail_wait_matched",
        "{waited_a}"
    );
    assert_eq!(
        waited_a["result"]["envelope"]["from_pane_id"], sender_a.pane_id,
        "envelope must carry a distinguishable from_pane_id for sender A"
    );
    let a_id = waited_a["result"]["envelope"]["id"].as_u64().unwrap();
    let read_a = mail_read(&socket_path, "w3:readA", &recipient.pane_id, a_id);
    assert_eq!(read_a["result"]["type"], "mail_read", "{read_a}");

    // After read + re-arm, the next wait must see B's mail — in send
    // order, never re-delivering A's.
    let sent_b = mail_send(
        &socket_path,
        "w3:sendB",
        &recipient.pane_id,
        "done",
        "B finished",
        "from B",
        Some(&sender_b.pane_id),
    );
    assert_eq!(sent_b["result"]["type"], "mail_sent", "{sent_b}");

    let waited_b = mail_wait(&socket_path, "w3:wait_b", &recipient.pane_id, Some(2_000));
    assert_eq!(
        waited_b["result"]["type"], "mail_wait_matched",
        "{waited_b}"
    );
    assert_eq!(
        waited_b["result"]["envelope"]["from_pane_id"], sender_b.pane_id,
        "envelope must carry a distinguishable from_pane_id for sender B"
    );
    let b_id = waited_b["result"]["envelope"]["id"].as_u64().unwrap();
    assert_ne!(b_id, a_id, "must not re-deliver A's already-read message");
    mail_read(&socket_path, "w3:readB", &recipient.pane_id, b_id);

    // Stricter concurrent variant (§2.6): both sends race the dispatch
    // queue with no waiter armed in between.
    let sender_c_pane = sender_a.pane_id.clone();
    let sender_d_pane = sender_b.pane_id.clone();
    let socket_path_c = socket_path.clone();
    let socket_path_d = socket_path.clone();
    let recipient_pane_c = recipient.pane_id.clone();
    let recipient_pane_d = recipient.pane_id.clone();
    let send_c = thread::spawn(move || {
        mail_send(
            &socket_path_c,
            "w3:sendC",
            &recipient_pane_c,
            "info",
            "C burst",
            "c",
            Some(&sender_c_pane),
        )
    });
    let send_d = thread::spawn(move || {
        mail_send(
            &socket_path_d,
            "w3:sendD",
            &recipient_pane_d,
            "info",
            "D burst",
            "d",
            Some(&sender_d_pane),
        )
    });
    let sent_c = send_c.join().unwrap();
    let sent_d = send_d.join().unwrap();
    assert_eq!(sent_c["result"]["type"], "mail_sent", "{sent_c}");
    assert_eq!(sent_d["result"]["type"], "mail_sent", "{sent_d}");

    let unread = mail_list(&socket_path, "w3:list", &recipient.pane_id, true);
    let messages = unread["result"]["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("mail.list failed: {unread}"));
    assert_eq!(
        messages.len(),
        2,
        "expected exactly the two concurrent sends unread, got {messages:?}"
    );
    let mut ids: Vec<u64> = messages.iter().map(|m| m["id"].as_u64().unwrap()).collect();
    ids.sort_unstable();
    assert_eq!(
        ids[1],
        ids[0] + 1,
        "concurrent sends must still be serialized into strictly sequential ids, got {ids:?}"
    );
    let senders: std::collections::HashSet<&str> = messages
        .iter()
        .map(|m| m["from_pane_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        senders.len(),
        2,
        "both concurrent senders must remain distinguishable, got {messages:?}"
    );

    // Draining must still take the lowest-id-first, one at a time.
    let first = mail_wait(
        &socket_path,
        "w3:wait_first",
        &recipient.pane_id,
        Some(2_000),
    );
    let first_id = first["result"]["envelope"]["id"].as_u64().unwrap();
    assert_eq!(first_id, ids[0]);
    mail_read(&socket_path, "w3:read_first", &recipient.pane_id, first_id);
    let second = mail_wait(
        &socket_path,
        "w3:wait_second",
        &recipient.pane_id,
        Some(2_000),
    );
    assert_eq!(second["result"]["envelope"]["id"].as_u64().unwrap(), ids[1]);

    cleanup_spawned_herdr(child, base);
}

// ================= sender filter (mail.wait/mail.list `--from`) =================

/// Live-validation finding: with several workers in flight, an orchestrator
/// waiting on one specific worker must not be satisfied by a DIFFERENT
/// worker's unrelated mail. Sender A's mail is queued unread first; a
/// sender-B-filtered wait must NOT match it and must genuinely time out,
/// leaving A's message untouched. Once B sends, the same filtered wait
/// matches B's envelope — and neither the failed nor the successful wait
/// marks anything read (only `mail.read` does), nor do they ever return the
/// other sender's message via `mail.list --from`.
#[test]
fn wait_filters_by_sender_while_other_mail_queued() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "sf:ws");
    let recipient = root_pane_of(&created);
    let sender_a = split_pane(&socket_path, "sf:splitA", &recipient.pane_id, false);
    let sender_b = split_pane(&socket_path, "sf:splitB", &recipient.pane_id, false);

    // A's mail is queued (unread) before B ever sends anything.
    let sent_a = mail_send(
        &socket_path,
        "sf:sendA",
        &recipient.pane_id,
        "info",
        "A queued first",
        "from A",
        Some(&sender_a.pane_id),
    );
    assert_eq!(sent_a["result"]["type"], "mail_sent", "{sent_a}");
    let a_id = sent_a["result"]["envelope"]["id"].as_u64().unwrap();

    // A waiter filtered to sender B must NOT match A's already-queued mail —
    // it must genuinely block until the deadline, not spuriously match.
    let start = Instant::now();
    let timed_out = mail_wait_from(
        &socket_path,
        "sf:wait_b_times_out",
        &recipient.pane_id,
        Some(&sender_b.pane_id),
        Some(300),
    );
    let elapsed = start.elapsed();
    assert_eq!(
        timed_out["error"]["code"], "timeout",
        "a sender-B-filtered wait must not be satisfied by sender A's mail: {timed_out}"
    );
    assert!(
        elapsed >= Duration::from_millis(300),
        "timeout fired too early ({elapsed:?}); the wait must actually block on the sender filter"
    );

    // A's message must still be unread and untouched by the failed wait.
    let a_still_unread = mail_list_from(
        &socket_path,
        "sf:list_a_unread",
        &recipient.pane_id,
        Some(&sender_a.pane_id),
        true,
    );
    let a_messages = a_still_unread["result"]["messages"].as_array().unwrap();
    assert_eq!(
        a_messages.len(),
        1,
        "sender A's mail must remain unread and visible to a sender-A list filter: {a_still_unread}"
    );
    assert_eq!(a_messages[0]["id"], a_id);
    assert_eq!(a_messages[0]["read"], false);

    // Now B sends; a sender-B-filtered wait must match B's mail specifically
    // (never re-surfacing A's), while A's mail is still untouched.
    let sent_b = mail_send(
        &socket_path,
        "sf:sendB",
        &recipient.pane_id,
        "done",
        "B finished",
        "from B",
        Some(&sender_b.pane_id),
    );
    assert_eq!(sent_b["result"]["type"], "mail_sent", "{sent_b}");
    let b_id = sent_b["result"]["envelope"]["id"].as_u64().unwrap();

    let waited_b = mail_wait_from(
        &socket_path,
        "sf:wait_b_matches",
        &recipient.pane_id,
        Some(&sender_b.pane_id),
        Some(2_000),
    );
    assert_eq!(
        waited_b["result"]["type"], "mail_wait_matched",
        "{waited_b}"
    );
    assert_eq!(waited_b["result"]["envelope"]["id"], b_id);
    assert_eq!(
        waited_b["result"]["envelope"]["from_pane_id"], sender_b.pane_id,
        "a sender-filtered wait must return that sender's envelope"
    );

    // Neither wait attempt marked anything read: both A's and B's messages
    // are still unread, each visible only to its own sender filter.
    let a_unread_after = mail_list_from(
        &socket_path,
        "sf:list_a_after",
        &recipient.pane_id,
        Some(&sender_a.pane_id),
        true,
    );
    let a_msgs_after = a_unread_after["result"]["messages"].as_array().unwrap();
    assert_eq!(a_msgs_after.len(), 1, "{a_unread_after}");
    assert_eq!(a_msgs_after[0]["id"], a_id);
    assert_eq!(
        a_msgs_after[0]["read"], false,
        "mail.wait must never mark a message read"
    );

    let b_unread_after = mail_list_from(
        &socket_path,
        "sf:list_b_after",
        &recipient.pane_id,
        Some(&sender_b.pane_id),
        true,
    );
    let b_msgs_after = b_unread_after["result"]["messages"].as_array().unwrap();
    assert_eq!(b_msgs_after.len(), 1, "{b_unread_after}");
    assert_eq!(b_msgs_after[0]["id"], b_id);

    // Unfiltered list still sees both — the sender filter is a view, not a
    // partition that hides messages from the base inbox.
    let all_unread = mail_list(&socket_path, "sf:list_all", &recipient.pane_id, true);
    let all_messages = all_unread["result"]["messages"].as_array().unwrap();
    assert_eq!(all_messages.len(), 2, "{all_unread}");

    // Unresolvable sender filters fail cleanly rather than matching anything.
    let bad_sender = mail_wait_from(
        &socket_path,
        "sf:wait_bad_sender",
        &recipient.pane_id,
        Some("p_9_999_does_not_exist"),
        Some(500),
    );
    assert_eq!(
        bad_sender["error"]["code"], "sender_not_found",
        "an unresolvable sender filter must fail explicitly: {bad_sender}"
    );
    let bad_sender_list = mail_list_from(
        &socket_path,
        "sf:list_bad_sender",
        &recipient.pane_id,
        Some("p_9_999_does_not_exist"),
        false,
    );
    assert_eq!(
        bad_sender_list["error"]["code"], "sender_not_found",
        "{bad_sender_list}"
    );

    cleanup_spawned_herdr(child, base);
}

// ================= W4: token discipline (envelope/body separation) =================

/// A large (~10KB) body must never leak into `mail.wait`/`mail.list`
/// responses, and those responses must stay small regardless of body size
/// (body-length-invariant). `mail.read` is the only method that returns
/// the full body and flips `read`.
#[test]
fn wait_and_list_are_envelope_only() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "w4:ws");
    let pane1 = root_pane_of(&created);

    let marker = "MAIL_BODY_MARKER_DO_NOT_LEAK_INTO_ENVELOPE";
    let body = format!("{marker}{}", "x".repeat(10_000));

    let sent = mail_send(
        &socket_path,
        "w4:send",
        &pane1.pane_id,
        "info",
        "big body",
        &body,
        None,
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    let mail_id = sent["result"]["envelope"]["id"].as_u64().unwrap();

    let waited = mail_wait(&socket_path, "w4:wait", &pane1.pane_id, Some(2_000));
    let waited_str = waited.to_string();
    assert!(
        !waited_str.contains(marker),
        "mail.wait leaked body content into the envelope: {waited_str}"
    );
    assert!(
        waited_str.len() < 2048,
        "mail.wait response was {} bytes; envelope size must be body-length-invariant",
        waited_str.len()
    );

    let listed = mail_list(&socket_path, "w4:list", &pane1.pane_id, false);
    let listed_str = listed.to_string();
    assert!(
        !listed_str.contains(marker),
        "mail.list leaked body content into the envelope: {listed_str}"
    );
    assert!(
        listed_str.len() < 2048,
        "mail.list response for a single envelope was {} bytes",
        listed_str.len()
    );

    let read = mail_read(&socket_path, "w4:read", &pane1.pane_id, mail_id);
    assert_eq!(read["result"]["type"], "mail_read", "{read}");
    let read_body = read["result"]["message"]["body"].as_str().unwrap();
    assert!(
        read_body.contains(marker),
        "mail.read must return the full body"
    );
    assert_eq!(
        read["result"]["message"]["read"], true,
        "mail.read must flip the message's read flag"
    );

    cleanup_spawned_herdr(child, base);
}

// ================= Semantics =================

#[test]
fn mail_read_marks_read_and_list_unread_only_filters() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "sem1:ws");
    let pane1 = root_pane_of(&created);

    mail_send(
        &socket_path,
        "sem1:send1",
        &pane1.pane_id,
        "info",
        "first",
        "b1",
        None,
    );
    mail_send(
        &socket_path,
        "sem1:send2",
        &pane1.pane_id,
        "info",
        "second",
        "b2",
        None,
    );

    let read = mail_read(&socket_path, "sem1:read", &pane1.pane_id, 1);
    assert_eq!(read["result"]["message"]["id"], 1);
    assert_eq!(read["result"]["message"]["read"], true);

    let unread = mail_list(&socket_path, "sem1:list_unread", &pane1.pane_id, true);
    let unread_msgs = unread["result"]["messages"].as_array().unwrap();
    assert_eq!(
        unread_msgs.len(),
        1,
        "unread_only must filter out the read message"
    );
    assert_eq!(unread_msgs[0]["id"], 2);
    assert_eq!(unread_msgs[0]["read"], false);

    let all = mail_list(&socket_path, "sem1:list_all", &pane1.pane_id, false);
    let all_msgs = all["result"]["messages"].as_array().unwrap();
    assert_eq!(
        all_msgs.len(),
        2,
        "unfiltered list must return both messages"
    );
    assert_eq!(all_msgs[0]["id"], 1);
    assert_eq!(all_msgs[0]["read"], true);
    assert_eq!(all_msgs[1]["id"], 2);
    assert_eq!(
        all_msgs[1]["read"], false,
        "marking id 1 read must not affect id 2"
    );

    cleanup_spawned_herdr(child, base);
}

/// When a `mail.send` call omits `from_agent` (as the plain CLI does), the
/// server now enriches it server-side from the resolved `from_pane_id`
/// sender's detected/reported agent label — the same label `agent.list`
/// shows — instead of leaving provenance blank.
#[test]
fn mail_send_enriches_from_agent_from_reported_sender_pane() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "enrich:ws");
    let recipient = root_pane_of(&created);
    let sender = split_pane(&socket_path, "enrich:split", &recipient.pane_id, false);

    let reported = pane_report_agent(
        &socket_path,
        "enrich:report",
        &sender.pane_id,
        "herdr:kimi",
        "kimi",
        "idle",
    );
    assert_eq!(reported["result"]["type"], "ok", "{reported}");

    let sent = mail_send(
        &socket_path,
        "enrich:send",
        &recipient.pane_id,
        "info",
        "no from_agent supplied",
        "body",
        Some(&sender.pane_id),
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    assert_eq!(
        sent["result"]["envelope"]["from_agent"], "kimi",
        "from_agent must be enriched from the sender pane's reported agent label: {sent}"
    );

    cleanup_spawned_herdr(child, base);
}

/// Amendment A1: the server must accept a raw `TerminalId` string as a
/// mail recipient (not only a public pane id, which compacts/renumbers on
/// pane close) — `resolve_mail_recipient` falls back to a direct
/// `AppState.terminals` key match.
#[test]
fn mail_send_to_raw_terminal_id_delivers() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "a1:ws");
    let pane1 = root_pane_of(&created);

    let sent = mail_send(
        &socket_path,
        "a1:send",
        &pane1.terminal_id,
        "done",
        "via raw terminal id",
        "body",
        None,
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    assert_eq!(
        sent["result"]["envelope"]["to_terminal_id"],
        pane1.terminal_id
    );

    let waited = mail_wait(&socket_path, "a1:wait", &pane1.pane_id, Some(2_000));
    assert_eq!(
        waited["result"]["type"], "mail_wait_matched",
        "mail sent to a raw terminal id must be visible via mail.wait on the equivalent pane id: {waited}"
    );
    assert_eq!(
        waited["result"]["envelope"]["to_terminal_id"],
        pane1.terminal_id
    );

    cleanup_spawned_herdr(child, base);
}

#[test]
fn mail_send_to_unknown_recipient_errors() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let sent = mail_send(
        &socket_path,
        "err1:send",
        "p_9_999_does_not_exist",
        "info",
        "x",
        "y",
        None,
    );
    assert_eq!(
        sent["error"]["code"], "recipient_not_found",
        "unresolvable recipient must fail with recipient_not_found: {sent}"
    );

    cleanup_spawned_herdr(child, base);
}

#[test]
fn mail_subject_and_body_truncation_at_caps() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "cap1:ws");
    let pane1 = root_pane_of(&created);

    let long_subject = "s".repeat(200);
    let long_body_len = 64 * 1024 + 500;
    let long_body = "b".repeat(long_body_len);

    let sent = mail_send(
        &socket_path,
        "cap1:send",
        &pane1.pane_id,
        "info",
        &long_subject,
        &long_body,
        None,
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    let envelope = sent["result"]["envelope"].clone();
    let subject = envelope["subject"].as_str().unwrap();
    assert_eq!(
        subject.chars().count(),
        120,
        "subject over the 120-char cap must be truncated to exactly 120 chars"
    );
    assert_eq!(envelope["body_truncated"], true);
    let body_bytes = envelope["body_bytes"].as_u64().unwrap();
    const TRUNCATION_MARKER: &str = "\n…[truncated]";
    assert!(
        body_bytes <= (64 * 1024 + TRUNCATION_MARKER.len()) as u64,
        "stored body must be bounded by the 64KB cap plus the truncation marker, got {body_bytes} bytes"
    );
    assert!(
        body_bytes < long_body_len as u64,
        "stored body must be strictly smaller than the 64KB+ input"
    );

    let mail_id = envelope["id"].as_u64().unwrap();
    let read = mail_read(&socket_path, "cap1:read", &pane1.pane_id, mail_id);
    let body = read["result"]["message"]["body"].as_str().unwrap();
    assert!(
        body.ends_with(TRUNCATION_MARKER),
        "truncated body must carry the trailing truncation marker, got tail: {:?}",
        &body[body.len().saturating_sub(30)..]
    );
    assert_eq!(
        body.len() as u64,
        body_bytes,
        "read body length must match the envelope's body_bytes"
    );

    cleanup_spawned_herdr(child, base);
}

/// SPEC §6/Amendment A1: `agent.start`'s `parent_pane_id` must be resolved
/// server-side into BOTH `HERDR_PARENT_TERMINAL_ID` (authoritative for mail
/// routing — durable, survives pane-id compaction) and `HERDR_PARENT_PANE_ID`
/// (display/debug convenience only) in the spawned pane's environment.
/// Exercised via a raw `agent.start` request (bypassing the CLI's
/// `--parent-pane` flag, which is a separate work package) followed by a
/// `pane.read` of the spawned pane's own stdout echoing those two env vars
/// back out — fully exercisable at the wire layer, no CLI/hook changes
/// needed for this assertion.
#[test]
fn parent_env_vars_stamped_on_agent_start() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "penv:ws");
    let orchestrator = root_pane_of(&created);

    let started = send_request(
        &socket_path,
        &json!({
            "id": "penv:start",
            "method": "agent.start",
            "params": {
                "name": "worker",
                "cwd": base.display().to_string(),
                "argv": ["/bin/sh", "-c", "echo TERM=$HERDR_PARENT_TERMINAL_ID PANE=$HERDR_PARENT_PANE_ID; sleep 5"],
                "parent_pane_id": orchestrator.pane_id,
            },
        }),
    );
    assert_eq!(started["result"]["type"], "agent_started", "{started}");
    let worker_pane_id = started["result"]["agent"]["pane_id"]
        .as_str()
        .unwrap()
        .to_string();

    let expected = format!(
        "TERM={} PANE={}",
        orchestrator.terminal_id, orchestrator.pane_id
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_text;
    loop {
        // `source: "visible"` (not "recent"/scrollback) is what actually
        // surfaces output for a pane that was never focused/rendered —
        // verified empirically against the real binary.
        let read = send_request(
            &socket_path,
            &json!({
                "id": "penv:read",
                "method": "pane.read",
                "params": {"pane_id": worker_pane_id, "source": "visible"},
            }),
        );
        last_text = read["result"]["read"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if last_text.contains(&expected) || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        last_text.contains(&expected),
        "expected worker pane output to contain {expected:?} (proving HERDR_PARENT_TERMINAL_ID/\
         HERDR_PARENT_PANE_ID were stamped from parent_pane_id), got: {last_text:?}"
    );

    cleanup_spawned_herdr(child, base);
}

// ================= Nudge (doorbell) mechanism =================

/// (a) A recipient pane with a hook-reported agent already in an
/// idle/done state receives the nudge immediately: `mail.send` types a
/// `[herdr mail] ...` line into the pane's PTY (observable via `pane.read`),
/// no separate wake-up step required.
#[test]
fn nudge_delivers_immediately_to_idle_reported_agent() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "nudge_a:ws");
    let recipient = root_pane_of(&created);
    let sender = split_pane(&socket_path, "nudge_a:split", &recipient.pane_id, false);

    let reported = pane_report_agent(
        &socket_path,
        "nudge_a:report",
        &recipient.pane_id,
        "herdr:kimi",
        "kimi",
        "idle",
    );
    assert_eq!(reported["result"]["type"], "ok", "{reported}");

    let sent = mail_send(
        &socket_path,
        "nudge_a:send",
        &recipient.pane_id,
        "done",
        "worker finished",
        "all good",
        Some(&sender.pane_id),
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    let mail_id = sent["result"]["envelope"]["id"].as_u64().unwrap();

    let text = wait_for_pane_text(
        &socket_path,
        &recipient.pane_id,
        "[herdr mail]",
        Duration::from_secs(5),
    )
    .unwrap_or_else(|| panic!("nudge line never appeared in the idle recipient's pane"));
    assert!(
        text.contains("done") && text.contains("worker finished"),
        "nudge line should name the kind and subject of the single unread message, got: {text}"
    );
    assert!(
        text.contains(&format!("herdr mail read {mail_id}")),
        "nudge line should point at the exact unread message id, got: {text}"
    );
    assert!(
        !text.contains("reply:"),
        "a `done` mail expects no reply, so its nudge line must not append a reply hint: {text}"
    );

    cleanup_spawned_herdr(child, base);
}

/// A `question` mail's nudge line appends a reply hint pointing at the
/// sender's pane id from the envelope, so a nudged recipient replies by
/// mail instead of `pane run`-ing text into the sender's pane (the failure
/// mode this hint exists to prevent).
#[test]
fn nudge_reply_hint_present_for_question_kind() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "nudge_f:ws");
    let recipient = root_pane_of(&created);
    let sender = split_pane(&socket_path, "nudge_f:split", &recipient.pane_id, false);

    let reported = pane_report_agent(
        &socket_path,
        "nudge_f:report",
        &recipient.pane_id,
        "herdr:kimi",
        "kimi",
        "idle",
    );
    assert_eq!(reported["result"]["type"], "ok", "{reported}");

    let sent = mail_send(
        &socket_path,
        "nudge_f:send",
        &recipient.pane_id,
        "question",
        "need input",
        "which way?",
        Some(&sender.pane_id),
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    let mail_id = sent["result"]["envelope"]["id"].as_u64().unwrap();

    let text = wait_for_pane_text(
        &socket_path,
        &recipient.pane_id,
        "[herdr mail]",
        Duration::from_secs(5),
    )
    .unwrap_or_else(|| panic!("nudge line never appeared in the idle recipient's pane"));
    // The recipient's pane is only 80 columns wide, so the visible-source
    // render can hard-wrap the (longer, reply-hint-bearing) nudge line
    // mid-token — join it back so a wrap boundary can't split a substring
    // match apart, matching how `--source recent-unwrapped` already
    // handles this for other assertions in this file.
    let joined: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    assert!(
        joined.contains(&format!("herdr mail read {mail_id}")),
        "nudge line should point at the exact unread message id, got: {text}"
    );
    assert!(
        joined.contains(&format!(
            "reply: herdr mail send {} --kind info --subject",
            sender.pane_id
        )),
        "a question mail's nudge line should hint replying to the sender's pane id, got: {text}"
    );

    cleanup_spawned_herdr(child, base);
}

/// (b) A recipient reported `working` must NOT receive the nudge — it must
/// be queued instead — and the queued nudge is delivered on the recipient's
/// next transition into idle (a second `pane.report_agent` call), not
/// before.
#[test]
fn nudge_queues_while_working_and_delivers_on_transition_to_idle() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "nudge_b:ws");
    let recipient = root_pane_of(&created);
    let sender = split_pane(&socket_path, "nudge_b:split", &recipient.pane_id, false);

    let reported = pane_report_agent(
        &socket_path,
        "nudge_b:report_working",
        &recipient.pane_id,
        "herdr:kimi",
        "kimi",
        "working",
    );
    assert_eq!(reported["result"]["type"], "ok", "{reported}");

    let sent = mail_send(
        &socket_path,
        "nudge_b:send",
        &recipient.pane_id,
        "done",
        "worker finished",
        "all good",
        Some(&sender.pane_id),
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");

    // While still working, no nudge — typing into a working agent's turn
    // would corrupt it.
    let text_while_working = pane_read_visible(&socket_path, "nudge_b:read1", &recipient.pane_id);
    assert!(
        !text_while_working.contains("[herdr mail]"),
        "must not nudge a working recipient: {text_while_working}"
    );
    thread::sleep(Duration::from_millis(300));
    let text_while_working_2 = pane_read_visible(&socket_path, "nudge_b:read2", &recipient.pane_id);
    assert!(
        !text_while_working_2.contains("[herdr mail]"),
        "must still not nudge a working recipient after a delay: {text_while_working_2}"
    );

    let transitioned = pane_report_agent(
        &socket_path,
        "nudge_b:report_idle",
        &recipient.pane_id,
        "herdr:kimi",
        "kimi",
        "idle",
    );
    assert_eq!(transitioned["result"]["type"], "ok", "{transitioned}");

    let text_after_idle = wait_for_pane_text(
        &socket_path,
        &recipient.pane_id,
        "[herdr mail]",
        Duration::from_secs(5),
    )
    .unwrap_or_else(|| panic!("queued nudge never arrived after transition to idle"));
    assert!(
        text_after_idle.contains("worker finished"),
        "delivered nudge should still describe the originally-queued message: {text_after_idle}"
    );

    cleanup_spawned_herdr(child, base);
}

/// (c) `--no-nudge` (wire: `no_nudge: true`) suppresses the nudge entirely
/// for that one message, even though the recipient is already idle and
/// would otherwise be nudged immediately. The message itself is still
/// stored and deliverable via `mail.wait`/`mail.read`.
#[test]
fn nudge_suppressed_by_no_nudge_flag() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "nudge_c:ws");
    let recipient = root_pane_of(&created);
    let sender = split_pane(&socket_path, "nudge_c:split", &recipient.pane_id, false);

    let reported = pane_report_agent(
        &socket_path,
        "nudge_c:report",
        &recipient.pane_id,
        "herdr:kimi",
        "kimi",
        "idle",
    );
    assert_eq!(reported["result"]["type"], "ok", "{reported}");

    let sent = mail_send_ex(
        &socket_path,
        "nudge_c:send",
        &recipient.pane_id,
        "done",
        "quiet completion",
        "no nudge please",
        Some(&sender.pane_id),
        true,
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");

    thread::sleep(Duration::from_millis(500));
    let text = pane_read_visible(&socket_path, "nudge_c:read", &recipient.pane_id);
    assert!(
        !text.contains("[herdr mail]"),
        "--no-nudge must suppress the keystroke nudge: {text}"
    );

    // The message is still there, just quiet — mail.wait still sees it.
    let waited = mail_wait(
        &socket_path,
        "nudge_c:wait",
        &recipient.pane_id,
        Some(2_000),
    );
    assert_eq!(waited["result"]["type"], "mail_wait_matched", "{waited}");
    assert_eq!(waited["result"]["envelope"]["subject"], "quiet completion");

    cleanup_spawned_herdr(child, base);
}

/// (d) A plain shell pane (no `pane.report_agent` ever called on it) must
/// never be nudged, regardless of mail sent to it — only panes with a
/// detected/reported agent are eligible.
#[test]
fn nudge_never_delivered_to_plain_shell_pane() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "nudge_d:ws");
    let recipient = root_pane_of(&created);

    let sent = mail_send(
        &socket_path,
        "nudge_d:send",
        &recipient.pane_id,
        "info",
        "fyi",
        "just some info",
        None,
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");

    thread::sleep(Duration::from_millis(500));
    let text = pane_read_visible(&socket_path, "nudge_d:read", &recipient.pane_id);
    assert!(
        !text.contains("[herdr mail]"),
        "a plain shell pane with no detected/reported agent must never be nudged: {text}"
    );

    // Mail is still delivered through the normal channel.
    let waited = mail_wait(
        &socket_path,
        "nudge_d:wait",
        &recipient.pane_id,
        Some(2_000),
    );
    assert_eq!(waited["result"]["type"], "mail_wait_matched", "{waited}");

    cleanup_spawned_herdr(child, base);
}

/// (e) If the recipient reads all its mail on its own before a queued nudge
/// is delivered, the pending nudge is cancelled — the subsequent transition
/// into idle must NOT type a now-stale nudge line.
#[test]
fn nudge_cancelled_when_recipient_reads_mail_before_delivery() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "nudge_e:ws");
    let recipient = root_pane_of(&created);
    let sender = split_pane(&socket_path, "nudge_e:split", &recipient.pane_id, false);

    let reported = pane_report_agent(
        &socket_path,
        "nudge_e:report_working",
        &recipient.pane_id,
        "herdr:kimi",
        "kimi",
        "working",
    );
    assert_eq!(reported["result"]["type"], "ok", "{reported}");

    let sent = mail_send(
        &socket_path,
        "nudge_e:send",
        &recipient.pane_id,
        "done",
        "worker finished",
        "all good",
        Some(&sender.pane_id),
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    let mail_id = sent["result"]["envelope"]["id"].as_u64().unwrap();

    // The recipient checks its own mail before the queued nudge fires.
    let read = mail_read(
        &socket_path,
        "nudge_e:read_own_mail",
        &recipient.pane_id,
        mail_id,
    );
    assert_eq!(read["result"]["type"], "mail_read", "{read}");

    let transitioned = pane_report_agent(
        &socket_path,
        "nudge_e:report_idle",
        &recipient.pane_id,
        "herdr:kimi",
        "kimi",
        "idle",
    );
    assert_eq!(transitioned["result"]["type"], "ok", "{transitioned}");

    thread::sleep(Duration::from_millis(500));
    let text = pane_read_visible(&socket_path, "nudge_e:read_pane", &recipient.pane_id);
    assert!(
        !text.contains("[herdr mail]"),
        "reading all mail before delivery must cancel the queued nudge: {text}"
    );

    cleanup_spawned_herdr(child, base);
}

// ================= mail.wait --consume =================

/// `mail.wait --consume` atomically marks the matched message read (the
/// returned envelope shows `read: true`) so a re-armed wait does not
/// re-return the same message — it must genuinely time out instead.
#[test]
fn mail_wait_consume_marks_read_and_re_armed_wait_times_out() {
    let _lock = test_lock();
    let (child, base, socket_path) = new_harness();

    let created = workspace_create(&socket_path, &base, "consume:ws");
    let pane1 = root_pane_of(&created);

    let sent = mail_send(
        &socket_path,
        "consume:send",
        &pane1.pane_id,
        "info",
        "consume me",
        "body",
        None,
    );
    assert_eq!(sent["result"]["type"], "mail_sent", "{sent}");
    let mail_id = sent["result"]["envelope"]["id"].as_u64().unwrap();

    let waited = mail_wait_ex(
        &socket_path,
        "consume:wait1",
        &pane1.pane_id,
        None,
        Some(2_000),
        true,
    );
    assert_eq!(waited["result"]["type"], "mail_wait_matched", "{waited}");
    assert_eq!(waited["result"]["envelope"]["id"], mail_id);
    assert_eq!(
        waited["result"]["envelope"]["read"], true,
        "consume must mark the matched envelope read in the same response"
    );

    // Confirm it's really persisted as read, not just reflected in this
    // one response.
    let listed = mail_list(&socket_path, "consume:list", &pane1.pane_id, true);
    let unread = listed["result"]["messages"].as_array().unwrap();
    assert!(
        unread.is_empty(),
        "consumed message must no longer show up as unread: {listed}"
    );

    // A re-armed wait must genuinely block and time out — the message was
    // consumed, not left for a second delivery.
    let start = Instant::now();
    let timed_out = mail_wait_ex(
        &socket_path,
        "consume:wait2",
        &pane1.pane_id,
        None,
        Some(300),
        false,
    );
    let elapsed = start.elapsed();
    assert_eq!(
        timed_out["error"]["code"], "timeout",
        "a re-armed wait after --consume must time out, not re-return the same message: {timed_out}"
    );
    assert!(
        elapsed >= Duration::from_millis(300),
        "timeout fired too early ({elapsed:?})"
    );

    cleanup_spawned_herdr(child, base);
}
