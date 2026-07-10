#!/usr/bin/env bash
#
# mail-smoke.sh — live smoke test for the herdr `mail.*` subsystem
# (SPEC.md §8, W5). Manual/CI-optional runbook, NOT part of `just test`/
# `just ci` (separate from the deterministic `tests/mail_api.rs` gate).
#
# WHAT IT DOES: start a real `herdr server` in a scratch session dir
# (never the user's real config) -> create a workspace (root pane =
# "orchestrator") -> spawn a "worker" pane via `agent start --parent-pane`
# (stamps HERDR_PARENT_TERMINAL_ID/HERDR_PARENT_PANE_ID into the worker's
# env, SPEC-AMENDMENTS.md A1) -> the worker runs a stub script that calls
# `herdr mail send parent --kind done ...` directly, WITHOUT assuming any
# claude/codex/opencode hook assets are installed. (To exercise the real
# hook path instead once hooks are installed, swap $worker_script's body
# for an actual claude/codex/opencode invocation that does one small task
# and exits — that's a separate, environment-dependent live validation,
# not this script's job.) -> the orchestrator runs `mail wait`, asserts a
# `done` envelope comes back, then `mail read`s and prints the body ->
# prints a back-of-envelope byte-count comparison (old poll-`pane read`
# loop vs the new one-shot mail.wait+mail.read path) -> tears down and
# prints a PASS/FAIL summary.
#
# PREREQS: a working `herdr` binary ($HERDR_BIN, or this script builds
# one) whose CLI has the `herdr mail` verbs and `agent start --parent-pane`.
# Preflight-checked below; prints "[SKIP]" (exit 0) if either is missing,
# so this is safe to run against a tree where that CLI work hasn't landed.
#
# RUN:    scripts/mail-smoke.sh
#         HERDR_BIN=/path/to/herdr scripts/mail-smoke.sh  # pick a binary
#         MAIL_SMOKE_KEEP=1 scripts/mail-smoke.sh         # keep scratch dir
#
# EXPECT: a "[PASS] ..." line per step, a byte-count comparison block, and
# a final "mail smoke: PASS" (exit 0). A failed assertion prints
# "[FAIL] ..." and exits non-zero.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERDR_BIN="${HERDR_BIN:-$ROOT/target/debug/herdr}"
BASE="$(mktemp -d /tmp/herdr-mail-smoke.XXXXXX)"
CONFIG_HOME="$BASE/config"
RUNTIME_DIR="$BASE/runtime"
SOCKET_PATH="$RUNTIME_DIR/herdr.sock"
SERVER_PID=""

if [[ "$CONFIG_HOME" == "$HOME/.config"* ]]; then
  echo "refusing to run smoke test against $CONFIG_HOME" >&2
  exit 1
fi

cleanup() {
  set +e
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" >/dev/null 2>&1
  if [[ "${MAIL_SMOKE_KEEP:-0}" != 1 ]]; then
    rm -rf "$BASE"
  else
    echo "scratch dir kept at: $BASE"
  fi
}
trap cleanup EXIT

herdr() {
  XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" \
    HERDR_SOCKET_PATH="$SOCKET_PATH" "$HERDR_BIN" "$@"
}

json_field() {
  # json_field <json-on-stdin> <dotted.path>
  python3 -c '
import json, sys
value = json.load(sys.stdin)
for key in sys.argv[1].split("."):
    value = value[int(key)] if key.isdigit() else value[key]
print(value)
' "$1"
}

raw_request() {
  # raw_request <json-request-body> — one raw socket round trip, printing
  # the exact response line (used for the byte-count comparison below, so
  # metadata the CLI's `pane read` strips out is still counted).
  python3 - "$SOCKET_PATH" "$1" <<'PY'
import socket, sys
path, body = sys.argv[1], sys.argv[2]
c = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
c.connect(path)
c.sendall(body.encode() + b"\n")
resp = b""
while not resp.endswith(b"\n"):
    chunk = c.recv(65536)
    if not chunk:
        break
    resp += chunk
sys.stdout.write(resp.decode().strip())
PY
}

if [[ ! -x "$HERDR_BIN" ]]; then
  echo "building herdr (no binary at $HERDR_BIN)..."
  cargo build --locked --bin herdr --manifest-path "$ROOT/Cargo.toml"
fi

mkdir -p "$CONFIG_HOME/herdr" "$RUNTIME_DIR"
printf 'onboarding = false\n' > "$CONFIG_HOME/herdr/config.toml"

if ! herdr mail help >/dev/null 2>&1; then
  echo "[SKIP] this build's CLI has no 'herdr mail' verb yet — nothing to smoke-test"
  exit 0
