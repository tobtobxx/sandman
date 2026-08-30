# Benchmarking

The bench measures what the model does inside Sandman: the same prompts, the same
tools, the same scheduler as any other run. It exists to inform prompt and mechanics
changes.

It answers one question: **what did the model reach for?** One Session against one real
Brief, with every tool call intercepted. Cheap, fast, and specific — it says which tool
the model chose, with what arguments, in what order, and it says nothing at all about
what the swarm would have done with that choice.

There is no second, whole-swarm kind of case. Integration is a series of unit benches:
each seam gets the case that covers it, and a failure names the decision that was wrong
instead of a run that ended badly.

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

Everything is real by default: real prompts, real model, real clock. Four seams can be
replaced, and a case should say in its first lines which reality it gave up and why.
The tools are replaced in every case — that is what keeps a case to one Session.

| Seam | Replace it when |
| --- | --- |
| `ToolsChoice` | always: a bench asks what the model reached for, not what happened next |
| `ModelChoice` | you are testing the Harness — the Turn loop, the scheduler, the review — and a real model would only add cost and variance |
| `ClockChoice` | you need a scheduled Task to actually fire. That is a case about the Harness, not the model, and it should say so |
| `Embedder` | you want a ranking that is the same every time |

The one to be careful with is the clock. A case asserting on the model's *judgement*
of time — did it schedule this three minutes out? — must use the real one, or it
benches a Sandman that does not exist. The three cases below assert on the Schedule the
model chose and never wait for it.

## Interception: how a case works

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

They exist because a runaway Session costs money and a wrong answer costs trust, and
they fail differently.

- **Tripwires** — evaluated continuously, on every Event. A violation ends the run at
  once: the Harness stops, remaining Tasks are cancelled, and the last in-flight call
  is given time to land so the cost record stays honest. A looping Session costs at
  most a call or two past the violation. Use for "this must never happen": a second
  `create_task`, a Session asking for the same tool a fifth time. A tripwire is given a
  `Watch` — the Store *and* the calls so far, because a case that answers `create_task`
  itself leaves no row to count.
- **Goal checks** — evaluated once, at the end. A failing check fails the run but does
  not stop it; the work is already done and its evidence is in the artifacts. Use for
  "this must have happened by the end".
- **Graders** — for outcomes no read of the state can judge: does the `create_task`
  call really carry the Brief that was wanted? Keep them rare and be able to say why
  a count would not do — a grader is itself a model judgement, it costs a call, and it
  varies between runs. One model call each, against `GRADER_MODEL`, which is stronger
  than the model the swarm uses: a judge no better than what it judges is not a judge.
  The call is made directly rather than through the scheduler — a grader is bench
  machinery, not part of the swarm, so its cost is reported separately and never counts
  as Spend. Graders run only after every goal check passes. **A reply with no parseable
  verdict is a FAIL**: an unparseable judgement must never quietly pass.

Rule of thumb: if a bad outcome would make the Session keep working, it belongs in a
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

- **`hello`** — `"Hello :)"` gets a reply and reaches for no tool at all. One Comms
  Session, every call denied.
- **`greet_again`** — asking to be greeted again in ~3 minutes reaches for
  `create_task` once; a grader judges whether the Brief is a faithful hand-off. The
  grader passes a Brief that only describes the delay in words: the Comms Session has
  no scheduling tool, so turning words into a timed Task is the next Worker's job.
- **`plan_greet`** — a planning Task seeded from outside reaches for `create_task` once,
  with a Schedule ~3 minutes out, and completes. The creation is answered by the case,
  so no child Worker runs and nothing is left to cancel.

Not covered: delivery — whether a Session reaches for `message_human` and names the
right Channel. That is the natural next case, and [TASKS.md](../TASKS.md) says why it is
the one most likely to fail.

## Adding a case

Write a `#[tokio::test]` in `tests/cases.rs`, mark it `#[ignore]` if it spends money,
and build its Rig. There is no registry to add it to.

Two things worth deciding before writing it:

- **Which Session is it about?** A case covers one. Intercept every tool, and drive
  only as much as it takes to get that Session running. A question that needs two
  Sessions is two cases.
- **What must never happen?** That is a tripwire, and adding it is what keeps a
  failing case cheap.

## Caveats

- A grader is itself a model judgement and can be wrong in both directions, stronger
  model or not; read `raw` in `result.json` before trusting a marginal verdict.
- Real waits are real. A case that has to let three minutes pass on the real clock
  will take three minutes. Shape the case to end before the wait, or use
  `ClockChoice::Manual` and accept that the case is now about the Harness.
