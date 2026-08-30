# Benchmarking

The bench measures what the model does inside Sandman: the same prompts, the same
tools, the same scheduler as any other run. It exists to inform prompt and mechanics
changes.

It answers two different questions, and the difference decides how a case is built.

**What did the swarm produce?** Let the whole thing run, and check the state it
reached. Expensive, slow, and the only way to know whether the parts fit together.

**What did the model reach for?** Run one Session against one real Brief and intercept
every tool call. Cheap, fast, and specific — it says which tool the model chose, with
what arguments, in what order, and it says nothing at all about what the swarm would
have done with that choice.

Most cases should be the second kind.

## Running

```sh
cargo test                            # everything that spends nothing
cargo test -- --ignored               # the cases, against a real model
cargo test -- --ignored greet         # one of them
cargo run --bin bench                 # with a report and artifacts
```

| Bench driver flag | Meaning |
| --- | --- |
| (none) | All cases, once, in parallel |
| `--case <name[,name]>` | Only the named case(s) |
| `--times N` | Run each selected case N times — for variance |
| `--serial` | One at a time |

Parallel runs hit the model concurrently. Rate limiting under load shows up as
inflated wall time, not failures — keep that in mind reading variance across `--times`.

## Isolation

A case is an ordinary `#[tokio::test]`. It needs no process of its own and it does not
move the working directory.

A `Rig` owns a whole Sandman: a private in-memory database, its own Event stream, its
own scheduler, its own log in a temporary directory that is removed with it. Ids come
from that database, so they start at one and mean nothing outside it. Nothing in a Rig
is process-global, so two cases running at once share nothing to interfere over.

A Rig also cleans up after itself. `wind_down` cancels everything unfinished and waits
for the last in-flight call so its cost still reaches the record; `Drop` aborts the
driver tasks as a backstop, so a case that panics cannot leave a Harness spending
behind it.

## What a case chooses to make unreal

Everything is real by default: real prompts, real model, real tools, real clock. Four
seams can be replaced one at a time, and a case should say in its first lines which
reality it gave up and why.

| Seam | Replace it when |
| --- | --- |
| `ToolsChoice` | you want to know what the model reached for, not what happened next |
| `ModelChoice` | you are testing the Harness — the Turn loop, the scheduler, the review — and a real model would only add cost and variance |
| `ClockChoice` | you need a scheduled Task to actually fire. That is a case about the Harness, not the model, and it should say so |
| `Embedder` | you want a ranking that is the same every time |

The one to be careful with is the clock. A case asserting on the model's *judgement*
of time — did it schedule this three minutes out? — must use the real one, or it
benches a Sandman that does not exist. The three cases below assert on the Schedule the
model chose and never wait for it.

## Interception: the unit bench

`ToolsChoice::Intercept` wraps the real registry. Every call is recorded — which
Session, which tool, what arguments, what came back — and the case decides which of
them actually happen: pass a tool through because its effect is what is being asserted
on, answer another from a closure because its result is a fixture, deny a third
because reaching for it at all is the failure.

The schemas the model is offered never change. It sees exactly what it would see in a
real run, whatever the case intends to do about the calls — changing them would change
the thing being measured.

```rust
let mut rig = Rig::builder()
    .drive(Drive::Full)
    .tools(ToolsChoice::Intercept(Box::new(|call| match call.name {
        ToolName::CreateTaskFull => Answer::Say("Created Task t-99.".into()),
        _ => Answer::Deny("not available in this case".into()),
    })))
    .build().await?;

let seed = rig.seed_task(planning("Greet the human in 3 minutes", "..."))?;
rig.until("the planner completes", |s| s.is_completed(seed)).await?;

assert_eq!(rig.interceptor.calls_to(ToolName::CreateTaskFull).len(), 1);
```

No child Worker runs, no web search happens, and the whole case is one Session's
decisions.

## Filling the state

`seed_task`, `seed_lesson` and `seed_session` write through the ordinary Store path.
Nothing reaches around the Harness to plant a row, so a seeded Task is a Task in every
way — it emits its Event, it takes an id from the same counter, and the swarm cannot
tell it from one a human asked for.

That is what lets a case test the `memory` Role without first making a swarm earn a
lesson, or test the queue without waiting for one to fill.

## Waiting

