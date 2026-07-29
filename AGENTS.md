# Purpose — herdr

`herdr` is the user’s maintained fork of a terminal-native workspace manager and persistent session runtime, especially for AI coding agents. It exists to make concurrent agents and supporting processes reliable, observable, fast, and low-overhead across local and remote use. Optimise it for the user’s environment and workflows; do not assume upstream-contribution, public-distribution, or release responsibilities unless explicitly requested.

Success means processes and sessions behave predictably across attachment, detachment, restoration, and handoff; the TUI, CLI, API, and provider integrations present coherent current state; failures and incompatibilities surface clearly; and the fork remains lean enough to maintain and update safely.

<!-- clai:instructions:coding:start -->
<!-- source-sha256:125fbd0ba45f15bcd8964ecd8bb5dd139da49002dbaf2db8229a6156593a274e -->
## Engineering Principles

- **Modern and idiomatic:** Use current, supported language, framework, and platform conventions. Match surrounding code when it is sound; do not reproduce obsolete patterns merely for consistency.
- **Lean end state:** Implement the intended final design directly. Remove superseded code, compatibility paths, shims, flags, dependencies, tests, comments, documentation, and configuration unless compatibility or migration is an explicit requirement. Git preserves history; current files describe only the current system.
- **Simple and explicit:** Use the least code and fewest moving parts that solve the problem. Prefer clear contracts, bounded resources, observable state, and existing project or platform primitives over speculative abstractions.
- **Efficient by design:** Avoid repeated work and unnecessary process, file, database, or network round trips. Reuse long-lived resources, batch small operations, stream large inputs, and keep concurrency, buffering, and retries bounded.
- **Evidence-led performance:** Set budgets and measure realistic workloads before optimizing. Fix algorithms, I/O, contention, and lifecycle design before micro-optimizing.
- **Risk-proportionate verification:** Define success before editing. Run the cheapest sufficient checks first and escalate according to risk. Bugs require regression coverage, and completion requires evidence at the surface the user cares about.
- **Timing and state:** Use time to model time, not to infer state. When work involves polling, debounce, readiness, timeouts, TTLs, cooldowns, throttling, retries, scheduling, animation timing, or event delivery, load the `timer-inference` skill.
<!-- clai:instructions:coding:end -->

## Instruction Ownership

- This file owns the contract for changing the Herdr fork. It does not own instructions for operating inside a Herdr session.
- `src/integration/session_doctrine.rs` owns the session doctrine: a compact block of operating instructions that tells an agent whether it is an orchestrator or worker, how to launch and close workers, coordinate through mail without polling, handle live IDs, and control non-agent support panes safely.
- Herdr renders that doctrine into the managed Claude Code, Codex, and OpenCode integrations. They inject it automatically when `HERDR_ENV=1`; agents should follow it as the live-session operating contract and use `herdr <group> --help` when they need exact command syntax.
- Provider hook and plugin assets are delivery adapters. They may implement provider-specific lifecycle mechanics but must not contain independent instructional prose.
- The `justfile` owns development command entry points. Live `herdr <group> --help` output owns CLI syntax. Do not duplicate either as long command catalogues here.

## Architecture Boundaries

- Keep testable state separate from live runtime. `AppState` and `TerminalState` own durable state and metadata; `TerminalRuntimeRegistry` owns PTYs, parsers, detector tasks, and channels. `PaneState` is a viewport attachment to a durable `TerminalId`; layout position and pane identity must not become terminal identity.
- The server owns shared session, process, terminal, persistence, and agent facts. Clients own input and presentation. Expose shared automation through the public JSON API and events rather than increasing dependence on private TUI/client transport.
- Compute geometry and reconcile terminal sizes before drawing. Rendering must remain side-effect-free. Reuse established TUI interactions and affordances instead of introducing isolated patterns.
- Each terminal has one effective agent-status authority. A live full-lifecycle integration is authoritative while reporting; otherwise fallback detection uses the current bottom-buffer snapshot and manifests, never a user-scrolled viewport.
- Detachment, snapshot restoration, and live handoff are distinct lifecycle paths. Detachment preserves a running server, restoration reconstructs saved state, and handoff transfers live runtimes. Test every affected path independently.
- Keep substantial operating-system behaviour under `src/platform/` and compile-gate narrower platform-specific exceptions at the smallest practical boundary.

## Compatibility and Generated State

- Treat public IDs, persisted snapshots, API schemas, and the client/server wire protocol as explicit compatibility contracts. When one changes, update its version, fixtures, migrations, generated artefacts, and round-trip or cross-version tests as appropriate.
- `src/protocol/wire.rs` owns `PROTOCOL_VERSION`; bincode enum order and field layout are compatibility-sensitive.
- API schema types own `docs/next/api/herdr-api.schema.json`. Regenerate it with `HERDR_UPDATE_API_SCHEMA=1 just test-one generated_protocol_schema_artifact_is_current` and review the resulting diff.
- `src/integration/session_doctrine.rs` must render identically into every provider integration. Integration checks must compare exact installed content and required registrations, not only version markers.
- Agent-detection changes require live detection-source evidence and explanation output. Match stable controls through explicit gates rather than incidental full-pane text.
- Vendored-source updates must reconcile the owning vendor manifest, patch index, patch files, and their maintenance tests. Remove patches already incorporated upstream.

## Verification

- Use `just test-one <filter>` while iterating.
- For a narrow completed change, run the affected tests and `just lint`. Use `just ci` for an ordinary cross-module or feature tranche.
- Use `just check` for broad, platform-sensitive, protocol, persistence, handoff, detection, vendor, maintenance-script, or release-risk changes.
- Before identity, state, protocol, persistence, or broad refactors, identify or add characterization coverage. Use existing adversarial-state fixtures and invariant assertions where applicable.
- When testing a source build from inside an existing Herdr session, use `env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH cargo run -- <command>` so the debug binary cannot silently target the installed server.
- Directly exercise affected TUI, remote, attachment, restoration, handoff, integration, or agent-status behaviour before describing it as complete.
