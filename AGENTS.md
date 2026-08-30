# Working in this repo

Sandman is a prototype. Prefer the direct change over the general one, and say what you
broke rather than building around it.

Read [CONTEXT.md](./CONTEXT.md) first. The words there are exact and the code uses them.
A concept that is not in the glossary is worth stopping over.

- **Nothing but `store.rs` touches the database.** SQL elsewhere means the Store is
  missing a word.
- **A Turn decides nothing.** Ending policy goes in `worker.rs` or `comms.rs`, never in
  `session.rs`.
- **Make the state unrepresentable, not documented.** A new `Option` on an entity is
  worth a second look; a variant usually does it better.
- **Bodies are `unimplemented!()`.** Fill them bottom-up in [TASKS.md](./TASKS.md) order
  and keep the docstrings honest — they are the intent, and a body that differs is the bug.
- **`cargo test` spends nothing and reaches no network.** Anything talking to a real
  model is `#[ignore]`d. Use `bench::Rig`, not a hand-built Harness, and prefer
  `ScriptedModel` for testing Sandman itself. See [docs/benchmarking.md](./docs/benchmarking.md).
