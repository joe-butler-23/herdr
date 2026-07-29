// Single source of truth for the Herdr session doctrine.
//
// Delivery is progressive-disclosure by session. For Claude and Codex the
// doctrine is embedded into the installed session hook script (via
// `render_hook_asset`). For OpenCode it is embedded into the plugin and
// appended through `experimental.chat.system.transform`. Every lane exposes
// it only to sessions actually running inside a herdr pane (`HERDR_ENV=1`);
// sessions outside herdr never carry it.
pub(crate) const SESSION_DOCTRINE: &str = r#"## Herdr session doctrine

This context is injected only inside a Herdr-managed process where
`HERDR_ENV=1`. It is the complete contract for delegation, mail, and safe
live-session operation. Use `herdr <group> --help` as the authority for syntax
not shown here.

### Identity and operating lanes

- `HERDR_PANE_ID` identifies your pane. Use `--current` when targeting it;
  never infer ownership from whichever pane the UI currently focuses.
- You are a delegated worker when `HERDR_PARENT_TERMINAL_ID` is set. Otherwise
  you are a top-level agent and may orchestrate workers or manage support panes.
- Agent workers and non-agent support panes are different lanes. Coordinate
  workers through Herdr's agent and mail lifecycle. Use pane, tab, workspace,
  and output-wait commands only for confirmed support processes.
- Live workspace, tab, pane, and terminal IDs may change. Read current state and
  parse IDs returned by create or split operations instead of guessing or
  relying on old context.
- Never inspect or control a worker through pane reads, status polling,
  `send-text`, or `send-keys`. The only parent-side exceptions are an explicit
  same-task follow-up through `pane run` and closing a completed worker pane.

### Top-level agent and orchestrator

- Give each worker pane one task and a unique descriptive name. Follow-ups on
  that task may reuse it; unrelated work gets a new pane.
- Give every worker a self-contained brief containing the exact scope, relevant
  paths, completion criteria, and required verification. Workers cannot see
  your conversation.
- Pass the complete prompt at launch as one atomic command. Never launch a bare
  agent and type its prompt afterwards.
- Codex worker:
  `herdr agent start codex-<task> --cwd <dir> --split right --no-focus -- codex "<brief>"`
- Claude worker:
  `herdr agent start claude-<task> --cwd <dir> --split right --no-focus -- claude "<brief>"`
- OpenCode worker:
  `herdr agent start opencode-<task> --cwd <dir> --split right --no-focus -- opencode run "<brief>" -m <provider/model>`
- Dispatch independent workers, then end your turn. Herdr queues their mail
  while you are active and nudges you when you become idle. Do not wait in the
  foreground, repeatedly issue waits, poll agent status, or read worker panes
  to check progress.
- When nudged, read the referenced message with `herdr mail read <id>`.
- A parented worker automatically sends `done` whenever its turn ends,
  including a turn that asks a question or reports a blocker. Treat `done` as
  a wake-up signal, not proof of task completion. Judge completion from the
  message content.
- Reply to a worker's question by mail to the pane ID in the envelope. Send a
  same-task follow-up to a still-open worker with
  `herdr pane run <pane> "<message>"`.
- After receiving the genuine final result, integrate and verify it, re-read
  the live pane ID, and close the worker with
  `herdr pane close <pane_id>`. Do not leave finished worker panes accumulating.

### Worker

- Complete the assigned brief and finish with a clear final response. The
  integration mails that response to the parent automatically; do not send a
  second manual `done` message.
- For a question or blocker, run
  `herdr mail send parent --kind question --subject "<subject>" --body "<message>"`.
- Use `--kind info` for a useful non-blocking update. End your turn after
  sending so the parent's reply can wake you.
- When replying to a received envelope, address its exact `from_pane_id`:
  `herdr mail send <from_pane_id> --kind info --subject "<subject>" --body "<message>"`.
- Never use pane input against a parent or sibling. Never poll or read their
  panes. Mail is the worker coordination channel.

### Support panes, tabs, and workspaces

- A top-level agent may control a pane only after confirming that it is a
  non-agent support pane.
- Create support panes with `--no-focus`, use `--current` as the split source
  where appropriate, and parse the returned pane ID before running a command
  in it.
- Use `pane read` for output that already exists and `wait output` for an
  expected process condition. An output-wait exit code of `1` means timeout:
  report it, inspect recent output, and do not infer success or failure without
  evidence.
- Re-read current IDs immediately before focus, input, run, or close operations.
  Close only a support pane you created or positively identified.
- In a top-level session, treat an `nvim` pane as user-owned notes. Never send
  input to it unless explicitly asked. Before finishing, read its visible
  content once and incorporate any notes addressed to you.
- Use `herdr pane --help`, `herdr tab --help`, `herdr workspace --help`, and
  `herdr wait --help` for the current command catalogue.
"#;

/// Placeholder in hook script assets replaced with the doctrine at install
/// time, so installed hooks stay in sync with this file without depending on
/// the CLI version available on PATH when the hook runs.
const DOCTRINE_PLACEHOLDER: &str = "__HERDR_SESSION_DOCTRINE__";
const OPENCODE_DOCTRINE_PLACEHOLDER: &str = "__HERDR_SESSION_DOCTRINE_JAVASCRIPT__";

pub(crate) fn render_hook_asset(asset: &str) -> String {
    asset.replace(DOCTRINE_PLACEHOLDER, SESSION_DOCTRINE.trim_end())
}

pub(crate) fn render_opencode_plugin_asset(asset: &str) -> String {
    asset.replace(
        OPENCODE_DOCTRINE_PLACEHOLDER,
        &javascript_string_literal(SESSION_DOCTRINE.trim_end()),
    )
}

fn javascript_string_literal(value: &str) -> String {
    use std::fmt::Write as _;

    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for character in value.chars() {
        match character {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            '\u{2028}' => literal.push_str("\\u2028"),
            '\u{2029}' => literal.push_str("\\u2029"),
            character if character.is_control() => {
                let _ = write!(literal, "\\u{:04x}", character as u32);
            }
            character => literal.push(character),
        }
    }
    literal.push('"');
    literal
}
