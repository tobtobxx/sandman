# Glossary

Sandman is an agent swarm coordinating through a shared queue, not a command hierarchy.
The words here are exact and the code uses them. A concept that is not here is worth
stopping over: either the project has a word for it, or you are inventing language.

## Work

- **Task** — The single unit of work; a human request, an investigation and delegated work
  are all Tasks, and working on one may produce more. `Pending` on the queue, `Running`
  while a Session holds it, `Completed` once a Result exists, `Cancelled` if stopped before
  one (task_manager only). May carry a Schedule and a Priority. _Avoid_: Intent, Job, Item.
- **Result** — What a Session produces on completing its Task, success or failure — a
  failure is a Result saying so, not the absence of one. Written by the Review, not the
  Worker. `TaskResult` in code.
- **Brief** — The instructions a Task carries, and the only thing its Worker gets; it
  must stand alone. _Avoid_: Description, Payload, Prompt, Body.
- **Title** — A Task's one line, so a human can scan the queue. Nothing depends on it.
  _Avoid_: Name, Label, Subject.
- **Role** — A system prompt plus a tool set, carried by a Task: research, planning,
  memory, task_manager. Closed set in code, a property of work, never a kind of agent.
  _Avoid_: Kind, Type, Mode, Persona.
- **Subscription** — The link between a Task and the Channel its answer should reach.
  Only a Comms Session subscribes; a Worker holds in `await_result` instead. An
  unsubscribed Result is recorded and nothing more. _Avoid_: Callback, Await, Listener.
- **Schedule** — When a Task may run, and whether it makes work of its own: now, not
  before a time, or a cron expression in local time. `in_seconds` and `cron` are the two
  ways to ask, never together. _Avoid_: Timer, Interval, Delay.
- **Cron Task** — A Task on a cron Schedule. It never runs and stays `Pending`: every
  occurrence it makes a **Daughter** instead — the same Title, Brief, Role, Priority and
  creator, running now. Cancel the cron Task to end the succession, a Daughter to skip
  one occurrence. _Avoid_: Chain, Recurring Task, Parent, Child, Occurrence.
- **Priority** — high, normal or low, default normal. Decides the Tier its Worker's calls
  wait at, and nothing else. _Avoid_: Urgency, Weight, Rank.

## Agents

- **Worker** — The one uniform kind of agent; only the Role of its Task varies.
  _Avoid_: Agent type, Specialist, Executor, Bot.
- **Session** — A live agent context owned by the Harness, in two shapes: Worker and
  Comms. _Avoid_: Run, Invocation, Instance, Thread.
- **Comms Session** — A standing Session bound to one Channel, keeping context across
  messages and able to issue Tasks. Not the planning Role: a Task never targets it, and a
  planning Worker reaches it with `message_human`. It cannot be re-run, so answers land in
  its Mailbox. _Avoid_: Comms agent, Interface agent.
- **Mailbox** — What has arrived unread for a Comms Session, from its human or the swarm.
  Post landing mid-turn waits for the next one. _Avoid_: Inbox, Queue, Buffer.
- **Turn** — One round of a Session's work: model calls and tool calls until the model
  replies with plain text. Every live Session runs its own, concurrently.
  _Avoid_: Step, Iteration, Cycle.
- **Model call** — One exchange with the model, belonging to a Session. It exists from the
  moment it joins the queue, so waiting is as visible as talking, and it records what it
  cost as billed. _Avoid_: LLM call, Request, Completion, Inference.
- **Tier** — Where a model call waits: 1 comms, 2 high Task, 3 metacognition, 4 normal
  Task, 5 low Task. Lower first, then arrival. A property of the caller, not the call.
- **Spend** — What a Run has cost, summed from finished Model calls. Always derived,
  never accumulated. _Avoid_: Budget, Usage, Billing, Total.

## Outside world

- **Channel** — A two-way connection to a human: terminal, web UI, chat network. Several
  may be open, each with its own Comms Session, sharing nothing. One-way sources are not
  Channels; they use the Control socket. _Avoid_: Connection, Transport, Endpoint.
- **Transcript** — What a human on a Channel actually saw and said. Narrower than the
  Comms Session's own history. _Avoid_: History, Log, Conversation, Messages.
- **Control socket** — How a non-human process puts work into a running Sandman: one Task
  in, one id back. Local, write-only. _Avoid_: API, RPC, Admin interface.

## Runtime

- **Harness** — Sandman itself: all the code we write, within which agents run. It owns
  Tasks, Results, Sessions, calls, Channels and Lessons; agents own no state.
  _Avoid_: Runtime, Engine, Kernel, Orchestrator.
- **Store** — Everything the Harness owns, behind one vocabulary. The only thing that
  touches the database and the only thing that emits Events.
  _Avoid_: Repository, DAO, State, Database.
- **Event** — One thing that happened, in order. Every Store change emits one, and the
  log, Watchers and bench cases read that one stream. _Avoid_: Message, Signal, Update.
- **Run** — One lifetime of Sandman. Several share a database: Spend is scoped to a Run,
  Lessons and past Tasks deliberately are not. _Avoid_: Session, Instance, Process, Boot.
- **Watcher** — Something reading state as it changes without taking part; a swarm
  behaves the same whether one is attached or not. _Avoid_: Observer, Monitor, Dashboard.
- **Metacognition** — Observation of an agent's reasoning while it runs, fed back into its
  context. Two kinds, Review and Interrupt; both may write Lessons. Neither is an agent: no
  Role, no identity, no tools. _Avoid_: Monitoring, Supervision, Oversight.
- **Review** — The metacognition every Worker's plain-text Turn passes through. It writes
  a Summary, or Feedback instead, or stays quiet. A Comms Session is never reviewed.
  _Avoid_: Reflection, Critique, Judgement.
- **Interrupt** — The metacognition firing on a message count mid-Turn. It asks whether
  the run is looping, done, chasing something unreachable, or off goal, never writes a
  Summary, and reaches every Session. _Avoid_: Check-in, Heartbeat, Watchdog, Nudge.
- **Summary** — The answer a Review writes, and the normal way a Task completes. Never
  written together with Feedback. _Avoid_: Judgement, Conclusion, Verdict.
- **Feedback** — Correction written into a Session's context as a message of its own, and
  the only thing a metacognition produces that the judged Session sees.
  _Avoid_: Note, Comment, Hint.
- **Lessons** — What metacognition kept: what a Session struggled with and what whoever
  does that work next would want to know. Anchored on the Session that wrote it, and never
  re-entering the Session it judged. _Avoid_: Memory, Journal, Notes.

## The bench

- **Rig** — One Sandman under test: its own database, Event stream, scheduler, log and
  Harness, sharing nothing. _Avoid_: Fixture, Sandbox, Environment.
- **Case** — One question put to the harness-and-model combination, with the verification
  that answers it: one Session, every tool call intercepted, and assertions on what the
  model reached for. A case is a test. _Avoid_: Scenario, Trial, Experiment.
- **Tripwire** — A condition evaluated continuously while a case runs: "this must never
  happen". Violating one ends the run at once. _Avoid_: Assertion, Guard, Alarm.
- **Grader** — Verification a model has to do, for outcomes no read of state can judge.
  Rare, and on a stronger model than the swarm's. Bench machinery, so its cost is reported
  apart from Spend. _Avoid_: Judge, Scorer.
