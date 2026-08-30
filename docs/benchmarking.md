# Benchmarking

What the model reaches for inside Sandman: which tool, with what arguments, in what
order. Real prompts, real model, real scheduler, every tool call intercepted — so a case
is one **Session** against one real **Brief**, and says nothing about what the swarm
would have done with that choice. Integration is a series of unit benches; there is no
whole-swarm case.

Code: `src/bench/` (`rig.rs`, `intercept.rs`, `grader.rs`, `report.rs`). Cases:
`tests/cases.rs`.

## Running

```sh
cargo test                            # everything that spends nothing
cargo test -- --ignored               # the cases, against a real model
cargo test -- --ignored greet         # one of them
cargo run --bin bench                 # report and artifacts; all cases, in parallel
cargo run --bin bench -- --case hello --times 5 --serial
```
`--times N` is for variance. Parallel runs hit the model concurrently; rate limiting
under load shows up as inflated wall time, not as failures.

## Seams

Everything is real by default. A case says in its first lines which reality it gave up.

| Seam | Replace it when |
| --- | --- |
| `ToolsChoice` | always — it is what keeps a case to one Session |
| `ModelChoice` | testing the Harness: the Turn loop, the scheduler, the review |
| `ClockChoice` | a scheduled Task has to actually fire |
| `Embedder` | a ranking has to be the same every time |

The clock is the one to be careful with. A case asserting on the model's *judgement* of
time must use the real one: assert on the Schedule it chose and never wait for it.

## A case

`ToolsChoice::Intercept` wraps the real registry: pass a tool through because its effect
is what is asserted on, answer another from a closure, deny a third because reaching for
it at all is the failure. The schemas the model is offered never change.
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
No child Worker runs and no web search happens. `seed_task` and `seed_lesson` write
through the ordinary Store path, so a seeded Task is a Task in every way. `rig.until`
follows the Event stream — nothing polls.

## Verification

- **Tripwires** — evaluated on every Event, and a violation ends the run at once, so a
  looping Session costs at most a call or two past it. For "this must never happen".
- **Goal checks** — once, at the end. A failure fails the run without stopping it.
- **Graders** — a model call, for what no count can judge. Rare, run only after the
  checks pass, and a reply with no parseable verdict is a FAIL.

If a bad outcome would make the Session keep working it is a tripwire; if it can only be
known at the end, a check; if a machine cannot judge it at all, a grader.

## Artifacts

`cargo run --bin bench` writes `bench/runs/<stamp>/<case>-run<k>/`: `result.json`,
`store.sqlite` and `sandman.log` — see `src/bench/report.rs` for what is in each.
```sh
sqlite3 bench/runs/*/plan-greet-run1/store.sqlite \
  'select idx, role, body_json from messages where session = 2 order by idx'
```

## Adding a case

A `#[tokio::test]` in `tests/cases.rs`, `#[ignore]` if it spends money. There is no
registry to add it to. Decide two things first: **which Session is it about** — a
question that needs two Sessions is two cases — and **what must never happen**, which is
the tripwire that keeps a failing case cheap.

## Caveats

- A grader is a model judgement and can be wrong in both directions; read `raw` in
  `result.json` before trusting a marginal verdict.
- Real waits are real. Shape the case to end before the wait, or use
  `ClockChoice::Manual` and accept that the case is now about the Harness.
