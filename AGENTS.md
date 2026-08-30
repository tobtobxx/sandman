# Working in this repo

Sandman is a prototype. Prefer the direct change over the general one, and say what
you broke rather than building around it.

Read [CONTEXT.md](./CONTEXT.md) before touching code. The words there are exact, and
the code uses them. If you need a concept that is not in the glossary, that is worth
stopping over: either the project has a word for it already, or you are about to
invent language it does not use.

## Where things go

[ARCHITECTURE.md](./ARCHITECTURE.md) has the file index and the seams. Two rules
follow from it and are worth repeating here:

- **Nothing but `store.rs` touches the database.** If you find yourself writing SQL
  anywhere else, the Store is missing a word.
- **A Turn decides nothing.** Ending policy belongs in `worker.rs` or `comms.rs`,
  never in `session.rs`. The moment the Turn loop knows about Results or Channels, the
  two shapes of Session start growing apart inside it again.

## Make the state unrepresentable, not documented

Where the prototype carried optional fields and a comment about which combinations
were real, this crate carries a sum type. A Task that is `Completed` has a Result
because there is no way to build one without; a call that is `Done` has a cost for the
same reason.

When you add something, ask what would have to be optional to hold it, and whether a
variant would do instead. A new `Option` on an entity is worth a second look.

## Building it out

Every file has its definitions and its documentation; the bodies are
`unimplemented!()`. Fill them in the order [TASKS.md](./TASKS.md) suggests — the layers
underneath first — and keep the docstrings honest as you go: they describe intent, and
a body that does something else is the bug.

## Testing

`cargo test` must spend nothing and reach no network. Anything that talks to a real
model is `#[ignore]`d and runs under `cargo test -- --ignored`.

Use `bench::Rig` rather than assembling a Harness by hand. It gives you a private
database, a private log, private id counters, and a wind-down that cannot leak a
running swarm. See [docs/benchmarking.md](./docs/benchmarking.md).

Prefer `ScriptedModel` for anything testing Sandman itself. A real model makes such a
test slow, expensive and only mostly repeatable, and it is not what is being measured.

## Git

Commit after every logical unit of work — a new module filled in, a schema change, a
prompt reworded. Do not batch unrelated changes. Say what changed, not which files
were touched.

When an AI tool contributes, add an `Assisted-by: AGENT_NAME:MODEL_VERSION` trailer.
