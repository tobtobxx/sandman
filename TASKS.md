# Tasks

Scratchpad. Debt goes here as it is created, not afterwards.

Known rough edges, not fixed here:
- `MailReceived` produces no Patch, so a Session's mailbox count on the wire goes
  stale between whichever other Events next patch that Session.
- A failed Lessons search (`on_search`) answers with an empty `Ranked` rather than
  surfacing the error — `Frame::Ranked` has no error field to put one in.

## Known debt

- **A Worker cannot report failure as failure.** A Result is written from the review's
  `<summary>`, which always records success. Only the Harness writes a failure, and
  only when the model cannot be reached. A Worker or review that decides a Task is
  impossible submits that statement as a successful Result saying so.

- **A recurring chain has no identity.** A repeating Task is a chain of ordinary
  Tasks, and cancelling one must stop the chain. Identity is by what the re-arm copies
  verbatim — Role, Title, Brief, creator, interval — so two identical
  recurring Tasks would cancel together. A chain-root id on the Task would settle it.

- **A restart ends a recurring chain that was mid-run.** The next occurrence is only
  created when one completes, so a Task cancelled by `Store::recover` never re-arms:
  Ctrl+C at the wrong moment loses a daily Task for good. Deliberate for now — the
  rework that gives a chain its own identity is where this gets fixed.

- **A clean shutdown leaves its Comms Sessions open.** `wind_down` cancels Tasks but
  ends no Session, so after `/quit` the two Comms Sessions sit at `idle` with no
  `ended_at` until the next start cancels them. Harmless and self-correcting; it does
  mean `cancelled` does not distinguish "aborted" from "shut down".

- **A Worker picks a Channel by guessing.** `message_human` names a Channel, and
  several may be open. A Task carries no record of which human it came from — Briefs
  stand alone, by design — so a Worker deciding who to tell has nothing principled to
  go on. It reads the Brief and guesses. Carrying provenance on a Task would fix it and
  would weaken the standalone Brief; neither side of that is obviously right. The pull
  direction does not guess: an answer a Comms Session subscribed to lands in its
  Mailbox with no Worker choosing anything. Push still does.

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

- **Nothing prunes the database.** Every Session, message and model call is kept
  forever. That is what makes the `memory` Role work across runs, and it means the
  file grows without bound. No answer needed yet; the shape when it bites is probably
  to keep Tasks and Lessons and drop the conversations behind old finished Sessions.

- Use sqlite vec instead of manual cosine
- Simplify which channels message_human lists?
- Memory search results cutoff by similarity
- Sessions are cutoff when calling view_session

## To do

- **The Comms Session claims it scheduled a reminder without calling `create_task`.**
  Found by the `greet-again` bench case: asked to "greet me again in about 3 minutes",
  the model replies "I've set a reminder" and never reaches for the tool that would make
  that true. Prompt or mechanics issue in `comms-session.md` / `mechanics.md`, not
  something the bench itself needs to change.

- **A fourth bench case for `message_human`.** Delivery is the path most likely to fail
  and the one nothing covers: does the Session reach for `message_human`, and does it
  name the right Channel? It needs a Rig with two Channels open, to make the "which
  human" guess visible. A unit bench like the rest — the call is intercepted, not
  delivered.
