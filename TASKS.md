# Tasks

Scratchpad. Debt goes here as it is created, not afterwards.

## Not built yet

Every source file has its definitions and its documentation; the bodies are
`unimplemented!()`. Order worth building in: `db` and `store`, then `event` and `log`,
then `scheduler` and `model`, then `session`/`worker`/`comms`/`reflect`, then `tools`,
then `harness`, then `control`, and `web` and `bench` last.

- **The Watcher UI has no front end.** `src/web/` serves it and turns Events into
  frames, but nothing under `web/` exists to receive them. The wire format it must
  read is `src/web/wire.rs`: one `init` frame carrying everything, then a `patch` per
  Event. The prototype's `web/app.js` is a reasonable starting shape but reads a
  different wire format — it merged whole entities out of a twice-a-second diff, and
  there are no diffs now.

## Known debt

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

- **Give the interrupt a way to know it is repeating itself.** It fires on a message
  count and reads the whole conversation each time, but it cannot see what the previous
  interrupt said, so a Session in a loop gets the same nudge worded three ways. The
  reflections are on the Session and could be shown to it.

- **A fourth bench case for `message_human`.** Delivery is the path most likely to fail
  and the one nothing covers: does the Session reach for `message_human`, and does it
  name the right Channel? It needs a Rig with two Channels open, to make the "which
  human" guess visible. A unit bench like the rest — the call is intercepted, not
  delivered.
