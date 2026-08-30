# Tasks

Scratchpad. Debt goes here as it is created, not afterwards.

## Not built yet

Every source file has its definitions and its documentation; the bodies are
`unimplemented!()`. The docstring is the intent; a body that differs is the bug.
Ten steps, bottom up. `cargo check` passes today and must pass after each step.

### 1. `domain/` — ids, text, time, and the small helpers — done

### 2. `event.rs`, `db/`, `store.rs` — done

### 3. `log.rs` — done

### 4. `model.rs`, `scheduler.rs` — done

Watch out:
- Do not build the queue by holding the `tokio::sync::Mutex` across `send`. Mutex
  wakeups are not tier-ordered. Register `(tier, arrival)`, pick the lowest, notify.
- `queue_call` before the wait, `InFlight` on send, `Done`/`Failed` on return. Waiting
  is as visible as working.
- A higher tier jumps the waiting calls only. The call in flight is paid for.
- `Reply::Empty` is a successful call, not an error.
- Cost comes off the response, never off a price list.
- `From<&Message> for WireMessage` drops reasoning. Keep the wire shape private.

### 5. `roles.rs`, `prompts.rs`, `waiters.rs`, `memory.rs`, `tools/`

Watch out:
- `roles::schemas_for` and `Registry::schemas` must not build two schema sets. One
  calls the other.
- `await_result` registers the wait *before* it reads the Task state. The other order
  hangs a Worker on a Task that completed in between.
- A tool that creates or cancels a Task calls the Harness, not the Store. The Store
  does not deliver, release waiters or re-arm.
- Every tool answers in words, failures included. `ToolError` becomes a string before
  the model sees it.
- `message_human` builds its `channel` enum from the Channels open now.
- `memory::rank` embeds only what has no cached vector, with the query in the same
  batch, keyed on the embedding model. Truncate at `MAX_INPUT_CHARS` rather than
  failing the batch.

### 6. `session.rs`, `reflect.rs`, `worker.rs`, `comms.rs`

Watch out:
- A Turn decides nothing. Any `if` on what the text *meant* belongs in `worker.rs` or
  `comms.rs`.
- The interrupt fires at the top of the loop, where every tool call has its result.
- Both metacognitions fail open. A call that cannot be made is `FailedOpen` and the
  Session carries on.
- Feedback is read before summary; an interrupt's `<summary>` is dropped.
- `section()` stops at the next section tag, not at the end of the reply.
- The judged Session sees only the feedback, as a User message of its own.
- `work_turn` returns `Done`; it must not write the Result itself, or delivery,
  waiters and re-arm are all skipped.

### 7. `harness.rs`

Watch out:
- `step` starts only what is not in motion. Test and insert `driving` /
  `comms_driving` under one lock.
- `run` sleeps on `next_due_in` and wakes on the Event stream. Looping on `step` burns
  a core.
- `complete_task`: record, deliver, release waiters, re-arm — in that order.
- `cancel_task` stops the whole chain, then releases waiters with `render_cancelled`.
  There is no chain id to match on (see Known debt).
- Nothing touches a running Task's Session. It reads the cancelled state itself.
- `run_until_idle` counts a Session blocked in `await_result`, and a Task waiting on
  its own time, as busy.
- `wind_down` cancels first, then waits for the last call, so its cost is recorded.

### 8. `channels/`, `control.rs`, `bin/sandman.rs` — first runnable Sandman

Watch out:
- Only the conversation reaches stdout. The trace goes to `sandman.log`.
- Stdin blocks. Read it on a blocking task.
- A unix socket, owner-only, and a stale file from a killed process is replaced.
- The socket never writes the database. It calls the Harness.
- Wiring lives here alone. Nothing below builds a Model, a Clock or a Registry.
- If `Store::migration()` is `Some((from, to))` after `Store::open`, note it on
  the Logger once — `db::schema::apply` already reports it; nothing before
  this step has a Logger to hand it to.

### 9. `bench/`, `tests/cases.rs`, `bin/bench.rs`

Watch out:
- `Rig::until` checks the predicate before it waits, and survives `Lagged`.
- `Interceptor::schemas` passes through unchanged. Changing them changes what is
  measured.
- `Drop` aborts the drivers. A panicking case must not leave a Harness spending.
- A grader reply with no verdict tag is a FAIL, and grader cost is never Spend.
- Cases keep the real clock unless the case is about the Harness.

### 10. `web/`

