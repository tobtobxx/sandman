# Sandman

An agent swarm. Agents talk to a human and complete or investigate work, coordinating
through a shared queue rather than a command hierarchy.

## Core idea

All work is split into atomic **Task** units. Each task spawns a **Session** which
works this task to completion. Either by itself or by spawning subtasks.
Each task/session has one **Role** (eg. planning, research, memory, ...). This role
determines the system prompt and available tools.

The bet is that this rigid way of collaboration reduces unneccessary back-and-forth,
keeps context small (or large context localized to one session - cachable).

Some additional features:
- **Long Channel Sessions**: The sessions directly communicating with the human are
  not spawned from tasks. They keep a history of the user interactions in order to
  form a coherent conversation.
- **Metacognition**: task sessions are concluded with a reflection. Long sessions
  also get interrupts.
- **Observer UI**: The harness spawns an observer UI on port 8080.

## Documents

- [CONTEXT.md](./CONTEXT.md) — the vocabulary. Read first; the code means these words
  precisely.
- [ARCHITECTURE.md](./ARCHITECTURE.md) — components, seams, invariants, file index.
- [docs/benchmarking.md](./docs/benchmarking.md) — the bench, and how to add a case.
- [TASKS.md](./TASKS.md) — known debt, and what to suspect when it misbehaves.
- [AGENTS.md](./AGENTS.md) — how to work in this repo.

`prototype.tar.gzip` is where this design came from. Kept for reference, not built.
Only very seldomly needed now. Avoid looking at it.

Keep all these docs files terse. Important commands, simple description, ~100 lines limit.

## Run it

Start the harness:
```sh
cargo run --bin sandman
```

A Task into a Sandman already running — this is how cron, an RSS script or a mail watcher
gets work in. Prints the Task's id and exits:
```sh
cargo run --bin sandman -- task \
  --role research \
  --title "Check the weather" \
  --brief "Find tomorrow's forecast for Bern and say what to wear." \
  --at 600
```

## Produced files

A run writes into the working directory. All of it is gitignored, and deleting all
of it gives you a fresh Sandman.

- `sandman.sqlite` — every Task, Session, transcript, model call and TaskResult.
- `sandman.log` — one line per Event: the order in which events happened.
  Truncated at each start. `--verbose` writes the bodies out too.
- `sandman.sock` — the control socket `sandman task` talks to. Lives in
  `$XDG_RUNTIME_DIR` when there is one, beside the database otherwise. Only the
  interactive harness opens it, and a stale one from a killed process is replaced.

`--db`, `--log` and `--socket` move each of the three.

## Test it

```sh
cargo test                           # spends nothing
cargo test -- --ignored              # the bench cases, against a real model
cargo run --bin bench -- --times 5   # with a report and artifacts
```

## Configuration

Almost none, on purpose. Model and API key in `src/model.rs`, port in `src/web/mod.rs`.
`OPENROUTER_API_KEY` and `SANDMAN_REASONING_EFFORT` override at run time. The key
committed here is limited and a leak costs nothing.
