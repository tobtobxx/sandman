# Sandman

An agent swarm. Agents talk to a human and complete or investigate work, coordinating
through a shared queue rather than a command hierarchy.

## The bet

**Nothing waits on the queue.** One kind of work — a Task — and one kind of Worker. A
Worker needing an answer holds inside a tool call and carries on where it stopped,
remembering why it asked. No suspended contexts to rebuild, no deadlocked queue.

The price: a Worker starts fresh and sees only its Brief, so every piece of context has
to be written down by whoever created the Task. This exists to find out whether that
trade is worth making, and where it first hurts.

Two things follow. **Metacognition** reads a Worker's conversation and writes the Task's
answer (a review), or fires mid-work every 15 messages to ask whether the Session is
looping, done, stuck or off goal (an interrupt) — it can push a Session on, but stops
nothing, so a human watching is still the guard rail. And **orchestration is plain code**,
so nothing in the swarm decides what runs next.

## Documents

- [CONTEXT.md](./CONTEXT.md) — the vocabulary. Read first; the code means these words
  precisely.
- [ARCHITECTURE.md](./ARCHITECTURE.md) — components, seams, invariants, file index.
- [docs/benchmarking.md](./docs/benchmarking.md) — the bench, and how to add a case.
- [TASKS.md](./TASKS.md) — known debt, and what to suspect when it misbehaves.
- [AGENTS.md](./AGENTS.md) — how to work in this repo.

`typescript-prototype/` is where this design came from. Kept for reference, not built.

## Run it

```sh
cargo run --bin sandman
```

That opens two Channels — the terminal and a browser at **http://localhost:8080** — which
are separate conversations, so the swarm is talking to two humans who share nothing. It
also opens a control socket.

One Task, run until nothing is left:

```sh
cargo run --bin sandman -- run \
  --role planning \
  --title "Compare two cities" \
  --brief "Compare Bern and Zurich on size and character. Break this into
           research work, wait for the answers, then write the comparison."
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

Roles are `research`, `planning`, `memory` and `task_manager`, a closed set in
`src/roles.rs`. `memory` is the odd one: it does no new work, it searches what the swarm
already did — the lessons metacognition wrote, past Tasks, and the conversations behind
them, by meaning rather than keyword. Because state persists, that reaches back across
every previous run.

## What happens when you type something

1. Your message reaches the **Comms Session** on that Channel. If it can answer, it does.
2. Otherwise it creates a **Task** — Role, Title, and a **Brief** that has to make sense
   to someone who was not there — subscribed to your Channel.
3. The Harness starts a **Worker Session**, which sees the Brief and nothing else. It may
   create Tasks of its own and hold for their answers.
4. The Worker finishes with plain text. A review reads the conversation and writes the
   **Result**. There is no tool to finish with.
5. The Result reaches your Comms Session as mail, and it tells you in its own words.

A Worker can start that last step unprompted with a `planning` Task calling
`message_human`. That is how the swarm tells you something you did not ask for.

## Watch it

The browser shows what is true **now**; `sandman.log` shows what happened **in order**.
Both read the same Event stream.

The browser shows every Task, Session and model call as it changes, requests in flight
included. Click anything to see inside it; the selection is in the URL, so a single call
is linkable. Under the Tasks list is **Memory** — the lessons, ranked by the same
embedding call the `memory` Role makes, so a score you see is one a Worker would see. The
header carries tokens and money as billed, not estimated.

```sh
tail -f sandman.log     # an index, not a transcript: --verbose writes the bodies
sqlite3 sandman.sqlite 'select id, title, state from tasks order by id desc limit 10'
```

`sandman.sqlite` holds every Task with its Result, every Session with its conversation,
and every call with its request and reply. It outlives the run, which is what makes the
`memory` Role useful.

## Test it

```sh
cargo test                           # spends nothing
cargo test -- --ignored              # the bench cases, against a real model
cargo run --bin bench -- --times 5   # with a report and artifacts
```

Each case builds its own Sandman — own database, log and id counters — so they run
together in one process. See [docs/benchmarking.md](./docs/benchmarking.md).

## Configuration

Almost none, on purpose. Model and API key in `src/model.rs`, port in `src/web/mod.rs`.
`OPENROUTER_API_KEY` and `SANDMAN_REASONING_EFFORT` override at run time. The key
committed here is limited and a leak costs nothing.
