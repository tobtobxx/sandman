# Architecture

One queue, one database, one Event stream, two shapes of live agent context.

Agentic: the Worker Session and the Comms Session. Plain code: Harness, Store, Scheduler,
Waiters, Role registry, Channel adapters, control socket, web server.

A Worker needing another's answer does not park and get rebuilt. It holds inside
`await_result`; the answer returns as that call's result, same Turn. A Comms Session is
never re-run — it subscribes and gets mail.

## Store, database, Events

SQLite is the single source of truth. No in-memory mirror. Structural, not documented:

- **No change without an Event.** The connection is private; no method mutates silently.
- **No lock across an await.** Every method takes `&self`. The `std::sync::Mutex` is
  deliberate: an await inside would not compile.
- **A transcript is a query, not a blob.** Row per entry, keyed `(owner, idx)`. A JSON
  column would make a long Comms Session quadratic.
- **Sum types are a discriminant column plus JSON.** The queue scan is an index lookup.

Ids from `counters`, inside the transaction using them — so two Harnesses can share a
process. Migrations ordered under `meta.schema_version`; a newer database is refused.

A Run opens by ending the last one's leftovers: `Store::open` cancels every Task marked
running, Session still open and call still queued or out. Nothing resumes a Session.
Pending Tasks are left — they are the queue. Unscoped by Run, and safe because
`db::Lock` gives one Sandman the database file: a pid lockfile beside it, stale locks
cleared by checking `/proc`, `--break-lock` to override.

One Event stream, read by `log.rs`, `web/` and bench cases. Broadcast: a slow consumer
loses Events, never slows the swarm. Tool calls are not state changes, so the registry
emits `ToolCalled`/`ToolReturned` on its own handle.

## Queue, Roles, ways in

- Picked on one condition: time.
- Completing a repeating Task creates the next occurrence, anchored to the schedule.
- Cancel is terminal. The Session stops at its next decision point, no Result; a repeating
  chain ends; waiters are told.
- A Role added without a prompt or without tools does not compile — `RoleName` is matched
  exhaustively.
- Prompts: Markdown, `include_str!`, shared mechanics plus the Role's file. Nothing
  templated, nothing conditional. Repetition is the price, paid on purpose — a prompt
  assembled in the reader's head hides its contradictions.
- Channel adapter: one transport's traffic ↔ Comms Session input. A new transport must not
  change the Session.
- Control socket: one line of JSON each way, never a second database writer. A direct
  insert would bypass the Store and emit no Event.

## Sessions

Three layers: the Harness starts Sessions, `worker.rs` and `comms.rs` hold the policy,
`session.rs` holds the loop. Neither policy file references the other.

```
Harness::run → step                  starts only what is not already in motion
  ├ drive_comms(channel)             drains the mailbox, one respond at a time
  │   └ comms::respond ────┐         mail → tell each → one Turn → idle
  └ drive_worker(session)  │         repeats until Done or Aborted
      └ worker::work_turn ─┤
          │                ▼
          │        session::turn(ctx, tier)      loop, per iteration:
          │          Task cancelled            → Turn::Cancelled
          │          msgs since last metacognition ≥ INTERRUPT_EVERY
          │                                    → reflect::interrupt → tell
          │          scheduler.request           one call, Tier from the caller
          │          Reply::Calls               → tools.run each, then loop
          │          Reply::Text                → Turn::Text / Turn::Silent
          └ reflect::reflect → tell             Worker only
```

`SessionCtx` — Store, Events, Scheduler, ToolRunner, Clock, Harness, each an `Arc` — is
the one handle, passed down all three layers and into every tool. A Session owns nothing;
its state is in the Store, so the loop stays watchable while it awaits.

**A Turn decides nothing.** It reports how it ended and the caller says what that means:

| `Turn` | `worker.rs` | `comms.rs` |
| --- | --- | --- |
| `Text` | Review, which writes the Result | said to the human |
| `Silent` | Review; nothing to say sends it back to work | a legitimate ending |
| `Unreachable` | Task failed — the one Result written without a Review | idle |
| `Cancelled` | Session ends, no Result | unreachable: no Task |

Both shapes get the Interrupt, because it fires inside the loop where no policy sees it —
a Worker grinding on tool calls never returns a Turn at all.

**Worker.** From a Task. Sees the Brief and nothing of the work behind it. No tool to
submit with.

**Comms.** One per Channel, standing, never ends. Owes no Result, never reviewed. Owns the
human-facing voice: verbatim or reworded. `<no-response />` is silence, because models are
bad at empty replies.

**Scheduler.** One call in flight; the rest ordered by Tier, then arrival. A higher Tier
jumps the *waiting* queue, never aborts the call in flight — that one is committed and
paid. Recorded on joining the queue, so waiting is as visible as working.

## Metacognition and Lessons

**Review**: a Worker ends a Turn without calling a tool. It reads the whole conversation
and writes `<summary>`, the Task's answer, or `<feedback>`, which buys a Turn, or
`<lessons>`. Neither of the first two, and the Worker's last words stand.
**Interrupt**: mid-Turn, counted from the last metacognition of either kind.

- An Interrupt decides nothing about the Task. Its signature says so; it is not a check.
- Recorded on the Session judged, for inspection. Only Feedback reaches it.
- **Both fail open.** A call that cannot be made found nothing.
- Neither is an agent. As swarm members their outcome would be asynchronous, and answers
  would hang on a pending Review.

