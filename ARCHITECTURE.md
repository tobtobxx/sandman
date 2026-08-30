# Architecture

How the parts fit. For what the terms mean, read [CONTEXT.md](./CONTEXT.md) — this file
does not repeat definitions.

One queue, one database, one Event stream, two shapes of live agent context. Only those
two — the Worker Session and the Comms Session — are agentic; the Harness, Store,
Scheduler, Waiters, Role registry, Channel adapters, control socket and web server are
plain code. A Worker needing another's answer does not park and get rebuilt: it holds
inside `await_result` and the answer returns as that call's result, in the same Turn. A
Comms Session cannot — it is never re-run — so it subscribes and gets mail instead.

## Store, database, Events

The Store speaks domain verbs — `start_task`, `complete_task`, `append_message` — not
fields, and returns owned values. Two properties hold structurally: **a change without an
Event cannot be written** (the connection is private, no method mutates without emitting),
and **a lock is never held across an await** (every method takes `&self` and returns; the
`std::sync::Mutex` is deliberate, so an await inside would not compile).

SQLite is the single source of truth — no in-memory mirror, nothing to disagree about.
**A transcript is a query, not a blob**: messages, mail, utterances and reflections get a
row each, keyed `(owner, idx)`, where a JSON column would make a long Comms Session
quadratic. **Sum types are a discriminant column plus JSON**, so `tasks.state` is its own
column and the queue scan is an index lookup. Ids come from `counters` inside the
transaction using them, so a fresh database counts from one and two Harnesses can share
a process; migrations are ordered under `meta.schema_version`, and a newer database is
refused.

Every change emits one Event, and `log.rs`, `web/` and bench cases read that one stream.
It is broadcast, so a slow consumer loses Events rather than slowing the swarm — the state
is still in the database. Tool calls are not state changes, so the tool registry emits
`ToolCalled`/`ToolReturned` on its own handle.

## Task queue, Roles, ways in

The queue is the only route between agents; no direct agent-to-agent call exists. Being
picked has one condition: time — holding for other work happens inside a Turn, after a
Task has started. Completing a repeating Task creates the next occurrence, anchored to the
schedule. Cancelling is terminal: a running Task ends at its Session's next decision point
with no Result, a repeating one stops as a chain, and whoever waited is told.

`RoleName` is the source of truth for Roles; the prompt and tool functions match on it
exhaustively, so a Role added without either does not compile. Every prompt is a Markdown
file compiled in with `include_str!`, and a Worker's system message is the shared mechanics
joined to its Role's file — nothing templated, nothing conditional. The cost is repetition,
paid on purpose: a prompt assembled in the reader's head hides its contradictions.

A Channel adapter converts one transport's traffic into Comms Session input and sends its
output back; adding a transport must not change the Session. The control socket is a Unix
domain socket, one line of JSON in and one out, rather than a second database writer: a
direct insert would bypass the Store and emit no Event, blinding the log and every Watcher.

## Sessions

Both shapes run one Turn loop, and **a Turn decides nothing**: it reports how it ended —
text, silence, an unreachable model, a Task cancelled underneath it — and the caller says
what that means. They differ by almost nothing else, and once drifted apart as two copies
of one loop. Session state lives in the Store, which is why the loop is a function over a
context: it must stay watchable while the loop awaits.

**Worker** — created from a Task, ends when it completes, sees the Brief and nothing of the
work that led to it. It has no tool to submit anything: on plain text the Review reads the
whole conversation, and its `<summary>` is the answer, `<feedback>` buys another Turn,
`<lessons>` is kept. A Review writing neither falls back to the Worker's last words, one of
silence sends it back to work, and only an unreachable model fails a Task without a Review.

**Comms** — one per Channel, standing, never ending. A Turn's text is something to say,
then it goes idle; text carrying `<no-response />` is silence, because models are bad at
empty replies. Silence is legitimate: it owes nobody a Result and is never reviewed, only
interrupted. It owns the human-facing voice, passing content on verbatim or rewording it.

**The scheduler** holds every call: one in flight, the rest waiting by Tier then arrival,
which makes a run possible to follow — the only guard against runaway work. A higher-Tier
call jumps the *waiting* queue but never aborts the one in flight, which is committed and
paid, and a call is recorded when it joins the queue, so waiting is as visible as working.

## Metacognition and Lessons

A **Review** runs when a Worker ends a Turn without calling a tool. An **Interrupt** runs
mid-Turn on a message count and decides nothing about the Task; it exists for the failure
a Review structurally cannot see — a Worker that never stops calling tools is never
reviewed — and it reaches Comms Sessions, which are never reviewed at all. It fires from
the top of the Turn loop, where every tool call already has its result and a pushed message
cannot split the two, and its count runs from the last metacognition of either kind. That
an Interrupt cannot complete a Task is a fact about its signature, not a check. Both are
recorded on the Session judged, for inspection only; only Feedback reaches it, and **both
fail open** — a call that cannot be made is recorded as having found nothing, so broken
metacognition never wedges a run. Neither is an agent, which is load-bearing: as a swarm
member its outcome would be asynchronous, and answers would hang on a pending Review.

