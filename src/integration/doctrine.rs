// Single source of truth for the herdr delegation doctrine.
//
// Delivery is progressive-disclosure by session. For Claude and Codex the
// doctrine is embedded into the installed session hook script (via
// `render_hook_asset`). For OpenCode it is embedded into the plugin and
// appended through `experimental.chat.system.transform`. Every lane exposes
// it only to sessions actually running inside a herdr pane (`HERDR_ENV=1`);
// sessions outside herdr never carry it.
pub(crate) const DELEGATION_DOCTRINE: &str = r#"## herdr delegation doctrine

this section applies only inside a herdr-managed pane: `HERDR_ENV=1` or
`HERDR_PARENT_TERMINAL_ID` set in your environment. outside herdr, ignore
this entire section.

this doctrine is complete for spawning workers, worker mail, and closing
worker panes — do not load the herdr skill for those. load the skill only
for pane work beyond delegation: splits, sibling shells, servers, log
streams, wait conditions, workspace/tab management.

read this whether you are the orchestrator spawning workers or a worker
spawned by one.

### as orchestrator

- panes are single-use. one task per worker pane: spawn it, integrate its
  result, close it. a new unrelated task gets a new pane, never a recycled
  one. follow-ups on the same task may continue in the still-open pane.
- name every worker pane for its task — the name is the first argument to
  `agent start`: `codex-kanban`, `codex-arh-review`, `claude-docs-sweep`.
  unique per-task names keep `--from <name>` mail filters unambiguous and
  make pane targeting readable.
- a worker prompt is a self-contained brief: name the exact files or
  paths, the done criteria, and how the worker should verify its work.
  never reference your own conversation — the worker cannot see it.
- codex model routing: `gpt-5.6-terra` is the default — most work goes
  there. `gpt-5.6-luna` is fast and affordable yet intelligent — use it
  for well-defined, high-volume work. `gpt-5.6-sol` is the most
  intelligent — reserve it for adversarial review, second opinions, and
  tricky debugging.
- spawn workers with the prompt as argv — one atomic command, never
  launch-then-type. typing a prompt into a pane after boot is racy: you can
  forget the Enter key, or hit a boot race where the agent is not ready yet.
- claude worker:
  `herdr agent start claude-<task> --cwd <dir> --split right --no-focus -- claude "task text — when done your final message is mailed to me automatically"`
- codex worker: always launch with `--cwd /home/joebutler/vault` — the
  vault's folder permissions are what make bypassing approvals/sandbox safe.
  pick model/effort with `-m` and `-c model_reasoning_effort=`:
  `herdr agent start codex-<task> --cwd /home/joebutler/vault --split right --no-focus -- codex --dangerously-bypass-approvals-and-sandbox -m <model> -c model_reasoning_effort=<minimal|low|medium|high> "task"`
- opencode worker: the prompt-as-argv subcommand is `run`, not the bare
  `opencode <project>` form:
  `herdr agent start opencode-<task> --cwd <dir> --split right --no-focus -- opencode run "task" -m <provider/model>`
- THE method, correct for every agent on every harness: dispatch your
  workers, then end your turn. herdr wakes you by typing a `[herdr mail]`
  notice into your pane the moment a worker's mail arrives (queued while
  you are mid-turn, delivered the instant you go idle) — there is nothing
  to wait for and nothing to poll. rationale in one line: a foreground
  wait blocks your whole turn and burns its duration for nothing the nudge
  doesn't already give you.
- the ONLY exception: if your harness can run a shell command in the
  background and notify you when it exits, you MAY background `herdr mail
  wait --from <worker> --timeout <ms>` instead, then `herdr mail read
  <id>` when it fires. claude code can do this; codex and opencode cannot
  — on those harnesses always end your turn instead.
- hard prohibitions, no exceptions: never run `mail wait` in the
  foreground — it blocks your turn for the full timeout. never treat
  `--timeout` as a polling interval to re-issue waits against. never
  re-issue waits in a loop, and never poll pane read/status to check on a
  worker. if you find yourself picking a `--timeout` value for
  coordination, you are doing it wrong — end your turn instead.
- a parented worker's hook auto-mails `done` at the end of EVERY turn,
  including a turn where it only asked you a question or reported it is
  blocked — that `done` mail is not the real completion. judge completion
  by the mail's content, not just its kind: an interim `done` still reads
  like a question/status update, and the real completion mail arrives only
  after you reply and the worker finishes its next turn.
- `--from <worker>` filters by the sender's terminal identity, not a
  display string. unique per-task pane names keep an agent-name filter
  (e.g. `--from codex-kanban`) reliable; if a name is ever shared by more
  than one live pane, filter by pane/terminal id instead.
- to send a follow-up to a worker that is still RUNNING, use the typed
  channel: `herdr pane run <pane> "message"`. argv only works at launch.
- more than one worker in flight: either issue one `mail wait --from
  <worker>` per worker you care about, or do an unfiltered wait and read
  whatever arrives.
- `mail wait` does not mark mail read. always `herdr mail read <id>` for
  whatever woke you, or pass `--consume` on the wait itself.
- once you have read a worker's final done-mail and integrated/verified its
  work, close its pane: `herdr pane close <pane_id>`. skip this only if you
  are about to reuse that same worker for follow-up work. finished panes
  left open accumulate, confuse later pane targeting, and keep sessions
  resident.

### as worker

you are a worker if `HERDR_PARENT_TERMINAL_ID` is set in your environment.

- your completion mails itself. with the integration installed, your
  agent's own hook sends the orchestrator a `done` mail automatically when
  your turn ends — just end your turn normally with a clear final message.
  do not also call `mail send parent --kind done` yourself.
- interim question or status while you are still working:
  `herdr mail send parent --kind question|info --subject "..." --body "..."`,
  then end your turn. the reply wakes you the same way completion wakes the
  orchestrator.
- never `pane run`/`pane send-text` into your parent's pane or any other
  pane to coordinate — that types the message in as if it were fake user
  input to whatever is running there, it is not a mail delivery. reply by
  mail to the pane id shown in the envelope you were nudged with:
  `herdr mail send <from_pane_id> --kind info --subject "..." --body "..."`.
- never poll your parent's panes, and never read sibling panes to
  coordinate. mail is the only channel.
"#;

/// Placeholder in hook script assets replaced with the doctrine at install
/// time, so installed hooks stay in sync with this file without depending on
/// the CLI version available on PATH when the hook runs.
const DOCTRINE_PLACEHOLDER: &str = "__HERDR_DELEGATION_DOCTRINE__";
const OPENCODE_DOCTRINE_PLACEHOLDER: &str = "__HERDR_DELEGATION_DOCTRINE_JAVASCRIPT__";

pub(crate) fn render_hook_asset(asset: &str) -> String {
    asset.replace(DOCTRINE_PLACEHOLDER, DELEGATION_DOCTRINE.trim_end())
}

pub(crate) fn render_opencode_plugin_asset(asset: &str) -> String {
    asset.replace(
        OPENCODE_DOCTRINE_PLACEHOLDER,
        &javascript_string_literal(DELEGATION_DOCTRINE.trim_end()),
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
