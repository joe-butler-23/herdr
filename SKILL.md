---
name: herdr
description: "Control herdr from inside it. Manage workspaces and tabs, split panes, spawn agents, read output, and wait for state changes — all via CLI commands that talk to the running herdr instance over a local unix socket. Use when running inside herdr (HERDR_ENV=1)."
---

# herdr — agent skill

before using this skill, check that `HERDR_ENV=1`. if it is not set to `1`, then you are not running inside a herdr-managed pane and stop. that is fine, but it may impact your ability to test functionality.

if you are running inside herdr already, then herdr is a terminal-native agent multiplexer. herdr gives you workspaces, tabs, and panes — each pane is a real terminal with its own shell, agent, server, or log stream — and you can control all of it from the cli.

this means you can:

- see what other panes and agents are doing
- create tabs for separate subcontexts inside one workspace
- split panes and run commands in them
- start servers, watch logs, and run tests in sibling panes
- wait for specific output before continuing
- wait for another agent to finish
- spawn more agent instances

the `herdr` binary is available in your PATH. its workspace, tab, pane, and wait commands talk to the running herdr instance over a local unix socket.

if you need the raw protocol or full api reference, read the [socket api docs](https://herdr.dev/docs/socket-api/).

## concepts

**workspaces** are project contexts. each workspace has one or more tabs. unless manually renamed, a workspace's label follows the first tab's root pane — usually the repo name, otherwise the root pane's current folder name.

**tabs** are subcontexts inside a workspace. each tab has one or more panes.

**panes** are terminal splits inside a tab. each pane runs its own process — a shell, an agent, a server, anything.

**agent status** is detected automatically by herdr. the api exposes one public field for it:

- `agent_status` — `idle`, `working`, `blocked`, `done`, `unknown`

`done` means the agent finished, but you have not looked at that finished pane yet.

plain shells still exist as panes, but herdr's sidebar agent section intentionally focuses on detected agents rather than listing every shell.

**ids** — workspace ids look like `1`, `2`. tab ids look like `1:1`, `1:2`, `2:1`. pane ids look like `1-1`, `1-2`, `2-1`. these are compact public ids for the current live session.

important: ids can compact when tabs, panes, or workspaces are closed. do not treat them as durable ids. re-read ids from `workspace list`, `tab list`, `pane list`, or create/split responses when you need a current id. do not guess that an older `1-3` is still the same pane later.

## discover yourself

see what panes exist and which one is focused:

```bash
herdr pane list
```

the focused pane is yours. other panes are your neighbors.

list workspaces:

```bash
herdr workspace list
```

## tab management

list tabs in the current workspace:

```bash
herdr tab list --workspace 1
```

create a new tab:

```bash
herdr tab create --workspace 1
```

without `--label`, the new tab keeps the default numbered tab name.

create and name it in one step:

```bash
herdr tab create --workspace 1 --label "logs"
```

rename it:

```bash
herdr tab rename 1:2 "logs"
```

focus it:

```bash
herdr tab focus 1:2
```

close it:

```bash
herdr tab close 1:2
```

## read another pane

see what is on another pane's screen:

```bash
herdr pane read 1-1 --source recent --lines 50
```

- `--source visible` = current viewport
- `--source recent` = recent scrollback as rendered in the pane
- `--source recent-unwrapped` = recent terminal text with soft wraps joined back together

## split a pane and run a command

split your pane to the right and keep focus on your current pane:

```bash
herdr pane split 1-2 --direction right --no-focus
```

that prints json with the new pane nested at `result.pane.pane_id`. parse that value, then run a command in that pane:

```bash
NEW_PANE=$(herdr pane split 1-2 --direction right --no-focus | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')
herdr pane run "$NEW_PANE" "npm run dev"
```

split downward instead:

```bash
herdr pane split 1-2 --direction down --no-focus
```

## wait for output

block until specific text appears in a pane. useful for waiting on servers, builds, and tests.

for `--source recent`, matching uses unwrapped recent terminal text, so pane width and soft wrapping do not break matches. `pane read --source recent` still shows the pane as rendered. if you want to inspect the same transcript that the waiter matches, use `pane read --source recent-unwrapped`.

```bash
herdr wait output 1-3 --match "ready on port 3000" --timeout 30000
```

with regex:

```bash
herdr wait output 1-3 --match "server.*ready" --regex --timeout 30000
```

if it times out, exit code is `1`.

## wait for an agent status

block until another agent reaches a specific status:

```bash
herdr wait agent-status 1-1 --status done --timeout 60000
```

use this when you want the same `done` / `idle` distinction the UI shows.

## send text or keys to a pane

send text without pressing Enter:

```bash
herdr pane send-text 1-1 "hello from claude"
```

press Enter or other keys:

```bash
herdr pane send-keys 1-1 Enter
```

`pane run` sends the text and then a real `Enter` key in one request:

```bash
herdr pane run 1-1 "echo hello"
```

## workspace management

create a new workspace:

```bash
herdr workspace create --cwd /path/to/project
```

without `--label`, the new workspace keeps the default cwd-based name.

create and name one in one step:

```bash
herdr workspace create --cwd /path/to/project --label "api server"
```

create one without focusing it:

```bash
herdr workspace create --no-focus
```

focus a workspace:

```bash
herdr workspace focus 2
```

rename:

```bash
herdr workspace rename 1 "api server"
```

close:

```bash
herdr workspace close 2
```

## close a pane

```bash
herdr pane close 1-3
```

## delegate to another agent (mail)

spawn a worker with its task as argv — never launch a bare agent and then
type the prompt in a second step. launch-then-type is racy: it is easy to
forget the Enter keypress, or to send the prompt before the agent has
finished booting and lose it to a boot race. `herdr agent start` takes the
full command, prompt included, after `--`, so the spawn is one atomic
operation, and it stamps the new pane's parent context for you (its
`HERDR_PARENT_TERMINAL_ID` is set without an explicit `--parent-pane`).

