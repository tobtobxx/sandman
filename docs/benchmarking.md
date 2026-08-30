# Benchmarking

What the model reaches for inside Sandman: which tool, with what arguments, in what
order. Real prompts, real model, real scheduler, every tool call intercepted — so a case
is one **Session** against one real **Brief**, and says nothing about what the swarm
would have done with that choice. Integration is a series of unit benches; there is no
whole-swarm case.

Code: `src/bench/` (`rig.rs`, `intercept.rs`, `grader.rs`, `report.rs`). Cases: one file
per case under `src/bench/cases/`, each opening with the scenario in plain language.
Cases live in the library, because `bin/bench` cannot reach into a test crate; the
`#[tokio::test]` wrapper for each lives in a `#[cfg(test)]` module at the bottom of its
own case file rather than in `tests/`, so the case and the test that runs it never drift
apart.

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

A case that seeds a Task and waits for it reads almost like the scenario itself:
```rust
let mut rig = Rig::builder()
    .drive(Drive::Full)
    .tools(ToolsChoice::Intercept(Box::new(|call| match call.name {
        ToolName::CreateTaskFull => Answer::Say("Created t-99.".into()),
        _ => Answer::Deny("not available in this case".into()),
    })))
    .build().await?;

let seed = rig.seed_task(brief())?;
rig.await_task(seed).await?;

assert_eq!(rig.interceptor.calls_to(ToolName::CreateTaskFull).len(), 1);
```
A conversational case is `rig.converse(text)` instead of `seed_task` +
`await_task` — it opens a Channel, sends, and waits for the Comms Session to
finish replying, in one call. Both are built on `rig.until`, which follows the
Event stream — nothing polls — and stay available directly for a case whose
wait is neither of those two shapes.

`seed_task` and `seed_lesson` write through the ordinary Store path, so a seeded
Task is a Task in every way; the creation above is answered by the case, so no
child Worker runs.

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

A new file under `src/bench/cases/` — its module doc comment says the scenario in plain
language — and a line in `CASES` in `cases/mod.rs`. Decide two things first: **which
Session is it about** — a question that needs two Sessions is two cases — and **what must
never happen**, which is the tripwire that keeps a failing case cheap.

`cases/mod.rs` carries the ceremony every case shares, so a case's `run` is just: build,
drive, check.
- Build with `Rig::builder()...build().await`; on `Err`, `return (None,
  super::build_failed(case, &trip))`.
- `super::at_most_creations(n)` is a ready-made tripwire for "no more than n Tasks".
- End with `super::finish(case, rig, outcome, graders).await`, which winds the Rig down,
  assembles the `RunReport`, and returns the `(Option<Rig>, RunReport)` a case must.
- `super::bench_test!(case_name);` at the bottom is the whole `#[ignore]`d test wrapper —
  one line instead of writing it out.

## Caveats

- A grader is a model judgement and can be wrong in both directions; read `raw` in
  `result.json` before trusting a marginal verdict.
- Real waits are real. Shape the case to end before the wait, or use
  `ClockChoice::Manual` and accept that the case is now about the Harness.
