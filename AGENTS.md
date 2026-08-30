# Working in this repo

Sandman is a prototype. Break backwardscompatibility. Do structural changes
and don't hestate do do major refactorings (after asking the human).

Useful files:
 - Context.md has domain vocabulary.
 - docs/benchmarking.md documents how the bench rig is designed.

- **Nothing but `store.rs` touches the database.** SQL elsewhere means the Store is
  missing a word.
- **A Turn decides nothing.** Ending policy goes in `worker.rs` or `comms.rs`, never in
  `session.rs`.
- **Make invalid states unrepresentable, not documented.** A new `Option` on an entity is
  worth a second look; a variant usually does it better.
- **Bodies are `unimplemented!()`.** Fill them bottom-up in [TASKS.md](./TASKS.md) order
  and keep the docstrings honest — they are the intent, and a body that differs is the bug.