```bash
# claude worker
NEW=$(herdr agent start claude --cwd . --split right --no-focus -- \
  claude "research kubernetes operator patterns — when done your final message is mailed to me automatically" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["agent"]["terminal_id"])')

# codex worker: always launch it in ~/vault — that cwd's folder permissions
# are what make bypassing approvals/sandbox safe. pick model/effort with
# -m and -c model_reasoning_effort= (codex has no literal --yolo flag; this
# is the flag combination its own TUI labels "YOLO mode").
herdr agent start codex --cwd /home/joebutler/vault --split right --no-focus -- \
  codex --dangerously-bypass-approvals-and-sandbox -m <model> \
  -c model_reasoning_effort=<minimal|low|medium|high> "task"

# opencode worker: the prompt-as-argv subcommand is `run`, not the bare
# `opencode <project>` form
herdr agent start opencode --cwd . --split right --no-focus -- \
  opencode run "task" -m <provider/model>
```

mail is the preferred way to get the result back, because it avoids
token-expensive polling. mail always lands in the **recipient's** inbox —
when a worker sends to `parent`, the message is stored in the orchestrator's
own inbox, not the worker's. so the orchestrator always waits/reads/lists
its **own** inbox (the default, taken from `HERDR_PANE_ID` — no `--inbox`
flag needed) and uses `--from <sender>` only to filter which sender's mail
it is willing to match. this matters with more than one worker in flight:
without `--from`, `mail wait` returns the oldest unread mail from ANY
sender, so a wait aimed at one worker can be satisfied by a different
worker's unrelated message. `--from` matches the sender's terminal
identity, not a display string — pass a pane id or terminal id for
reliability; an agent-name filter (e.g. `--from codex`) only resolves
correctly while that agent label is unambiguous (one live pane wearing
it), so switch to pane/terminal id once more than one worker shares an
agent name.

there are two zero-token ways to wait for the result — pick whichever your
harness supports, do not fall back to polling:

```bash
# 1. background the wait, if your harness notifies you on background-task
# completion (claude code does), and keep working until it fires
herdr mail wait --timeout 120000 --from "$NEW" &

# 2. otherwise just end your turn right after dispatch. herdr wakes a
# waiting orchestrator by typing a mail notice into its pane the moment the
# worker's mail arrives — mail sent while you are still mid-turn queues and
# is delivered the instant you go idle.
```

never loop a foreground `mail wait` with retries, and never poll `pane
read`/`wait agent-status` to check on a worker instead — both defeat the
point of mail. when you need the result, read just the envelope first
(`mail read` has no sender filter — it looks up an exact id in your own
inbox by default), then the body only if it is worth the token cost:

```bash
ENVELOPE=$(herdr mail read <id>)
herdr mail read <id> | jq .body
```

`mail wait` does not mark mail read — always follow it with `herdr mail
read <id>` for whatever woke you, or pass `--consume` on the wait itself.

to message a worker that is still RUNNING (not at launch), use the typed
channel instead: `herdr pane run <pane> "message"`. argv-as-prompt only
works at spawn time. that channel is for the orchestrator to reach a
worker only — a worker must never `pane run`/`pane send-text` into its
parent's pane (or any other pane) to reply or coordinate: that types the
message in as fake user input to whatever is running there, it is not a
mail delivery. a worker replies by mail, to the pane id shown in the
envelope it was nudged with.

worker sends a reply by calling `herdr mail send parent --kind done --subject "..." --body-stdin` (resolve `parent` in the CLI from `HERDR_PARENT_TERMINAL_ID` or `HERDR_PARENT_PANE_ID`; the server has no notion of "parent"). for an interim question or status while still working, send `--kind question|info` instead and then end the turn — the reply wakes the worker the same way completion wakes the orchestrator.

