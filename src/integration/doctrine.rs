// Single source of truth for the herdr delegation doctrine. The same text is
// installed into every agent's global instructions file (claude, codex,
// opencode) via a herdr-managed fenced block, and is taught in SKILL.md, so
// orchestrators and workers on any of those runtimes see identical guidance
// instead of each agent improvising its own polling loop.
pub(crate) const DELEGATION_DOCTRINE: &str = r#"## herdr delegation doctrine

read this whether you are the orchestrator spawning workers or a worker
spawned by one, in a herdr-managed pane.

### as orchestrator

- spawn workers with the prompt as argv — one atomic command, never
  launch-then-type. typing a prompt into a pane after boot is racy: you can
  forget the Enter key, or hit a boot race where the agent is not ready yet.
- claude worker:
  `herdr agent start claude --cwd <dir> --split right --no-focus -- claude "task text — when done your final message is mailed to me automatically"`
- codex worker: always launch with `--cwd /home/joebutler/vault` — the
  vault's folder permissions are what make bypassing approvals/sandbox safe.
  pick model/effort with `-m` and `-c model_reasoning_effort=`:
  `herdr agent start codex --cwd /home/joebutler/vault --split right --no-focus -- codex --dangerously-bypass-approvals-and-sandbox -m <model> -c model_reasoning_effort=<minimal|low|medium|high> "task"`
- opencode worker: the prompt-as-argv subcommand is `run`, not the bare
  `opencode <project>` form:
  `herdr agent start opencode --cwd <dir> --split right --no-focus -- opencode run "task" -m <provider/model>`
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
  display string — pass a pane id or terminal id for reliability. an
  agent-name filter (e.g. `--from codex`) only resolves correctly while
  that agent label is unambiguous (one live pane wearing it); once more
  than one worker shares an agent name, filter by pane/terminal id instead.
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
