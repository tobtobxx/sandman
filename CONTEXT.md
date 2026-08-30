# Sandman

Sandman is an agent swarm. Agents interface with the outside world, and complete or
investigate work. They coordinate through a shared queue rather than through a
command hierarchy.

The words here are exact, and the code uses them. If you need a concept that is not
in this glossary, that is worth stopping over: either the project has a word for it
already, or you are about to invent language it does not use.

## Work

**Task**:
The single unit of work in the system. There is exactly one Task concept — a request
from a human, an investigation, and a piece of work handed between agents are all
Tasks. Tasks are recursive: working on a Task may produce further Tasks. A Task is
Pending while it waits on the queue, Running while a Session holds it, and Completed
once a Result exists. A Task stopped before a Result exists is Cancelled — the
task_manager Role's doing, always — and nothing further happens to it. A Task may
carry a Schedule, which is either "now", a time before which it must not run, or an
interval on which completing it creates the next occurrence. All of these are
properties of the one Task, not a new kind of work.
_Avoid_: Intent, Assignment, Job, Ticket, Item, Work unit

**Result**:
What a Session produces when it completes its Task, whether the work succeeded or
failed — a failure is a Result saying so, not the absence of one. Chosen by the
metacognitive review from what the Worker wrote, not written out by the Worker
itself. A Result nothing subscribes to is recorded and nothing further happens.
In code this is `TaskResult`, so the domain word is not confused with Rust's own.

**Brief**:
The instructions a Task carries. A Session starts fresh and sees nothing of the work
that led to it, so the Brief must stand alone — it is the only thing the Worker gets.
_Avoid_: Description, Payload, Prompt, Body, Content

**Title**:
A Task's one-line description. It exists so a human can scan the queue; no Session
depends on it. The Brief still has to stand alone.
_Avoid_: Name, Label, Subject, Headline

**Role**:
A system prompt plus a set of tools, carried by a Task, which together determine how
its Session approaches the problem — research, planning, memory, task_manager. A
closed set defined in code; tools are independent things that Roles select from. A
Role is a property of work, never a kind of agent.
_Avoid_: Kind, Type, Mode, Persona, Profile, Skill

**Subscription**:
The link between a Task and the Channel its answer should reach. Only a Comms Session
subscribes, because it cannot hold for an answer: it is handed one as mail instead. A
Worker that wants a child's answer does not subscribe — it holds, in place, with
`await_result`. A Task nobody subscribed to is work nobody is waiting on: its Result
is recorded and nothing further happens.
_Avoid_: Callback, Continuation, Await, Watch, Listener, Blocked, Dependency,
Waiting Task

**Schedule**:
When a Task may run, and whether finishing it arms another. Either now, or not before
a given time, or repeating on an interval anchored to the schedule rather than to
when the last occurrence ended — so a late run does not push the next one back. A
repeating Task is a chain of ordinary Tasks, each with its own Result.
_Avoid_: Timer, Cron, Delay, Recurrence, Trigger

## Agents

**Worker**:
The one uniform kind of agent. Every Worker is the same mechanically; what varies is
the Role of the Task it is given.
_Avoid_: Agent type, Specialist, Executor, Bot

**Session**:
A live agent context managed by the Harness. Comes in two shapes: a Worker Session,
created from a Task and ending when that Task completes; and a Comms Session.
_Avoid_: Run (that is its own term here), Invocation, Instance, Thread, Context

**Comms Session**:
A standing Session bound to a Channel, one per Channel, mechanically driven by the
Harness to interface with a human. It keeps context across messages, and can issue
Tasks. It is not the same thing as the planning Role: a Task never targets a Comms
Session, it targets the planning Role, and the resulting Worker messages the Comms
Session with the message_human tool. It cannot be re-run, so answers it subscribed to
are put in its Mailbox directly.
_Avoid_: Comms agent, Conversation agent, Interface agent

**Mailbox**:
What has arrived for a Comms Session and has not been read yet — from its human, or
from the swarm. Post that lands while the Session is mid-turn waits here until the
next one, so nothing arrives in the middle of its thinking.
_Avoid_: Inbox, Queue, Buffer, Pending

**Turn**:
One round of a Session's work: model calls and tool calls, until the model replies
with plain text. Each live Session runs its own Turn loop concurrently with the rest,
and the scheduler orders the model calls those loops make by Tier — so a piece of work
needing fewer Turns than another lands first even if it started later, and a
higher-Tier call jumps the queue. A Review follows the Turn it judges; an Interrupt
fires part-way through one.
_Avoid_: Step, Iteration, Cycle

**Model call**:
One exchange with the model, belonging to a Session. Exactly one is ever in flight,
and a call exists from the moment it joins the queue, so waiting for the model is as
visible as talking to it. It records what it cost, as billed rather than as
estimated.
_Avoid_: LLM call, Request, Completion, Inference, Turn

**Priority**:
A property of a Task: high, normal or low, defaulting to normal. Set by
create_task_full or from outside. It decides the Tier its Worker's calls wait at, and
nothing else.
_Avoid_: Urgency, Weight, Rank, Importance