fi
# Captured (not piped straight into `grep -q`): a live pipe into `grep -q`
# can make the writer see SIGPIPE once grep finds its match and exits,
# which `pipefail` would then report as a spurious failure.
agent_start_help="$("$HERDR_BIN" agent start help 2>&1 || true)"
if ! grep -q -- '--parent-pane' <<<"$agent_start_help"; then
  echo "[SKIP] this build's 'herdr agent start' has no --parent-pane flag yet — nothing to smoke-test"
  exit 0
fi

XDG_CONFIG_HOME="$CONFIG_HOME" XDG_RUNTIME_DIR="$RUNTIME_DIR" HERDR_SOCKET_PATH="$SOCKET_PATH" \
  "$HERDR_BIN" server >"$BASE/server.log" 2>&1 &
SERVER_PID=$!
for _ in $(seq 1 200); do [[ -S "$SOCKET_PATH" ]] && break; sleep 0.05; done
[[ -S "$SOCKET_PATH" ]] || { echo "[FAIL] server socket never appeared"; exit 1; }
echo "[PASS] server started ($SOCKET_PATH)"

created="$(herdr workspace create --cwd "$BASE" --focus)"
orch_pane="$(echo "$created" | json_field result.root_pane.pane_id)"
echo "[PASS] orchestrator pane: $orch_pane"

# Stub worker: stands in for a claude/codex/opencode hook, calling
# `herdr mail send parent` directly (no hook assets required).
worker_script="$BASE/worker.sh"
cat >"$worker_script" <<WORKER
#!/bin/sh
set -e
echo "stub worker running in \$HERDR_PANE_ID, parent=\$HERDR_PARENT_TERMINAL_ID"
"$HERDR_BIN" mail send parent --kind done --subject "task complete" --body-stdin <<BODY
stub worker finished its one small task at \$(date -u +%FT%TZ)
BODY
WORKER
chmod +x "$worker_script"

started="$(herdr agent start worker --cwd "$BASE" --parent-pane "$orch_pane" -- /bin/sh "$worker_script")"
worker_pane="$(echo "$started" | json_field result.agent.pane_id)"
echo "[PASS] worker pane started: $worker_pane"

wait_response="$(HERDR_PANE_ID="$orch_pane" herdr mail wait --timeout 60000)"
if ! echo "$wait_response" | json_field result.type >/dev/null 2>&1 \
  || [[ "$(echo "$wait_response" | json_field result.type)" != "mail_wait_matched" ]]; then
  echo "[FAIL] mail.wait did not return a matched envelope: $wait_response"
  exit 1
fi
mail_kind="$(echo "$wait_response" | json_field result.envelope.kind)"
mail_id="$(echo "$wait_response" | json_field result.envelope.id)"
[[ "$mail_kind" == "done" ]] || { echo "[FAIL] expected kind=done, got $mail_kind"; exit 1; }
echo "[PASS] mail.wait matched id=$mail_id kind=$mail_kind"

read_response="$(herdr mail read "$mail_id" --from "$orch_pane")"
echo "[PASS] mail.read body:"
echo "$read_response" | json_field result.message.body | sed 's/^/    /'

# Back-of-envelope token-cost comparison: old poll-`pane read` loop vs the
# new one-shot mail.wait + mail.read path. Not a formal benchmark. Warms
# the orchestrator pane up with some representative chatter first (a bare
# idle prompt would understate a real polling loop's per-read size), then
# measures the RAW wire response (not the CLI's extracted-text-only print,
# which would drop the metadata a real poll loop still pays to transfer).
herdr pane send-text "$orch_pane" "echo agent status: still working, 3 files changed" >/dev/null
herdr pane send-keys "$orch_pane" Enter >/dev/null
sleep 0.3
old_style_poll="$(raw_request "{\"id\":\"smoke:oldpoll\",\"method\":\"pane.read\",\"params\":{\"pane_id\":\"$orch_pane\",\"source\":\"visible\",\"lines\":100}}")"
old_poll_bytes=${#old_style_poll}
polls=20
old_total_bytes=$((old_poll_bytes * polls))
new_total_bytes=$((${#wait_response} + ${#read_response}))
cat <<SUMMARY

--- token-cost comparison (back-of-envelope, not a formal benchmark) ---
  old style: ~${polls} polls x ${old_poll_bytes} bytes/poll (pane read --lines 100)
             = ${old_total_bytes} bytes transferred over the task
  new style: 1x mail.wait (${#wait_response} bytes) + 1x mail.read (${#read_response} bytes)
             = ${new_total_bytes} bytes transferred
SUMMARY

echo
echo "mail smoke: PASS"
