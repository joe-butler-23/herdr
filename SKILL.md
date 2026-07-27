---
name: herdr
description: Manage Herdr workspaces, tabs, and non-worker support panes for shells, servers, tests, logs, output reads, and process wait conditions. Use only inside Herdr (`HERDR_ENV=1`). If `HERDR_ENV` is not `1`, stop instead of inspecting or controlling Herdr.
---

# Herdr operator skill

Before using this skill, check that `HERDR_ENV=1`. If it is not exactly `1`, say that you are not running inside a Herdr-managed pane and stop. Do not inspect or control a focused Herdr pane from outside Herdr.

Worker delegation, mail, and lifecycle are owned by Herdr-injected session context; do not use this skill for those operations.

Use this skill to manage:

- workspaces and tabs
- non-worker support shells
- development servers and log streams
- test and build processes
- output-based process readiness

The `herdr` CLI talks to the running Herdr instance over its local Unix socket. For the raw protocol and full API, read the [socket API documentation](https://herdr.dev/docs/socket-api/).

## Concepts

- A workspace is a project context containing one or more tabs.
- A tab is a subcontext containing one or more panes.
- A pane is a terminal split running one process.
- Workspace IDs look like `w1`, tab IDs like `w1:t1`, and pane IDs like `w1:p1`.
- IDs describe the live session. Re-read them from list or create responses before acting instead of relying on earlier context.

## Operating rules

1. Confirm `HERDR_ENV=1`.
2. Run `herdr pane list` before controlling panes. The focused pane is yours.
3. Re-read current IDs before focus, split, run, send, or close operations.
4. Parse new IDs from command JSON instead of guessing.
5. Keep your pane focused when creating support panes unless the user explicitly asks otherwise. Use `--no-focus`.
6. Treat an `nvim` pane as the user's notes pane. Never send input to it unless explicitly asked. Before ending your turn, read its visible content and incorporate any notes addressed to you.
7. Use pane read, run, send, wait, and close commands only on confirmed non-worker support panes. Do not use them to coordinate with an agent.

## Inspect the live layout

```bash
herdr pane list
herdr workspace list
herdr tab list --workspace w1
```

## Manage tabs

```bash
herdr tab create --workspace w1 --no-focus
herdr tab create --workspace w1 --label "logs" --no-focus
herdr tab rename w1:t2 "logs"
herdr tab focus w1:t2
herdr tab close w1:t2
```

Re-read the tab list immediately before focus or close.

## Read a support pane

After confirming that the target is a non-worker support pane:

```bash
herdr pane read w1:p2 --source recent --lines 50
```

- `--source visible` reads the current viewport.
- `--source recent` reads rendered recent scrollback.
- `--source recent-unwrapped` joins soft-wrapped recent terminal text.
- `--format ansi` preserves terminal styling when visual state matters.

## Split a support pane and run a process

Parse the new pane ID from the split response:

```bash
NEW_PANE=$(herdr pane split w1:p1 --direction right --no-focus | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')
herdr pane run "$NEW_PANE" "npm run dev"
```

Split downward when that better fits the layout:

```bash
herdr pane split w1:p1 --direction down --no-focus
```

## Wait for process output

Use output waits only for process conditions such as server readiness or a test run ending:

```bash
herdr wait output w1:p2 --match "ready on port 3000" --timeout 30000
herdr wait output w1:p2 --match "server.*ready" --regex --timeout 30000
```

Exit code `1` means the wait timed out. Report the timeout, then inspect recent output before deciding what happened.

## Send input to a support shell

After re-reading the live layout and confirming the target is a non-worker support shell:

```bash
herdr pane send-text w1:p2 "echo hello"
herdr pane send-keys w1:p2 Enter
herdr pane run w1:p2 "echo hello"
```

`pane run` sends text followed by Enter in one request.

## Manage workspaces

```bash
herdr workspace create --cwd /path/to/project --no-focus
herdr workspace create --cwd /path/to/project --label "api server" --no-focus
herdr workspace focus w2
herdr workspace rename w1 "api server"
herdr workspace close w2
```

Re-read the workspace list immediately before focus or close.

## Close a support pane

After re-reading the live layout and confirming the target is a non-worker support pane:

```bash
herdr pane close w1:p2
```

## Recipes

Run a server and wait until it is ready:

```bash
NEW_PANE=$(herdr pane split w1:p1 --direction right --no-focus | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')
herdr pane run "$NEW_PANE" "npm run dev"
herdr wait output "$NEW_PANE" --match "ready" --timeout 30000
herdr pane read "$NEW_PANE" --source recent --lines 20
```

Run tests in a support pane and inspect the result:

```bash
NEW_PANE=$(herdr pane split w1:p1 --direction down --no-focus | python3 -c 'import sys,json; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')
herdr pane run "$NEW_PANE" "cargo test"
herdr wait output "$NEW_PANE" --match "test result" --timeout 60000
herdr pane read "$NEW_PANE" --source recent --lines 30
```

## CLI output notes

- Workspace, tab, pane-list, pane-get, pane-split, and output-wait commands print JSON on success.
- `pane read` prints text.
- Pane send and run commands print nothing on success.
- Create and split responses contain the new IDs; parse them rather than predicting them.
- Use `pane read` for output that already exists and `wait output` for output expected next.
- `--no-focus` on split, tab-create, and workspace-create preserves the current terminal context.
