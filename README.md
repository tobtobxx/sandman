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

First start finds no configuration: it writes the commented default to
`$XDG_CONFIG_HOME/sandman/config.toml` and stops. Read it and start again —
every key is required, an unknown key is an error. `--config <path>` names
another file, all state paths included.

Start the harness:
```sh
cargo run --bin sandman
```

`/quit` or Ctrl+D leaves cleanly: nothing new starts, the last calls land, the **Run** is
marked ended. Ctrl+C aborts — the process dies with its calls in flight. The next start
ends what a dead Run left open: Tasks marked running, Sessions, queued and in-flight
calls. Pending Tasks survive a restart; Sessions never do.

One Sandman per database. A second start on one database — including `sandman run` —
is refused while the first is live. A lock left by a dead Sandman clears itself; pass
`--break-lock` for the case it does not.

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

Paths come from `[sandman]`; the defaults are under the XDG directories, not the
working directory. Deleting all of it gives you a fresh Sandman.

- `$XDG_STATE_HOME/sandman/sandman.sqlite` — every Task, Session, transcript, model
  call and TaskResult.
- `$XDG_STATE_HOME/sandman/sandman.log` — one line per Event: the order in which
  events happened. Truncated at each start. `--verbose` writes the bodies out too;
  with the terminal not a Channel (`[channels].stdio = false`) the trace also goes
  to stdout.
- `$XDG_RUNTIME_DIR/sandman/sandman.sock` — the control socket `sandman task`
  talks to. Only the interactive harness opens it, and a stale one from a killed
  process is replaced.
- `$XDG_CONFIG_HOME/sandman/config.toml` — the configuration, written once on
  first start.

## Test it

```sh
cargo test                           # spends nothing
cargo test -- --ignored              # the bench cases, against a real model
cargo run --bin bench -- --times 5   # with a report and artifacts
```

## Configuration

One file: `--config`, else `$XDG_CONFIG_HOME/sandman/config.toml`. Read once at
start; the only place anything is configured — models per Purpose, state paths,
embedding, Channels, tool endpoints, the bench grader.
`src/default-config.toml` is the template and the documentation. Any string may
name a `$VAR`; a variable not set is an error. The API key in the default is a
limited prototype one; a leak costs nothing.