**Tier**:
Where a model call waits in the scheduler: 1 comms, 2 a high Task, 3 metacognition,
4 a normal Task, 5 a low Task. Lower runs first; within a tier, arrival order decides.
A higher-Tier call that arrives while a lower one waits skips the waiting queue but
never aborts the call already in flight. A Tier is a property of the caller, not of
the call: comms and metacognition have fixed ones, and a Worker's comes from its
Task's Priority.
_Avoid_: Priority (that is the Task's property), Level, Class, Lane

**Spend**:
What a Run has cost in money, summed from the Model calls that finished. It is always
derived, never accumulated, so it cannot drift from the calls it came from.
_Avoid_: Budget, Usage, Billing, Total

## Outside world

**Channel**:
A two-way connection to a human — the terminal, the web UI, a chat network. More than
one may be open, and each has its own Comms Session, so the swarm may be talking to
several humans who share nothing. One-way sources such as RSS or mail are not
Channels: anything outside issues a Task through the Control socket instead.
_Avoid_: Connection, Transport, Feed, Interface, Endpoint

**Transcript**:
What a human on a Channel has actually seen, and what they said. Narrower than the
Comms Session's own history, which also holds system prompts, tool calls and post from
the swarm that the human was never shown.
_Avoid_: History, Log, Conversation, Messages

**Control socket**:
The way a process that is not a human puts work into a running Sandman: one Task in,
one id back. Cron, a mail watcher and a shell one-liner all arrive here. It is a write
path and stays local.
_Avoid_: API, RPC, Admin interface, Command port

## Runtime

**Harness**:
Sandman itself — the whole of the code we write, within which agents run. It owns
Tasks, Results, Sessions, model calls, Channels and the Lessons; agents never manage
that state themselves.
_Avoid_: Runtime, Engine, Framework, Kernel, Orchestrator

**Store**:
Everything the Harness owns, behind one vocabulary. It is the only thing that touches
the database and the only thing that emits Events, so a change nothing can see cannot
be written.
_Avoid_: Repository, DAO, State, Database (that is what it sits on)

**Event**:
One thing that happened, in order. Every change the Store makes emits one, and
everything that needs to know what happened — the log, a Watcher, a bench case waiting
for something — reads that one stream. State and sequence are one mechanism, not two.
_Avoid_: Message, Signal, Notification, Change, Update

**Run**:
One lifetime of Sandman. Several share a database, so the word has to exist: Spend is
scoped to a Run, and the Lessons and past Tasks deliberately are not — they are
searched across every Run, which is what the memory Role is for.
_Avoid_: Session (that is a live agent context), Instance, Process, Boot

**Watcher**:
Something reading the Harness's state as it changes, without taking part. A Watcher
never decides anything, so a swarm behaves the same whether one is attached or not.
Two exceptions, both deliberate: a message on its own Channel, and a search over the
Lessons — a read that costs an embedding call.
_Avoid_: Observer, Monitor, Dashboard, Inspector, Client

**Metacognition**:
Observation of an agent's own reasoning while it runs, delivered back into that
agent's context. It comes in two kinds, the Review and the Interrupt, and both may
write to the Lessons. Neither is an agent: neither has a Role, an identity or tools of
any kind.
_Avoid_: Monitoring, Supervision, Oversight

**Review**:
The metacognition every Worker's plain-text turn passes through. It writes the Task's
answer as its Summary, corrects the Worker with Feedback instead, or stays quiet. A
Comms Session is never reviewed: it owes nobody an answer.
_Avoid_: Reflection, Critique, Judgement, Check

**Interrupt**:
The metacognition that fires on a message count, between an agent's model calls,
rather than at the end of a turn. It asks whether the run is looping, already done,
chasing something unreachable, or off its goal, and it never writes a Summary — the
Session it judges has not offered an answer. Feedback and silence are both normal
outcomes. Unlike a Review it reaches every Session, Worker and Comms alike.
_Avoid_: Check-in, Heartbeat, Watchdog, Timeout, Nudge

**Summary**:
The answer a review writes for the Task it judged, and the normal way a Task
completes. A review writes a Summary or it writes Feedback, never both: Feedback means
the run is not over, so there is no answer yet.
_Avoid_: Judgement, Decision, Conclusion, Outcome, Submission, Verdict

**Feedback**:
Correction a metacognition writes into a Session's context as a message of its own.
The Session takes its next Turn on it. It is the only thing a metacognition produces
that the Session it judged ever sees.
_Avoid_: Note, Comment, Hint, Advice

**Lessons**:
What metacognition has kept: one lesson per Review or Interrupt that had something
worth keeping — what a Session struggled with, and what whoever does that kind of work
next would want to know. A lesson is anchored on the Session that wrote it, not the
Task: the Session is always the way back to the full conversation, and a lesson is
searched on its own. Most come from a Task; one from a Comms Session has no Task,
because a conversation with a human is not one. It never re-enters the Session it
judged — it is found later, by someone looking for it, which is what the memory Role
is for.
_Avoid_: Memory, Journal, Notes, Diary, Knowledge base, Tips, Takeaways

## The bench

**Rig**:
One Sandman under test: its own database, Event stream, scheduler, log and Harness,
sharing nothing with any other. What makes a bench case a test rather than a process.
_Avoid_: Fixture, Sandbox, Environment, Setup

**Case**:
One question put to the harness-and-model combination, with the verification that
answers it. A case is a test.
_Avoid_: Scenario, Benchmark, Trial, Experiment

**Tripwire**:
A condition evaluated continuously while a case runs: "this must never happen".
Violating one ends the run at once, so a looping swarm costs at most a call or two
past the violation.
_Avoid_: Assertion, Guard, Invariant, Alarm

**Grader**:
Verification a model has to do, for outcomes no read of the state can judge — whether
a Task really is the one that was wanted. It is bench machinery, not part of the
swarm, so what it costs is reported apart from Spend.
_Avoid_: Judge, Evaluator, Scorer, Critic
