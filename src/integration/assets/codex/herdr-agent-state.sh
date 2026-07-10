#!/bin/sh
# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# HERDR_INTEGRATION_ID=codex
# HERDR_INTEGRATION_VERSION=11

set -eu

action="${1:-}"
hook_input_file="$(mktemp "${TMPDIR:-/tmp}/herdr-codex-hook.XXXXXX")" || exit 0
trap 'rm -f "$hook_input_file"' EXIT HUP INT TERM
cat >"$hook_input_file" 2>/dev/null || true

case "$action" in
  session|mail-done|mail-blocked) ;;
  *) exit 0 ;;
esac

[ "${HERDR_ENV:-}" = "1" ] || exit 0
[ -n "${HERDR_SOCKET_PATH:-}" ] || exit 0
[ -n "${HERDR_PANE_ID:-}" ] || exit 0
command -v python3 >/dev/null 2>&1 || exit 0

HERDR_ACTION="$action" HERDR_HOOK_INPUT_FILE="$hook_input_file" python3 - <<'PY'
import json
import os
import random
import socket
import time

source = "herdr:codex"
action = os.environ.get("HERDR_ACTION", "")
pane_id = os.environ.get("HERDR_PANE_ID")
socket_path = os.environ.get("HERDR_SOCKET_PATH")
hook_input_file = os.environ.get("HERDR_HOOK_INPUT_FILE")

if not pane_id or not socket_path:
    raise SystemExit(0)

hook_input = {}
if hook_input_file:
    try:
        with open(hook_input_file, encoding="utf-8") as handle:
            content = handle.read()
        if content.strip():
            hook_input = json.loads(content)
    except Exception:
        hook_input = {}

hook_event_name = str(hook_input.get("hook_event_name") or "")
if action == "session" and hook_event_name and hook_event_name != "SessionStart":
    raise SystemExit(0)

if action in ("mail-done", "mail-blocked"):
    # Mail is additive, alongside session reporting, and only fires for
    # delegated workers: standalone (non-herdr-delegated) sessions never have
    # a parent stamped, so they stay silent by construction.
    parent_pane_id = os.environ.get("HERDR_PARENT_TERMINAL_ID") or os.environ.get(
        "HERDR_PARENT_PANE_ID"
    )
    if not parent_pane_id:
        raise SystemExit(0)
    if action == "mail-done":
        if hook_input.get("stop_hook_active"):
            # This Stop fired because an earlier stop hook already continued
            # the turn; it is not the genuine final stop, so don't emit a
            # duplicate done-mail for the same logical turn.
            raise SystemExit(0)
        last_message = hook_input.get("last_assistant_message") or ""
        mail_kind = "done"
        if last_message:
            mail_subject = (last_message.splitlines() or [""])[0][:120]
            mail_body = last_message
        else:
            # A turn that emitted no assistant text (e.g. a tool-only turn)
            # would otherwise produce a zero-information envelope: empty
            # subject and body_bytes 0. Say so explicitly instead.
            mail_subject = "done (no message)"
            mail_body = "(no message)"
    else:
        # PermissionRequest's payload carries tool_name/tool_input, not a
        # free-text message field; build a best-effort human summary.
        tool_name = hook_input.get("tool_name") or ""
        notification_message = (
            f"needs permission: {tool_name}" if tool_name else "needs permission"
        )
        mail_kind = "blocked"
        mail_subject = notification_message[:120]
        mail_body = notification_message
    mail_request = {
        "id": f"{source}:mail:{int(time.time() * 1000)}:{random.randrange(1_000_000):06d}",
        "method": "mail.send",
        "params": {
            "to": parent_pane_id,
            "kind": mail_kind,
            "subject": mail_subject,
            "body": mail_body,
            "from_pane_id": pane_id,
            "from_agent": "codex",
        },
    }
    try:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(0.5)
        client.connect(socket_path)
        client.sendall((json.dumps(mail_request) + "\n").encode())
        try:
            client.recv(4096)
        except Exception:
            pass
        client.close()
    except Exception:
        pass
    raise SystemExit(0)

request_id = f"{source}:{int(time.time() * 1000)}:{random.randrange(1_000_000):06d}"
report_seq = time.time_ns()
session_id = hook_input.get("session_id")
agent_session_id = session_id if isinstance(session_id, str) and session_id else None
session_start_source = hook_input.get("source") if hook_event_name == "SessionStart" else None
if not isinstance(session_start_source, str) or not session_start_source:
    session_start_source = None
if agent_session_id:
    params = {
        "pane_id": pane_id,
        "source": source,
        "agent": "codex",
        "seq": report_seq,
        "agent_session_id": agent_session_id,
    }
    if session_start_source:
        params["session_start_source"] = session_start_source
    request = {
        "id": request_id,
        "method": "pane.report_agent_session",
        "params": params,
    }
else:
    raise SystemExit(0)

try:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(0.5)
    client.connect(socket_path)
    client.sendall((json.dumps(request) + "\n").encode())
    try:
        client.recv(4096)
    except Exception:
        pass
    client.close()
except Exception:
    pass
PY