A lesson is anchored on the Session judged — the way back to the conversation. Nothing
reads one back automatically; the `memory` Role finds it later, across every Run, by
meaning: text to a vector ranked by cosine, brute force, which stops being right in the
tens of thousands of entries. **Indexing is lazy**, since embedding at creation would put
a network call on the synchronous `create_task` path; the first search embeds what is
uncached in one batch with the query riding along, and a cached vector is never stale
because nothing in the corpus is edited. Embedding calls skip the scheduler, having no
Session and nothing to follow, so they never reach Spend. See [TASKS.md](./TASKS.md).

## Tools

Three create-task tools, so the common case carries no arguments to get wrong:
`create_task` (planning work; `research`, `planning`, `memory`, Comms),
`create_research_task` (`research`), and `create_task_full` with Role, timing and priority
(`planning`, `task_manager`). All return an id, none wait. Every Worker Role has
`await_result`, which holds the Turn until a Task completes or returns a notice if it was
cancelled; `planning` alone has `message_human`. `research` gets `web_search` — which reads
`unresponsive_engines`, so a rate limit is told apart from an empty web — and `web_fetch`,
whose scripts never run. `memory` gets `search_lessons`, `search_tasks` (also
`task_manager`; ranks on Title and Brief, never the Result), `view_session` and
`current_time`; `task_manager` gets `list_tasks` and `cancel_task`, which tells whoever
waited. That is the whole set, and metacognition holds none of it: it answers in
`<summary>`, `<feedback>` and `<lessons>` for the Harness to read. A tool answers in words,
always, including when it failed — an error a model can read is domain output, not a Rust
error. Schemas are per Session, because `message_human` must offer the Channels open now.

## Seams

Four traits, each with a real adapter and a bench adapter — two adapters is what makes a
seam real rather than hypothetical. The scheduler decides *when* a call goes out; `Model`
decides *how*.

| Trait | Real | Bench |
| --- | --- | --- |
| `Model` | OpenRouter over the wire | replies written by the test |
| `ToolRunner` | the tool registry | the registry, watched and answered for |
| `Clock` | the system clock | stopped, or moved by hand |
| `Embedder` | the embedding service | whatever a test wants |

`Model` sits **under** the scheduler, so a scripted bench still exercises the real queue,
Tier ordering and one-call-at-a-time. `ToolRunner` wrapped in a recorder watches every call
without touching a prompt, and answering from a closure drives a model down a path without
paying for the work behind it. `web_search` and `web_fetch` need no HTTP seam: being tools,
they are already interceptable here. Two boundaries hold without a trait and matter as
much: the queue is the only path between agents, so new capability arrives as a Task with
a Role; and the Brief is the boundary between parent and child, so whatever the parent did
not write down is lost — the most likely source of trouble.

## Data flow

Work starts one of three ways: a human on a Channel (the Comms Session answers or issues a
Task), a control socket request, or a one-shot command line run. The Harness does not
round-robin — its loop only starts what is not yet in motion, a Pending Task whose time has
come or a Comms Session with mail — and each Session then runs its own Turn loop, every
call waiting on the scheduler. So two children of one Session run at once, and the one
needing fewer Turns finishes first.

Reaching a human has one route in two directions. **Pull**: the Comms Session issues a Task
subscribed to its Channel, and the answer lands in its Mailbox. **Push**: a Worker creates
a `planning` Task nobody subscribed to, whose Worker calls `message_human`. Push is where
the guessing lives — with several Channels open that Worker chooses which human to tell,
and a Brief carries no record of where the work came from. See [TASKS.md](./TASKS.md).

## Invariants

A Session is `waiting`, `thinking`, `tools`, `idle` (Comms only), `reflecting`, then
`finished` or `failed`, and stays in the database once done. A model call is `queued`,
`in_flight`, then `done` or `failed`, recording tokens and what the provider billed as an
integer of nano-dollars. And these hold everywhere:

- One Task concept: human request, investigation and delegated work are the same thing.
- A Task has exactly one Result, on success or failure — or it is cancelled and has none.
- Only the Review completes a Task; only an unreachable model fails one without a Review;
  only a cancellation stops one mid-run.
- One Comms Session per Channel. A Role is a property of a Task, never a kind of agent.
- Every Task becomes a Worker Session, never a Comms Session, so dispatch is branchless.
- Exactly one model call in flight, across the whole Harness, and every state change emits
  exactly one Event. Nothing but the Store writes to the database.
- Spend is re-summed on every read, never accumulated, so it cannot drift. Nothing else is
  bounded: no cap on any loop, and the human watching is the guard rail.

## File index

```
src/
  domain/         Every definition, no logic: ids (newtypes, minted by the Store), text
                  (Title, Brief, Day), time (Timestamp, Cost, the Clock seam), run, task,
                  session (and metacognition's record of it), call, channel, lesson,
                  message.
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
  session.rs      The Turn. Both shapes run this, and it decides nothing; policy lives in
                  worker.rs (prose triggers a Review) and comms.rs (prose is something to
                  say).
  harness.rs      Task lifecycle, delivery, and the loops that start work.
  channels/       One connection to a human each; control.rs is the socket for the rest.
  web/            The Watcher UI: sockets, and Events as frames.
  bench/          A Sandman under test, with four seams to make unreal. bin/ holds sandman
                  and the bench driver, tests/cases.rs the cases themselves.
```
