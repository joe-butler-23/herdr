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
- wait for the result with `herdr mail wait --from <worker> --timeout <ms>`,
  then `herdr mail read <id>`. spend zero tokens while waiting, two ways:
  background the wait if your harness notifies you on background-task
  completion (claude code does); otherwise just end your turn right after
  dispatch — herdr wakes you by typing a mail notice into your pane the
  moment the worker's mail arrives (queued while you are mid-turn, delivered
  the instant you go idle). never loop a foreground timeout wait, and never
  poll pane read/status to check on a worker.
- to send a follow-up to a worker that is still RUNNING, use the typed
  channel: `herdr pane run <pane> "message"`. argv only works at launch.
- more than one worker in flight: either issue one `mail wait --from
  <worker>` per worker you care about, or do an unfiltered wait and read
  whatever arrives.
- `mail wait` does not mark mail read. always `herdr mail read <id>` for
  whatever woke you, or pass `--consume` on the wait itself.

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
- never poll your parent's panes, and never read sibling panes to
  coordinate. mail is the only channel.
"#;