A lesson is anchored on the Session judged. Nothing reads one back automatically; `memory`
finds it later, across every Run, by meaning — cosine, brute force, wrong somewhere in the
tens of thousands. **Indexing is lazy**: embedding at creation would put a network call on
the synchronous `create_task` path. The first search embeds the uncached in one batch. A
cached vector is never stale, because nothing in the corpus is edited. Embedding calls skip
the scheduler and never reach Spend. See [TASKS.md](./TASKS.md).

## Tools

Role to tools is `roles.rs`. Across all of them:

- Three create-task tools, so the common case carries no arguments to get wrong.
- `await_result` is the only one that holds a Turn.
- Metacognition has none. It answers in `<summary>`, `<feedback>` and `<lessons>`.
- A tool answers in words, always, including when it failed. An error a model can read is
  domain output, not a Rust error.
- Schemas are per Session — `message_human` must offer the Channels open now.

## Seams

Four traits, two adapters each. Two is what makes a seam real rather than hypothetical.
The scheduler decides *when* a call goes out; `Model` decides *how*.

| Trait | Real | Bench |
| --- | --- | --- |
| `Model` | OpenRouter over the wire | replies written by the test |
| `ToolRunner` | the tool registry | the registry, watched and answered for |
| `Clock` | the system clock | stopped, or moved by hand |
| `Embedder` | the embedding service | whatever a test wants |

`Model` sits **under** the scheduler, so a scripted bench still exercises the real queue,
Tier ordering and one-call-at-a-time. `ToolRunner` in a recorder watches every call without
touching a prompt; answering from a closure drives a model down a path without paying for
the work behind it. `web_search` and `web_fetch` need no HTTP seam — being tools, they are
already interceptable.

Two boundaries hold without a trait, and matter as much:

- The queue is the only path between agents. New capability arrives as a Task with a Role.
- The Brief is the parent/child boundary. Whatever the parent did not write down is lost —
  the most likely source of trouble.

## What happens when you type something

1. The message reaches the **Comms Session** on that Channel. It answers if it can.
2. Otherwise it creates a **Task**: Role, Title, and a **Brief** that has to make sense to
   someone who was not there. Subscribed to that Channel.
3. The Harness starts a **Worker Session** — the Brief and nothing else. It may create
   Tasks of its own and hold for their answers.
4. The Worker ends in plain text. The Review writes the **Result**.
5. The Result arrives as mail, and the Comms Session says it in its own words.

That is **pull**. **Push**: a Worker creates a `planning` Task nobody subscribed to, whose
Worker calls `message_human` — the swarm saying something unasked. Push is where the
guessing lives: several Channels may be open, and a Brief carries no record of where the
work came from. See [TASKS.md](./TASKS.md).

Three ways in, then: a human on a Channel, a control socket request, a one-shot command
line run. The Harness does not round-robin. Every Session loop runs at once and every call
waits on the scheduler, so two children of one Session run together and the one needing
fewer Turns finishes first.

## Invariants

A Session is `waiting`, `thinking`, `tools`, `idle` (Comms only), `reflecting`, then
`finished`, `failed` or `cancelled`, and stays in the database once done. A model call is
`queued`, `in_flight`, then `done`, `failed` or `dropped`, recording tokens and what the
provider billed as an integer of nano-dollars. And everywhere:

- One Task concept: human request, investigation and delegated work are the same thing.
- A Task has exactly one Result, on success or failure — or it is cancelled and has none.
- Only the Review completes a Task. Only an unreachable model fails one without a Review.
  Only a cancellation stops one mid-run.
- One Comms Session per Channel. A Role is a property of a Task, never a kind of agent.
- Every Task becomes a Worker Session, never a Comms Session, so dispatch is branchless.
- Exactly one model call in flight, across the whole Harness. Every state change emits
  exactly one Event. Nothing but the Store writes to the database.
- Spend is re-summed on every read, never accumulated, so it cannot drift. Nothing else is
  bounded: no cap on any loop, and the human watching is the guard rail.

## File index

```
src/
  domain/         Every definition, no logic: ids (newtypes, minted by the Store), text,
                  time (including the Clock seam), run, task, session, call, channel,
                  lesson, message.
  event.rs        The one ordered trace, which log.rs writes out as sandman.log.
  db/             SQLite: schema, migrations, rows to domain values.
  store.rs        All state, behind one vocabulary. Emits every Event.
  waiters.rs      Who is blocked in await_result on what.
  scheduler.rs    One call in flight, ordered by Tier then arrival.
  model.rs        The Model seam, the OpenRouter adapter, the wire shape.
  memory.rs       The Embedder seam, and ranking by meaning.
  roles.rs        The closed Role set: prompt plus tool names.
  prompts.rs      Every prompt, compiled in from prompts/ (one Markdown file each).
  tools/          The Tool and ToolRunner seams, and every tool.
  reflect.rs      Metacognition: the Review, and the Interrupt.
  session.rs      The Turn loop. Both shapes run it; policy is in worker.rs and comms.rs.
  harness.rs      Task lifecycle, delivery, and the loops that start work.
  channels/       One connection to a human each; control.rs is the socket for the rest.
  web/            The Watcher UI: sockets, and Events as frames.
  bench/          A Sandman under test, with four seams to make unreal, and the cases.
                  bin/ holds sandman and the bench driver; tests/cases.rs wraps each
                  case as a test.
```