Watch out:
- **The Watcher UI has no front end.** `src/web/` serves it and turns Events into
  frames; nothing under `web/` exists to receive them. The prototype's `web/app.js` is
  a reasonable starting shape but reads a different wire format — it merged whole
  entities out of a twice-a-second diff, and there are no diffs now.
- `patch_for` returns `None` for Events a Watcher shows nothing for.
- Nothing in `wire.rs` recomputes. Fields come off the value the Store handed over.
- Two writes only: a message on the browser's Channel, and a Lessons search — ranked
  in the server, with the embedder the `memory` Role uses.

## Known debt

- **A call's three timestamps are one timestamp.** `Scheduler::request` takes a
  single `now` and has no `Clock` of its own, so `queued_at`, `sent_at` and
  `finished_at` all record the instant the caller decided to make the call, not
  when it actually left the queue or came back. Fine for ordering and display;
  wrong for measuring how long a call actually waited or took. Giving the
  Scheduler a `Clock` would fix it and was left out deliberately for now.

- **A Worker cannot report failure as failure.** A Result is written from the review's
  `<summary>`, which always records success. Only the Harness writes a failure, and
  only when the model cannot be reached. A Worker or review that decides a Task is
  impossible submits that statement as a successful Result saying so.

- **A recurring chain has no identity.** A repeating Task is a chain of ordinary
  Tasks, and cancelling one must stop the chain. Identity is by what the re-arm copies
  verbatim — Role, Title, Brief, subscriber, creator, interval — so two identical
  recurring Tasks would cancel together. A chain-root id on the Task would settle it.

- **A Worker picks a Channel by guessing.** `message_human` names a Channel, and
  several may be open. A Task carries no record of which human it came from — Briefs
  stand alone, by design — so a Worker deciding who to tell has nothing principled to
  go on. It reads the Brief and guesses. Carrying provenance on a Task would fix it and
  would weaken the standalone Brief; neither side of that is obviously right. The pull
  direction does not guess: an answer a Comms Session subscribed to lands in its
  Mailbox with no Worker choosing anything. Push still does.

- **A Worker holding for a repeating Task hears only the first answer.** A Worker that
  creates a recurring Task and calls `await_result` is released by occurrence one and
  moves on; later occurrences reach nobody, because a Worker is never a subscriber. A
  Comms Session subscriber hears every occurrence — that is the shape recurring work
  should take.

- **Repetition counts intervals, not wall-clock.** `repeat_seconds: 86400` means every
  24 hours from the anchor, not "every morning at nine". Anchored to the schedule so it
  does not drift, but a true time-of-day schedule needs an absolute one, which nobody
  has needed yet.

- **A Comms Session's context grows without bound.** It is standing, one per Channel,
  and keeps context across every message — it never dies, so nothing trims it. Worker
  Sessions avoid this by being ephemeral. Now that Sessions persist, the context also
  outlives the process it was built in, so this bites sooner than it used to.

- **Embedding calls are invisible to Spend.** `memory.rs` talks to the embedding
  service directly, not through the scheduler, so an embedding is not a Model call and
  what it costs never reaches the run total or the UI. It is small — a batch of a few
  dozen short texts is a fraction of a cent — but the number shown is knowingly
  incomplete rather than exact, which is a worse property than the size suggests.

- **Delivery can land mid-Turn.** Sessions run concurrently, so a child can complete
  while a parent is mid-Turn. The answer resolves a Worker's `await_result` inside the
  tool call it holds on, or lands in a Comms Session's Mailbox and is read on the next
  respond. The interrupt already feeds back the same way.

- **The Role catalogue is written out more than once.** It appears in `planning.md`
  (the one Role prompt that chooses a Role), in `role_catalogue()` (an error message,
  not a prompt), and loosely in `comms-session.md`. The hard fact is the `RoleName`
  enum, and the compiler now catches a Role added without a prompt or a tool set — but
  rewording what a Role is *for* still means editing several places, and nothing
  catches a copy left behind. Taken deliberately: this prompt set has twice shipped a
  self-contradiction, and both times it hid in text no single place held whole.

- **Nothing prunes the database.** Every Session, message and model call is kept
  forever. That is what makes the `memory` Role work across runs, and it means the
  file grows without bound. No answer needed yet; the shape when it bites is probably
  to keep Tasks and Lessons and drop the conversations behind old finished Sessions.

## To do

- **A fourth bench case for `message_human`.** Delivery is the path most likely to fail
  and the one nothing covers: does the Session reach for `message_human`, and does it
  name the right Channel? It needs a Rig with two Channels open, to make the "which
  human" guess visible. A unit bench like the rest — the call is intercepted, not
  delivered.
