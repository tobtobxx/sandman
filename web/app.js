// Watches a running Sandman.
//
// One `init` frame carries every entity the server holds; after that, one
// frame per Event. A `patch` always carries the whole current entity for its
// id — never just the field that changed — so applying one is a plain
// replace, `state[bucket][id] = entity`, never a merge. `appended` is the one
// exception: it names a Session and an index, so a message can be added to a
// running conversation without resending the whole thing.
//
// `/` and `/chat` are the same page; only the body's `chat` class (set below,
// from the path) decides which of the right-hand panels — the inspector or
// the chat window — CSS shows. That is what lets `/chat` be its own link: a
// browser open on just that path never renders the lists at all.

const CHAT = location.pathname === "/chat";
document.body.classList.toggle("chat", CHAT);
for (const a of document.querySelectorAll("#routes a")) {
  a.classList.toggle("here", a.getAttribute("href") === location.pathname);
}

const state = { tasks: {}, sessions: {}, calls: {}, channels: {}, lessons: {} };
let spend = { calls: 0, tokens: 0, cost: 0 };
let bucket = "tasks";
let selected = null; // { bucket, id } | null — what the inspector shows
let socket = null;

/** The Lessons search box's own state: nothing was pushed for it. `hits` is
 *  null while a query is outstanding, and holds `[id, score]` pairs once the
 *  matching `ranked` frame arrives — searches are round trips, and the newest
 *  query wins if two are in flight. */
let find = null; // { query, hits: [[id, score]] | null }

// --- connection --------------------------------------------------------

function connect() {
  socket = new WebSocket(`ws://${location.host}/ws`);
  socket.onopen = () => setLink("live", "up");
  socket.onclose = () => {
    setLink("reconnecting…", "down");
    setTimeout(connect, 1000);
  };
  socket.onmessage = (ev) => onFrame(JSON.parse(ev.data));
}

// The indicator counts the Run's age, not the socket's: it comes off the
// `started_at` the `init` frame carries, so a browser that reconnects — or one
// opened an hour in — reads the same uptime as every other, and a dropped
// socket never looks like a restart. Repainted on a timer because nothing
// events a passing second.
let link = { text: "connecting…", cls: "" };
let runStartedAt = null;

function setLink(text, cls) {
  link = { text, cls };
  paintLink();
}

function paintLink() {
  const el = document.getElementById("link");
  const age = link.cls === "up" && runStartedAt !== null ? uptime(Date.now() - runStartedAt) : null;
  el.textContent = age ? `${link.text} · ${age}` : link.text;
  el.className = `link ${link.cls}`;
}

function onFrame(frame) {
  switch (frame.type) {
    case "init":
      for (const key of Object.keys(state)) state[key] = frame.state[key] ?? {};
      spend = frame.spend;
      runStartedAt = frame.run.started_at;
      paintLink();
      break;
    case "patch":
      state[frame.bucket][frame.id] = frame.entity;
      break;
    case "appended": {
      const session = state.sessions[frame.session];
      if (session) {
        session.messages = session.messages ?? [];
        session.messages[frame.index] = frame.message;
      }
      break;
    }
    case "ranked":
      if (find && find.query === frame.query) find.hits = frame.hits;
      break;
  }
  render();
}

// --- reading the domain shapes on the wire ------------------------------
//
// An enum with no data (TaskPriority, most of SessionStatus, …) arrives as a
// plain string. One with data (TaskState::Running, CallStatus::Done, …)
// arrives as a single-key object, `{ variant: payload }`. Both are read the
// same way here, without knowing the full set of variants up front.

const esc = (s) =>
  String(s ?? "").replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c],
  );

/** The variant name, whichever shape the enum arrived in. */
function tagOf(v) {
  if (typeof v === "string") return v;
  if (v && typeof v === "object") return Object.keys(v)[0];
  return String(v);
}

/** The payload alongside a variant name, or undefined for a plain string one. */
function payloadOf(v) {
  if (v && typeof v === "object") return v[tagOf(v)];
  return undefined;
}

/** The number in an id — every id reads `<prefix>-<n>`, and the number is the
 *  order it was made in. */
const serialOf = (id) => Number(id.slice(id.lastIndexOf("-") + 1));

