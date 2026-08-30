# Sandman

An agent swarm. Agents talk to a human, and complete or investigate work. They
coordinate through a shared queue, not through a command hierarchy.

## What it is trying to answer

Sandman is built around one bet: **nothing waits on the queue**. There is a single
kind of work — a Task — and a single kind of Worker. A Worker that needs an answer does
not block the queue on it: it holds inside a tool call and carries on where it stopped
when the answer arrives, remembering why it asked.

That buys a system with no suspended contexts to rebuild and no deadlocked queue, and
charges for it in a specific way: because a Worker starts fresh and sees only its
Brief, every piece of context has to be written down by whoever created the Task. This
exists to find out whether that trade is worth making, and where it first hurts.

**Metacognition** comes in two kinds. A *review* reads a Worker's whole conversation
once it stops calling tools, and writes the Task's answer. An *interrupt* fires every
15 messages, mid-work, and asks whether the Session is looping, already done, chasing
something unreachable, or off its goal — it can push a Session on, but it stops
nothing, so a human watching is still the guard rail. One thing stays deliberately
absent: **orchestration is plain code**, not an agent, so nothing in the swarm decides
what runs next.

## Documents

- [CONTEXT.md](./CONTEXT.md) — the vocabulary. Read this first; the code uses these
  words and means them precisely.
- [ARCHITECTURE.md](./ARCHITECTURE.md) — components, data flow, seams and invariants.
- [docs/benchmarking.md](./docs/benchmarking.md) — how the bench works and how to add
  a case.
- [TASKS.md](./TASKS.md) — known debt, and what to suspect when it misbehaves.
- [AGENTS.md](./AGENTS.md) — how to work in this repo.

`typescript-prototype/` is the prototype this design came from. It is kept for
reference and is not built.

## Run it

```sh
cargo run --bin sandman
```

That opens two Channels at once — the terminal you started it in, and a browser at
**http://localhost:8080**. They are separate conversations, so the swarm is talking to
two humans who share nothing. It also opens a control socket, so another process can
put work in while it runs.

Give it one Task and let it run until nothing is left:

```sh
cargo run --bin sandman -- run \
  --role planning \
  --title "Compare two cities" \
  --brief "Compare Bern and Zurich on size and character. Break this into
           research work, wait for the answers, then write the comparison."
```

Put a Task into a Sandman that is already running:

```sh
cargo run --bin sandman -- task \
  --role research \
  --title "Check the weather" \
  --brief "Find tomorrow's forecast for Bern and say what to wear." \
  --at 600
```

That is how anything that is not a Channel — cron, an RSS script, a mail watcher —
gets work in. It prints the Task's id and exits.

Roles are `research`, `planning`, `memory` and `task_manager`. They are a closed set
in `src/roles.rs`.

`memory` is the odd one: it does no new work, it searches what the swarm has already
done. A review or an interrupt may write a lesson — what a Session struggled with,
what whoever does that work next would want to know — and a `memory` Task searches
those lessons, past Tasks, and the conversations behind them, by meaning rather than
by keyword. Because the state persists, that reaches back across every previous run.

## What happens when you type something

Worth reading once, because the indirection is the whole design:

1. Your message reaches the **Comms Session** standing on that Channel. If it can
   answer, it just answers.
2. Otherwise it creates a **Task** — a Role, a one-line Title, and a **Brief** that has
   to make sense to someone who was not there — subscribed to your Channel. Then it
   replies to say it is working on it.
3. The Harness picks the Task up and starts a **Worker Session**. The Worker sees the
   Brief and nothing else. It may create Tasks of its own, and hold for their answers.
4. The Worker finishes by replying with plain text. A review reads the whole
   conversation and writes the **Result**. There is no tool to finish with.
5. The Result reaches your Comms Session as mail, and it tells you — in its own words,
   with the context you need.

A Worker can also start that last step unprompted, by creating a `planning` Task that
calls `message_human`. That is how the swarm tells you something you did not ask for.

## Watch it

Two ways, and they show different things. The browser shows what is true **now**; the
log shows what happened **in order**. Both read the same Event stream.

**The browser** shows the live state of the Harness: every Task, Session and model
call, as it changes — including requests still in flight. Click anything to see inside
it: a Session's whole message history, or the exact request and reasoning behind one
model call. The selection is in the URL, so you can hand someone a link to a single
call.

Under the Tasks list is **Memory** — the lessons metacognition has written. The search
box ranks them by meaning, using the same embedding call the `memory` Role's own
searches make, so a score you see is the score a Worker would see.

The header carries the running total of tokens and money. Cost comes from what the
provider billed for each call, so it is what the run actually spent rather than an
estimate.

**`sandman.log`** is the sequence, which the browser cannot show:

```sh
tail -f sandman.log
```

It is an index, not a transcript: a line names what happened and the id to look it up
under. The bodies live in the database. `--verbose` writes them out in full.

**`sandman.sqlite`** holds everything — every Task with its Result, every Session with
its whole conversation, every model call with its request and reply. It outlives the
run, which is what makes the `memory` Role useful:

```sh
sqlite3 sandman.sqlite 'select id, title, state from tasks order by id desc limit 10'
```

## Test it

Ordinary tests spend nothing:

```sh
cargo test
```

The bench cases talk to a real model, so they are ignored by default:

```sh
cargo test -- --ignored              # all the cases
cargo run --bin bench -- --times 5   # with a report and artifacts
```

Each case builds its own Sandman — its own database, log and id counters — so they run
together in one process without a hack between them. See
[docs/benchmarking.md](./docs/benchmarking.md).

## Configuration

Almost none, on purpose. The model and the API key are in `src/model.rs`, the port in
`src/web/mod.rs`. `OPENROUTER_API_KEY` and `SANDMAN_REASONING_EFFORT` override at run
time. The key committed here is limited and a leak costs nothing.