`rig.until(what, pred)` follows the Event stream. The predicate is re-checked when
something actually changed, and every tripwire is evaluated on the way past. Nothing
polls, and nothing has to throw across a polling loop to stop a run.

A tripped wire comes back as `Err(Trip)`, which a case propagates with `?`. It is a
value, not a panic and not a process exit.

For a Comms Session, `rig.comms_idle(since_calls)` is true once the mail is read, the
Session is idle, and at least one model call has been made since. Capture the call
count *before* sending, or it is true before the run has done anything.

## The three kinds of verification

They exist because a runaway swarm costs money and a wrong answer costs trust, and
they fail differently.

- **Tripwires** — evaluated continuously, on every Event. A violation ends the run at
  once: the Harness stops, remaining Tasks are cancelled, and the last in-flight call
  is given time to land so the cost record stays honest. A looping swarm costs at most
  a call or two past the violation. Use for "this must never happen": a second Task
  spawning, a Task creating itself again.
- **Goal checks** — evaluated once, at the end. A failing check fails the run but does
  not stop it; the work is already done and its evidence is in the artifacts. Use for
  "this must have happened by the end".
- **Graders** — for outcomes no read of the state can judge: is the spawned Task
  *really* the one that was wanted? One model call each, against the same model the
  swarm uses, made directly rather than through the scheduler — a grader is bench
  machinery, not part of the swarm, so its cost is reported separately and never
  counts as Spend. Graders run only after every goal check passes. **A reply with no
  parseable verdict is a FAIL**: an unparseable judgement must never quietly pass.

Rule of thumb: if a bad outcome would make the swarm keep working, it belongs in a
tripwire; if it can only be known at the end, a check; if a machine cannot judge it at
all, a grader.

## Artifacts

`cargo run --bin bench` writes `bench/runs/<stamp>/<case>-run<k>/`:

- `result.json` — pass or fail, every check with what it saw, the trip that ended the
  run if one did, wall time, Spend, and the graders with their cost kept apart.
- `store.sqlite` — the run's whole database: every Task with its Result, every Session
  with its full transcript and its metacognition, every model call with its request
  and reply. This is what you open when a run fails and you want to know why.
  `sqlite3` reads it.
- `sandman.log` — the order, which the database cannot show.

The directory is named by the driver, never assumed to be the working one, which is
what lets several cases write artifacts from one process.

```sh
sqlite3 bench/runs/*/plan-greet-run1/store.sqlite \
  'select idx, role, body_json from messages where session = 2 order by idx'
```

## The current cases

- **`hello`** — `"Hello :)"` gets a reply and creates no Tasks. Comms-only.
- **`greet_again`** — asking to be greeted again in ~3 minutes spins off exactly one
  Task; a grader judges whether it is a faithful hand-off. The Task is never executed.
  The grader passes a Task that only describes the delay in words: the Comms Session
  has no scheduling tool, so turning words into a timed Task is the next Worker's job.
- **`plan_greet`** — a planning Task seeded from outside spins off exactly one Task
  scheduled ~3 minutes out, and completes. The whole swarm runs. The scheduled Task is
  cancelled unexecuted once the planner is done: a case that waits for work it no
  longer cares about wastes money.
- **`planner_schedules_the_greeting`** — the same question as `plan_greet`, asked as a
  unit bench: one Session, every tool call intercepted, assertions on what the model
  reached for.

Not covered: end-to-end delivery — whether a greeting actually reaches the human,
through `message_human`. That is the natural next case, and [TASKS.md](../TASKS.md)
says why it is the one most likely to fail.

## Adding a case

Write a `#[tokio::test]` in `tests/cases.rs`, mark it `#[ignore]` if it spends money,
and build its Rig. There is no registry to add it to.

Two things worth deciding before writing it:

- **Which question is it?** If it is about what the model reached for, intercept the
  tools and keep the case to one Session. If it is about what the swarm produced, run
  `Drive::Full` and shape the case to end as soon as its question is answered.
- **What must never happen?** That is a tripwire, and adding it is what keeps a
  failing case cheap.

## Caveats

- A grader uses the same model as the swarm. It is itself a model judgement and can be
  wrong in both directions; read `raw` in `result.json` before trusting a marginal
  verdict.
- Real waits are real. A case that has to let three minutes pass on the real clock
  will take three minutes. Shape the case to end before the wait, or use
  `ClockChoice::Manual` and accept that the case is now about the Harness.