const money = (nanoUsd) => `$${((nanoUsd ?? 0) / 1e9).toFixed(6)}`;
const when = (ms) => (ms ? new Date(ms).toLocaleTimeString() : "");
/** A span in ms, read at a glance: sub-second stays in ms, longer goes to
 *  seconds, past a minute to minutes and seconds. */
const span = (ms) => {
  if (ms == null) return undefined;
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m${String(Math.round((ms % 60_000) / 1000)).padStart(2, "0")}s`;
};
/** How long something has been going, coarser than `span` — an uptime ticking
 *  in the corner should not redraw a digit every second for hours. */
const uptime = (ms) => {
  const secs = Math.max(0, Math.floor(ms / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, "0")}m`;
};

/** How a state reads at a glance. Every state name a row can show — across
 *  Tasks, Sessions and calls — maps to one of six tones, so the same colour
 *  always means the same thing whichever list is open. Unknown names stay
 *  neutral rather than guessing. */
const TONE = {
  running: "busy", thinking: "busy", tools: "busy", in_flight: "busy",
  pending: "wait", queued: "wait", waiting: "wait", idle: "wait",
  completed: "ok", finished: "ok", done: "ok", succeeded: "ok",
  failed: "bad",
  cancelled: "warn", dropped: "warn",
  reflecting: "think",
};

/** The state badge for a row, coloured by what the state means. */
function stateTag(name, tone = TONE[name] ?? "wait") {
  return `<span class="tag state ${tone}">${esc(name)}</span>`;
}

/** Cost and tokens off a call that finished, or null for one that has not. */
function usageOf(call) {
  if (tagOf(call.status) !== "done") return null;
  return payloadOf(call.status).usage;
}

/** How long a call waited in the queue and how long the model held it, in ms.
 *  Either is null while the call has not got that far: a queued call has
 *  neither, an in-flight one has only the wait. Both come off the timestamps
 *  the status carries — nothing is timed here. */
function spansOf(call) {
  const p = payloadOf(call.status) ?? {};
  return {
    waited: p.sent_at != null ? p.sent_at - call.queued_at : null,
    wall: p.finished_at != null ? p.finished_at - p.sent_at : null,
  };
}

/** The Worker Session running a Task, if any. A Task has at most one. */
function sessionForTask(taskId) {
  return Object.values(state.sessions).find(
    (s) => tagOf(s.kind) === "worker" && payloadOf(s.kind).task === taskId,
  );
}

/** The Task whose Worker created this one, if any — `created_by` names a
 *  Session, and a Worker Session names the Task it is running. A Task made
 *  from the command line or the control socket, or by a Comms Session, is a
 *  root. */
function parentTaskId(t) {
  if (tagOf(t.created_by) !== "session") return null;
  const s = state.sessions[payloadOf(t.created_by)];
  return s && tagOf(s.kind) === "worker" ? payloadOf(s.kind).task : null;
}

/** Every Task, depth-first under whichever Task's Worker created it, newest
 *  sibling first. Flat by default — most Tasks have no children. */
function taskTree() {
  const children = new Map();
  const roots = [];
  for (const t of Object.values(state.tasks)) {
    const p = parentTaskId(t);
    if (p) (children.get(p) ?? children.set(p, []).get(p)).push(t);
    else roots.push(t);
  }
  const byAge = (a, b) => b.created_at - a.created_at;
  for (const list of children.values()) list.sort(byAge);
  roots.sort(byAge);

  const items = [];
  const walk = (t, depth) => {
    items.push({ task: t, depth });
    for (const child of children.get(t.id) ?? []) walk(child, depth + 1);
  };
  for (const r of roots) walk(r, 0);
  return items;
}