with the claude/codex/opencode integrations installed, delegated workers
(panes spawned with a parent context) automatically send `done` mail when
their turn finishes and `blocked` mail when awaiting permission — a worker
does not need to send its own `done` mail, ending the turn is enough. this
fires at the end of EVERY turn, including a turn where the worker only
asked a question or reported a block, so an orchestrator should expect an
interim `done` mail that is not the real completion; judge completion by
the mail's content, not just its `done` kind — the real completion mail
arrives only after the orchestrator replies and the worker finishes its
next turn. standalone panes (no parent) never send automatic mail. `herdr
integration install` also installs this delegation doctrine into
claude/codex/opencode's global instructions, so a worker running any of
those agents already knows these rules without being told them in its
task prompt.

## recipes

### run a server and wait until it is ready

```bash
NEW_PANE=$(herdr pane split 1-2 --direction right --no-focus | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')
herdr pane run "$NEW_PANE" "npm run dev"
herdr wait output "$NEW_PANE" --match "ready" --timeout 30000
herdr pane read "$NEW_PANE" --source recent --lines 20
```

### run tests in a separate pane and inspect the result

```bash
herdr pane split 1-2 --direction down --no-focus
herdr pane run 1-3 "cargo test"
herdr wait output 1-3 --match "test result" --timeout 60000
herdr pane read 1-3 --source recent --lines 30
```

### check what another agent is working on

```bash
herdr pane list
herdr pane read 1-1 --source recent --lines 80
```

### watch another pane robustly

use this pattern when you need to coordinate with a sibling pane:

```bash
# inspect what is already there
herdr pane read 1-3 --source recent --lines 40

# wait only for the next output you expect
herdr wait output 1-3 --match "ready" --timeout 30000

# if you need to inspect the same transcript the waiter matched,
# read the unwrapped recent text directly
herdr pane read 1-3 --source recent-unwrapped --lines 40
```

### spawn a new agent and give it a task

```bash
NEW=$(herdr agent start claude --cwd . --split right --no-focus -- \
  claude "review the test coverage in src/api/ — when done your final message is mailed to me automatically" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["agent"]["terminal_id"])')
herdr mail wait --timeout 120000 --from "$NEW"
```

the prompt is argv, not typed in after boot — one atomic spawn, no race on
the Enter key or the agent's boot time. the `mail wait` blocks until the
worker sends `done` mail (automatic with current integrations when its turn
completes); `--from "$NEW"` filters to that sender so a different in-flight
worker's mail can't satisfy this wait, and the wait runs against your own
default inbox since no `--inbox` is given. background this wait if your
harness notifies on background-task completion; otherwise end your turn
right after dispatch and let herdr's mail-arrival nudge wake you instead of
blocking. use this to coordinate with the agent without ever polling its
screen.

### coordinate with another agent

prefer the mail loop (above) to avoid polling. if you must check an agent's status screen-based, use `wait agent-status` as a fallback for panes without mail support, but note that `done` only reliably occurs when the pane is not being viewed; `idle` is the more stable completion signal when you are watching the pane in the active tab.

```bash
herdr wait agent-status 1-1 --status idle --timeout 120000
herdr pane read 1-1 --source recent --lines 100
```

## notes

- `workspace list`, `workspace create`, `tab list`, `tab create`, `tab get`, `tab focus`, `tab rename`, `tab close`, `pane list`, `pane get`, `pane split`, `wait output`, `wait agent-status`, `mail send`, `mail wait`, `mail read`, and `mail list` print json on success.
- for `mail wait`/`mail list`/`mail read`, `--inbox` selects whose inbox to act on (default: your own pane, from `HERDR_PANE_ID`); `mail wait`/`mail list` additionally take `--from <sender>` to filter to one sender's mail (a pane id, agent name, or terminal id) — `--from` never changes whose inbox is read, it only narrows which messages in that inbox count.
- `pane read` prints text, not json.
- `pane read --format ansi` or `pane read --ansi` returns a rendered ANSI snapshot for TUI feedback loops.
- `pane read --source recent-unwrapped` is useful when you want to inspect the same unwrapped transcript that `wait output --source recent` matches against.
- `pane send-text`, `pane send-keys`, and `pane run` print nothing on success.
- parse ids from `workspace create`, `tab create`, and `pane split` responses when you need new ids. `workspace create` returns `result.workspace`, `result.tab`, and `result.root_pane`. `tab create` returns `result.tab` and `result.root_pane`. for `pane split`, the new pane id is at `result.pane.pane_id`.
- use `pane read` for current output that already exists. use `wait output` for future output you expect next.
- `--no-focus` on split, tab create, and workspace create keeps your current terminal context focused.
- without `--label`, workspace create keeps cwd-based naming and tab create keeps numbered naming.
- `--label` on tab create and workspace create applies the custom name immediately.
- if you are running inside herdr, the `HERDR_ENV` environment variable is set to `1`.
