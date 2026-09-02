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