function kv(pairs) {
  return `<dl class="kv">${pairs
    .filter(([, v]) => v !== undefined && v !== "")
    .map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`)
    .join("")}</dl>`;
}

// --- rows, one renderer per bucket --------------------------------------
//
// Each returns the row's inner markup only; `renderRows` wraps it with the
// bucket and id a click needs to open the inspector on the right thing.

function canCancelTask(t) {
  const s = tagOf(t.state);
  return s === "pending" || s === "running";
}

function taskRow(t) {
  // A completed Task takes its colour from the Result it carries rather than
  // from the word "completed": a Task that finished having failed is not good
  // news, and the badge is the only place a row says so.
  const name = tagOf(t.state);
  const tone =
    name === "completed" ? TONE[tagOf(payloadOf(t.state).result)] : undefined;
  const cancel = canCancelTask(t)
    ? `<button class="cancel-btn" data-cancel="${esc(t.id)}">Cancel</button>`
    : "";
  return `<span class="id">${esc(t.id)}</span>
    <span class="ttl" title="${esc(t.brief)}">${esc(t.title)}</span>
    <span class="tag">${esc(t.role)}</span>
    <span class="tag pri">${esc(t.priority)}</span>
    ${stateTag(name, tone)}
    ${cancel}`;
}

function sessionRow(s) {
  const kind = tagOf(s.kind);
  const detail =
    kind === "worker"
      ? `${esc(payloadOf(s.kind).role)} · ${esc(payloadOf(s.kind).task)}`
      : `channel ${esc(payloadOf(s.kind).channel)}`;
  const n = s.calls?.length ?? 0;
  return `<span class="id">${esc(s.id)}</span>
    <span class="ttl">${esc(kind)} · ${detail}</span>
    ${stateTag(tagOf(s.status))}
    <span class="cost">${n} call${n === 1 ? "" : "s"}</span>`;
}

function callRow(c) {
  const usage = usageOf(c);
  const { wall } = spansOf(c);
  return `<span class="id">${esc(c.id)}</span>
    <span class="ttl">${esc(c.session)} · tier ${esc(c.tier)} · ${esc(c.model)}</span>
    ${stateTag(tagOf(c.status))}
    <span class="cost">${usage ? money(usage.cost) : "—"}</span>
    <span class="tok">${usage ? `${usage.tokens} tok` : ""}</span>
    <span class="wall">${span(wall) ?? ""}</span>`;
}

function channelRow(c) {
  const last = c.transcript?.[c.transcript.length - 1];
  return `<span class="id">${esc(c.id)}</span>
    <span class="ttl">${esc(c.kind)} · ${c.transcript?.length ?? 0} said</span>
    <span class="snip">${last ? esc(last.text) : ""}</span>`;
}

function lessonRow(l, score) {
  return `<span class="id">${esc(l.id)}</span>
    <span class="ttl">${esc(l.day)} · ${esc(tagOf(l.about))}</span>
    ${score !== undefined ? `<span class="score">${score.toFixed(2)}</span>` : ""}
    <span class="snip">${esc(l.text)}</span>`;
}

const ROW = { sessions: sessionRow, calls: callRow, channels: channelRow };

/** Wrap one row's markup with what a click needs to select it. `depth` nudges
 *  a task tree's children right, a little at a time — enough to read as a
 *  tree, not a staircase, so it stays capped rather than growing with a deep
 *  chain of delegation. */
function row(bucketName, id, inner, depth = 0) {
  const sel = selected?.bucket === bucketName && selected?.id === id ? " sel" : "";
  const indent = depth ? ` style="padding-left: calc(1rem + ${Math.min(depth, 4) * 12}px)"` : "";
  return `<div class="row${sel}" data-bucket="${bucketName}" data-id="${esc(id)}"${indent}>${inner}</div>`;
}

// --- the inspector, one detail renderer per bucket ----------------------

function taskDetail(t) {
  const s = sessionForTask(t.id);
  const result = payloadOf(t.state);
  const cancel = canCancelTask(t)
    ? `<button class="cancel-btn" data-cancel="${esc(t.id)}">Cancel task</button>`
    : "";
  return (
    kv([
      ["task", esc(t.id)],
      ["title", esc(t.title)],
      ["role", esc(t.role)],
      ["priority", esc(t.priority)],
      ["state", esc(tagOf(t.state))],
      ["schedule", esc(tagOf(t.schedule))],
      ["subscriber", t.subscriber ? esc(t.subscriber) : undefined],
      ["created by", esc(tagOf(t.created_by))],
      ["created at", when(t.created_at)],
      ["session", s ? esc(s.id) : undefined],
    ]) +
    (cancel ? `<div class="actions">${cancel}</div>` : "") +
    `<h3>Brief</h3><pre>${esc(t.brief)}</pre>` +
    (tagOf(t.state) === "completed"
      ? `<h3>Result</h3><pre>${esc(payloadOf(result.result))}</pre>`
      : "")
  );
}

function sessionDetail(s) {
  const kind = tagOf(s.kind);
  return (
    kv([
      ["session", esc(s.id)],
      ["kind", esc(kind)],
      ["status", esc(tagOf(s.status))],
      [
        kind === "worker" ? "task" : "channel",
        esc(kind === "worker" ? payloadOf(s.kind).task : payloadOf(s.kind).channel),
      ],
      ["calls", s.calls?.length ?? 0],
      ["started", when(s.started_at)],
    ]) +
    `<h3>Messages (${s.messages?.length ?? 0})</h3>` +
    (s.messages ?? []).map(messageHtml).join("")
  );
}

function messageHtml(m) {
  const role = tagOf(m);
  const p = payloadOf(m);
  let body;
  if (role === "assistant") {
    const bodyTag = tagOf(p.body);
    body =
      bodyTag === "text"
        ? esc(payloadOf(p.body))
        : [
            payloadOf(p.body).preamble ? esc(payloadOf(p.body).preamble) : "",
            ...payloadOf(p.body).calls.map((c) => `→ ${esc(c.name)}(${esc(c.arguments)})`),
          ]
            .filter(Boolean)
            .join("\n");
  } else {
    body = esc(p.content);
  }
  return `<div class="msg ${role}"><span class="who">${role}</span><pre>${body}</pre></div>`;
}

function callDetail(c) {
  const usage = usageOf(c);
  const status = payloadOf(c.status);
  const { waited, wall } = spansOf(c);
  return (
    kv([
      ["call", esc(c.id)],
      ["session", esc(c.session)],
      ["tier", esc(c.tier)],
      ["model", esc(c.model)],
      ["status", esc(tagOf(c.status))],
      ["queued", when(c.queued_at)],
      ["sent", when(status?.sent_at)],
      ["finished", when(status?.finished_at)],
      ["waited", span(waited)],
      ["wall", span(wall)],
      ["tokens", usage?.tokens],
      ["cost", usage ? money(usage.cost) : undefined],
      ["error", tagOf(c.status) === "failed" ? esc(status.error) : undefined],
      ["tools offered", c.request.tools.map((t) => esc(t.name)).join(", ") || "none"],
    ]) +
    (tagOf(c.status) === "done"
      ? `<h3>Reply</h3>${messageHtml({ assistant: { body: status.reply, reasoning: null } })}`
      : "") +
    `<h3>Request (${c.request.messages.length} messages)</h3>` +
    c.request.messages.map(messageHtml).join("")
  );
}

function channelDetail(c) {
  return (
    kv([
      ["channel", esc(c.id)],
      ["kind", esc(c.kind)],
      ["session", esc(c.session)],
    ]) +
    `<h3>Transcript (${c.transcript?.length ?? 0})</h3>` +
    (c.transcript ?? [])
      .map(
        (u) =>
          `<div class="msg ${u.who}"><span class="who">${esc(u.who)}</span><pre>${esc(u.text)}</pre></div>`,
      )
      .join("")
  );
}

function lessonDetail(l) {
  return (
    kv([
      ["lesson", esc(l.id)],
      ["day", esc(l.day)],
      ["about", esc(tagOf(l.about))],
      ["session", esc(l.session)],
    ]) + `<h3>Kept</h3><pre>${esc(l.text)}</pre>`
  );
}

const DETAIL = {
  tasks: taskDetail,
  sessions: sessionDetail,
  calls: callDetail,
  channels: channelDetail,
  lessons: lessonDetail,
};

// --- rendering -----------------------------------------------------------

function renderRows() {
  const box = document.getElementById("rows");
  const findBox = document.getElementById("find");

  if (bucket === "tasks") {
    findBox.classList.add("hidden");
    const items = taskTree();
    box.innerHTML =
      items.map(({ task, depth }) => row("tasks", task.id, taskRow(task), depth)).join("") ||
      `<p class="empty">Nothing here yet.</p>`;
    return;
  }

  if (bucket !== "lessons") {
    findBox.classList.add("hidden");
    // Newest on top. Ids read `<prefix>-<n>`, so it is the number that orders
    // them — sorting the strings would put `call-10` before `call-9`.
    const items = Object.values(state[bucket]).sort(
      (a, b) => serialOf(b.id) - serialOf(a.id),
    );
    box.innerHTML =
      items.map((item) => row(bucket, item.id, ROW[bucket](item))).join("") ||
      `<p class="empty">Nothing here yet.</p>`;
    return;
  }

  findBox.classList.remove("hidden");
  if (!find || !find.hits) {
    const items = Object.values(state.lessons);
    box.innerHTML =
      items.map((l) => row("lessons", l.id, lessonRow(l))).join("") ||
      `<p class="empty">${find ? "Searching…" : "Nothing remembered yet."}</p>`;
    return;
  }
  const ranked = find.hits
    .map(([id, score]) => (state.lessons[id] ? [state.lessons[id], score] : null))
    .filter(Boolean);
  box.innerHTML =
    ranked.map(([l, score]) => row("lessons", l.id, lessonRow(l, score))).join("") ||
    `<p class="empty">No lessons match “${esc(find.query)}”.</p>`;
}

function renderInspector() {
  const title = document.getElementById("insp-title");
  const body = document.getElementById("insp-body");
  const entity = selected && state[selected.bucket]?.[selected.id];
  if (!entity) {
    selected = null;
    title.textContent = "Inspector";
    body.innerHTML = `<p class="empty">Pick anything on the left to see inside it.</p>`;
    return;
  }
  title.textContent = `${selected.bucket} · ${selected.id}`;
  body.innerHTML = DETAIL[selected.bucket](entity);
}

// Whether this window is a Channel is simply whether the swarm has a `web`
// Channel open — `channels.web = false` in config.toml opens none, and then
// there is nothing to type into. Read only once `init` has landed, or an
// unopened socket would look like a Channel that is turned off.
function renderChat() {
  const channel = Object.values(state.channels).find((c) => c.kind === "web");
  const box = document.getElementById("transcript");
  const input = document.getElementById("say-text");

  input.disabled = runStartedAt !== null && !channel;
  input.placeholder = input.disabled
    ? "The web Channel is turned off."
    : "Talk to the swarm…";
  if (input.disabled) {
    box.innerHTML =
      `<p class="empty">The web Channel is turned off. Set ` +
      `<code>channels.web = true</code> in config.toml and start Sandman ` +
      `again to talk here.</p>`;
    return;
  }

  box.innerHTML = (channel?.transcript ?? [])
    .map(
      (u) =>
        `<div class="utt ${u.who}"><span class="who">${
          u.who === "human" ? "you" : "sandman"
        }</span>${esc(u.text)}<span class="at">${when(u.at)}</span></div>`,
    )
    .join("");
  box.scrollTop = box.scrollHeight;
}

function render() {
  document.getElementById("spend").textContent =
    `${spend.calls} call${spend.calls === 1 ? "" : "s"} · ` +
    `${spend.tokens.toLocaleString()} tok · ${money(spend.cost)}`;
  renderRows();
  renderInspector();
  renderChat();
}

// --- interaction -----------------------------------------------------------

document.querySelectorAll(".tab").forEach((b) =>
  b.addEventListener("click", () => {
    bucket = b.dataset.bucket;
    document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("here", t === b));
    render();
  }),
);

document.getElementById("rows").addEventListener("click", (ev) => {
  const cancel = ev.target.closest("[data-cancel]");
  if (cancel) {
    ev.stopPropagation();
    const taskId = cancel.dataset.cancel;
    if (socket?.readyState === WebSocket.OPEN) {
      socket.send(JSON.stringify({ t: "cancel", task_id: taskId }));
    }
    return;
  }
  const r = ev.target.closest(".row[data-id]");
  if (!r) return;
  selected = { bucket: r.dataset.bucket, id: r.dataset.id };
  render();
});

document.getElementById("insp-body").addEventListener("click", (ev) => {
  const cancel = ev.target.closest("[data-cancel]");
  if (!cancel) return;
  const taskId = cancel.dataset.cancel;
  if (socket?.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify({ t: "cancel", task_id: taskId }));
  }
});

document.getElementById("find").addEventListener("submit", (ev) => {
  ev.preventDefault();
  const query = document.getElementById("find-text").value.trim();
  if (query === "") {
    find = null;
  } else if (socket?.readyState === WebSocket.OPEN) {
    find = { query, hits: null };
    socket.send(JSON.stringify({ t: "find", query }));
  }
  render();
});

document.getElementById("say").addEventListener("submit", (ev) => {
  ev.preventDefault();
  const input = document.getElementById("say-text");
  const text = input.value.trim();
  if (!text || socket?.readyState !== WebSocket.OPEN) return;
  socket.send(JSON.stringify({ t: "say", text }));
  input.value = "";
});

setInterval(paintLink, 1000);
connect();
