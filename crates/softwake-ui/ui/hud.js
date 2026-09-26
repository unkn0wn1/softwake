const capsule = document.querySelector("#capsule");
const canvas = document.querySelector("#bloom");
const ctx = canvas.getContext("2d");
const strip = document.querySelector("#strip");
const logEl = document.querySelector("#log");
const liveEl = document.querySelector("#live");
const form = document.querySelector("#ask-form");
const input = document.querySelector("#ask-input");
const talkBtn = document.querySelector("#talk");
const pendingCard = document.querySelector("#hud-pending");
const pendingNameEl = document.querySelector("#hud-pending-name");
const pendingDescEl = document.querySelector("#hud-pending-desc");
const pendingArgsEl = document.querySelector("#hud-pending-args");
const approveBtn = document.querySelector("#hud-approve");
const denyBtn = document.querySelector("#hud-deny");
const allowBar = document.querySelector("#hud-allow");
const allowText = document.querySelector("#hud-allow-text");
const allowYes = document.querySelector("#hud-allow-yes");
const allowNo = document.querySelector("#hud-allow-no");

const IDLE_MIN_MS = 1000;
const IDLE_MAX_MS = 30000;
const IDLE_DEFAULT_MS = 3000;
const MAX_TURNS = 40;

const particles = [];
let level = 0.02;
let state = "sleep";
let captureRunning = false;
let expanded = false;
let idleTimer = null;
let configuredIdleMs = IDLE_DEFAULT_MS;
let lastActivity = Date.now();
let pointerOver = false;
let raf = 0;
let holding = false;
let talkPending = false;
let autoListening = false;
let lastReplyKey = "";
let refreshInFlight = false;
let lastStatusMessage = "";
let profileName = "Softwake";
let profilePollAt = 0;
let pendingToolId = null;
let pendingBusy = false;
let allowOfferName = "";
let viewW = 120;
let viewH = 120;
const turns = [];

function invoke(command, args) {
  const core = window.__TAURI__ && window.__TAURI__.core;
  if (!core) {
    return Promise.reject(new Error("window bridge is not available"));
  }
  return core.invoke(command, args);
}

function clampIdleMs(ms) {
  const n = Math.round(Number(ms));
  if (!Number.isFinite(n)) {
    return IDLE_DEFAULT_MS;
  }
  return Math.min(IDLE_MAX_MS, Math.max(IDLE_MIN_MS, n));
}

function palette() {
  if (state === "awake") {
    return [
      [255, 170, 90],
      [255, 120, 100],
      [240, 200, 110],
      [255, 140, 160],
    ];
  }
  if (state === "hibernate") {
    return [
      [120, 130, 150],
      [100, 110, 130],
      [140, 145, 160],
    ];
  }
  return [
    [90, 210, 230],
    [120, 160, 255],
    [160, 130, 255],
    [80, 190, 200],
  ];
}

function fitCanvas() {
  const rect = canvas.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  viewW = Math.max(1, Math.round(rect.width) || 120);
  viewH = Math.max(1, Math.round(rect.height) || 120);
  const backingW = Math.max(1, Math.round(viewW * dpr));
  const backingH = Math.max(1, Math.round(viewH * dpr));
  if (canvas.width !== backingW || canvas.height !== backingH) {
    canvas.width = backingW;
    canvas.height = backingH;
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}

function spawnBurstWithLevel(drawLevel) {
  const dens = Math.floor(2 + drawLevel * 14);
  const colors = palette();
  // Square collapsed bloom and the expanded strip both center the particles.
  const cx = viewW * 0.5;
  const cy = viewH * 0.5;
  for (let i = 0; i < dens; i += 1) {
    const color = colors[Math.floor(Math.random() * colors.length)];
    const angle = Math.random() * Math.PI * 2;
    const speed = 0.2 + Math.random() * (0.6 + drawLevel * 1.8);
    particles.push({
      x: cx + (Math.random() - 0.5) * 18,
      y: cy + (Math.random() - 0.5) * 12,
      vx: Math.cos(angle) * speed,
      vy: Math.sin(angle) * speed - 0.15,
      r: 3 + Math.random() * (4 + drawLevel * 10),
      life: 1,
      decay: 0.008 + Math.random() * 0.012,
      color,
      alpha: 0.25 + drawLevel * 0.55,
    });
  }
}

/** Local breath while STT/ask/TTS holds the daemon — never await status here. */
function bloomLevel() {
  const thinking =
    talkPending ||
    lastStatusMessage === "thinking…" ||
    lastStatusMessage === "thinking...";
  if (thinking) {
    const wave = Math.sin(performance.now() / 420);
    return Math.min(0.92, Math.max(0.22, 0.42 + 0.28 * wave, level));
  }
  return level;
}

function tick() {
  fitCanvas();
  const drawLevel = bloomLevel();
  ctx.clearRect(0, 0, viewW, viewH);
  const listening = captureRunning || drawLevel > 0.08 || talkPending;
  if (listening && Math.random() < 0.08 + drawLevel * 0.35) {
    spawnBurstWithLevel(drawLevel);
  } else if (!listening && Math.random() < 0.02) {
    spawnBurstWithLevel(drawLevel);
  }

  for (let i = particles.length - 1; i >= 0; i -= 1) {
    const p = particles[i];
    p.x += p.vx;
    p.y += p.vy;
    p.vy -= 0.008;
    p.life -= p.decay;
    p.r *= 0.992;
    if (p.life <= 0 || p.r < 0.4) {
      particles.splice(i, 1);
      continue;
    }
    const [r, g, b] = p.color;
    const a = p.alpha * p.life;
    const gdv = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, p.r);
    gdv.addColorStop(0, `rgba(${r},${g},${b},${a})`);
    gdv.addColorStop(0.55, `rgba(${r},${g},${b},${a * 0.35})`);
    gdv.addColorStop(1, `rgba(${r},${g},${b},0)`);
    ctx.fillStyle = gdv;
    ctx.beginPath();
    ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
    ctx.fill();
  }
  raf = requestAnimationFrame(tick);
}

async function applyWindowLayout(next) {
  try {
    await invoke("hud_set_layout", { expanded: next });
  } catch (_error) {
    // Layout still updates in-page if the window bridge rejects.
  }
}

function idleBlocked() {
  if (pointerOver || holding || talkPending || pendingToolId) {
    return true;
  }
  return !!input.value.trim();
}

function armIdle() {
  if (idleTimer) {
    window.clearTimeout(idleTimer);
    idleTimer = null;
  }
  if (!expanded) {
    return;
  }
  idleTimer = window.setTimeout(() => {
    idleTimer = null;
    if (!expanded) {
      return;
    }
    if (idleBlocked() || Date.now() - lastActivity < configuredIdleMs) {
      armIdle();
      return;
    }
    setExpanded(false);
  }, configuredIdleMs);
}

function markActivity() {
  lastActivity = Date.now();
  if (expanded) {
    armIdle();
  }
}

function setConfiguredIdle(ms) {
  const next = clampIdleMs(ms);
  capsule.dataset.idleMs = String(next);
  if (next === configuredIdleMs) {
    return;
  }
  configuredIdleMs = next;
  if (expanded) {
    armIdle();
  }
}

function setExpanded(next) {
  if (!next && pendingToolId) {
    next = true;
  }
  if (expanded === next) {
    if (next) {
      markActivity();
    }
    return;
  }
  expanded = next;
  capsule.classList.toggle("expanded", next);
  capsule.setAttribute("aria-expanded", next ? "true" : "false");
  capsule.setAttribute("role", next ? "group" : "button");
  void applyWindowLayout(next);
  if (next) {
    strip.hidden = false;
    input.focus();
    markActivity();
  } else {
    strip.hidden = true;
    liveEl.hidden = true;
    input.blur();
    if (idleTimer) {
      window.clearTimeout(idleTimer);
      idleTimer = null;
    }
  }
  requestAnimationFrame(fitCanvas);
}

function updateHint() {
  const hint = document.querySelector("#hint");
  if (!hint) {
    return;
  }
  hint.hidden = true;
  if (state === "awake") {
    hint.textContent = autoListening
      ? "speak or hold · drag to move"
      : "hold to talk · drag to move";
  } else if (state === "hibernate") {
    hint.textContent = "wake from Settings";
  } else {
    hint.textContent = "click to type · hold to talk";
  }
}

function setLive(text, isError) {
  const line = text || "";
  liveEl.classList.toggle("error", !!isError);
  liveEl.textContent = line;
  liveEl.hidden = !line;
}

function formatClock(ts) {
  return new Date(ts).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function appendBubble(turn) {
  const article = document.createElement("article");
  article.className = "bubble " + (turn.role === "user" ? "user" : "assistant");
  if (turn.error) {
    article.classList.add("error");
  }
  const header = document.createElement("header");
  const who = document.createElement("span");
  who.className = "who";
  who.textContent = turn.name;
  const time = document.createElement("time");
  time.dateTime = new Date(turn.ts).toISOString();
  time.textContent = formatClock(turn.ts);
  header.append(who, time);
  const body = document.createElement("p");
  body.className = "body";
  body.textContent = turn.text;
  article.append(header, body);
  if (turn.note) {
    const note = document.createElement("p");
    note.className = "note";
    note.textContent = turn.note;
    article.append(note);
  }
  logEl.append(article);
  logEl.scrollTop = logEl.scrollHeight;
}

function renderLog() {
  logEl.replaceChildren();
  for (const turn of turns) {
    appendBubble(turn);
  }
}

function pushTurn(turn) {
  turns.push(turn);
  const overflow = turns.length > MAX_TURNS;
  while (turns.length > MAX_TURNS) {
    turns.shift();
  }
  if (overflow) {
    renderLog();
    return;
  }
  appendBubble(turn);
}

function dropTrailingError() {
  const last = turns[turns.length - 1];
  if (!last || !last.error) {
    return;
  }
  turns.pop();
  const node = logEl.lastElementChild;
  if (node) {
    node.remove();
  }
}

function pushUser(text) {
  const clean = String(text || "").trim();
  if (!clean) {
    return;
  }
  pushTurn({ role: "user", name: "You", text: clean, ts: Date.now(), error: false, note: "" });
}

function replyNote(detail, text) {
  const note = String(detail || "").trim();
  if (!note || note === text) {
    return "";
  }
  if (note === "press and hold to talk" || note === "press to talk") {
    return "";
  }
  return note;
}

function pushAssistant(text, detail, isError) {
  const clean = String(text || "").trim();
  if (!clean) {
    return;
  }
  if (!isError) {
    dropTrailingError();
  }
  const last = turns[turns.length - 1];
  // Command result and the next status poll can deliver the same reply twice.
  // A later turn that happens to repeat the words still gets its own bubble.
  if (
    last &&
    last.role === "assistant" &&
    last.text === clean &&
    !!last.error === !!isError &&
    Date.now() - last.ts < 2000
  ) {
    return;
  }
  pushTurn({
    role: "assistant",
    name: profileName || "Softwake",
    text: clean,
    ts: Date.now(),
    error: !!isError,
    note: isError ? "" : replyNote(detail, clean),
  });
}

function isThinking(message) {
  return message === "thinking…" || message === "thinking...";
}

function considerStatus(message, detail) {
  const text = message || "";
  if (!text || holding) {
    return;
  }
  const key = text + "\0" + (detail || "");
  if (key === lastReplyKey) {
    return;
  }
  if (text === "listening" || text === "listening…") {
    lastReplyKey = key;
    return;
  }
  if (isThinking(text)) {
    lastReplyKey = key;
    setLive(detail ? "thinking… (" + detail + ")" : "thinking…", false);
    setExpanded(true);
    return;
  }
  lastReplyKey = key;
  setLive("", false);
  pushAssistant(text, detail, false);
  setExpanded(true);
}

async function refreshIdlePref() {
  try {
    const snap = await invoke("ui_prefs_snapshot");
    if (snap && typeof snap.hud_idle_collapse_ms === "number") {
      setConfiguredIdle(snap.hud_idle_collapse_ms);
    }
  } catch (_error) {
    capsule.dataset.idleMs = String(configuredIdleMs);
  }
}

async function refreshProfileName(force) {
  const now = Date.now();
  if (!force && now - profilePollAt < 5000) {
    return;
  }
  profilePollAt = now;
  try {
    const snap = await invoke("profiles_snapshot", { selectedId: null });
    const rows = (snap && snap.profiles) || [];
    const active = rows.find((row) => row && row.active);
    const name = active && active.name ? String(active.name).trim() : "";
    profileName = name || "Softwake";
    capsule.dataset.profile = profileName;
  } catch (_error) {
    capsule.dataset.profile = profileName;
  }
}

function hideAllow() {
  allowOfferName = "";
  if (allowBar) {
    allowBar.hidden = true;
  }
}

function applyPending(pending) {
  const nextId = pending && pending.pending_id ? String(pending.pending_id) : "";
  if (!nextId) {
    const had = pendingToolId !== null;
    pendingToolId = null;
    if (pendingCard) {
      pendingCard.hidden = true;
    }
    if (had) {
      markActivity();
    }
    return;
  }
  const changed = nextId !== pendingToolId;
  pendingToolId = nextId;
  if (pendingNameEl) {
    pendingNameEl.textContent = pending.name || "";
  }
  if (pendingDescEl) {
    pendingDescEl.textContent = pending.description || "";
  }
  const args = (pending.args || []).join(" ");
  const description = pending.description || "";
  const showArgs = !!args && args !== description;
  if (pendingArgsEl) {
    pendingArgsEl.hidden = !showArgs;
    pendingArgsEl.textContent = showArgs ? args : "";
  }
  if (pendingCard) {
    pendingCard.hidden = false;
  }
  if (!pendingBusy && approveBtn && denyBtn) {
    approveBtn.disabled = false;
    denyBtn.disabled = false;
  }
  if (changed) {
    hideAllow();
    const askFocused = document.activeElement === input;
    setExpanded(true);
    if (!askFocused && approveBtn) {
      approveBtn.focus();
    }
  }
}

async function maybeOfferAlwaysAllow(name) {
  if (!name) {
    return;
  }
  try {
    const snap = await invoke("tools_snapshot");
    const row = ((snap && snap.tools) || []).find((tool) => tool && tool.name === name);
    if (!row || row.permission !== "ask") {
      return;
    }
    allowOfferName = name;
    if (allowText) {
      allowText.textContent = "Always allow " + name + "?";
    }
    if (allowBar) {
      allowBar.hidden = false;
    }
  } catch (_error) {
    hideAllow();
  }
}

async function approvePending() {
  if (!pendingToolId || pendingBusy) {
    return;
  }
  const id = pendingToolId;
  const name = pendingNameEl ? pendingNameEl.textContent : "";
  pendingBusy = true;
  if (approveBtn) {
    approveBtn.disabled = true;
  }
  if (denyBtn) {
    denyBtn.disabled = true;
  }
  try {
    const status = await invoke("confirm_tool", { pendingId: id });
    applyPending(status && status.pending_tool);
    await maybeOfferAlwaysAllow(name);
  } catch (_error) {
    // The next poll refreshes the card.
  } finally {
    pendingBusy = false;
    if (pendingToolId && approveBtn && denyBtn) {
      approveBtn.disabled = false;
      denyBtn.disabled = false;
    }
  }
}

async function denyPending() {
  if (!pendingToolId || pendingBusy) {
    return;
  }
  const id = pendingToolId;
  pendingBusy = true;
  if (approveBtn) {
    approveBtn.disabled = true;
  }
  if (denyBtn) {
    denyBtn.disabled = true;
  }
  hideAllow();
  try {
    const status = await invoke("cancel_tool", { pendingId: id });
    applyPending(status && status.pending_tool);
  } catch (_error) {
    // The next poll refreshes the card.
  } finally {
    pendingBusy = false;
    if (pendingToolId && approveBtn && denyBtn) {
      approveBtn.disabled = false;
      denyBtn.disabled = false;
    }
  }
}

async function acceptAlwaysAllow() {
  const name = allowOfferName;
  if (!name) {
    hideAllow();
    return;
  }
  if (allowYes) {
    allowYes.disabled = true;
  }
  try {
    await invoke("tools_set_permission", { name: name, permission: "always_allow" });
  } catch (_error) {
    // The Tools page can still write the mode.
  } finally {
    if (allowYes) {
      allowYes.disabled = false;
    }
    hideAllow();
  }
}

async function refresh() {
  if (refreshInFlight) {
    return;
  }
  refreshInFlight = true;
  try {
    const snap = await invoke("hud_snapshot");
    state = snap.state || "sleep";
    captureRunning = !!snap.capture_running;
    level = typeof snap.level === "number" ? snap.level : 0.02;
    autoListening = !!snap.auto_listening;
    const message = (snap && snap.message) || "";
    const detail = (snap && snap.detail) || "";
    lastStatusMessage = message;
    considerStatus(message, detail);
    applyPending(snap && snap.pending_tool);
  } catch (_error) {
    state = "sleep";
    captureRunning = false;
    level = 0.02;
    autoListening = false;
  } finally {
    refreshInFlight = false;
  }
  updateHint();
  void refreshIdlePref();
  void refreshProfileName(false);
}

function errorText(error, fallback) {
  if (typeof error === "string") {
    return error;
  }
  if (error && error.message) {
    return error.message;
  }
  return fallback;
}

async function beginTalk() {
  if (holding || talkPending || state === "hibernate") {
    if (state === "hibernate") {
      setExpanded(true);
      const line =
        "Softwake is hibernating — leave hibernate from Settings (Resume) first";
      setLive(line, true);
      pushAssistant(line, "", true);
    }
    return;
  }
  holding = true;
  talkBtn.classList.add("holding");
  talkBtn.setAttribute("aria-pressed", "true");
  setExpanded(true);
  markActivity();
  setLive("listening…", false);
  try {
    await invoke("hud_talk_start");
    await refresh();
  } catch (error) {
    holding = false;
    talkBtn.classList.remove("holding");
    talkBtn.setAttribute("aria-pressed", "false");
    const line = errorText(error, "could not start listening");
    setLive(line, true);
    pushAssistant(line, "", true);
  }
}

function paintReleasedMic(label) {
  holding = false;
  talkBtn.classList.remove("holding");
  talkBtn.setAttribute("aria-pressed", "false");
  lastStatusMessage = isThinking(label) ? "thinking…" : lastStatusMessage;
  setLive(label, false);
}

/** Yield two animation frames so the released mic paints before a long invoke. */
function afterPaint() {
  return new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(resolve));
  });
}

function endTalk() {
  if (!holding || talkPending) {
    return;
  }
  talkPending = true;
  paintReleasedMic("thinking…");
  markActivity();
  // Fire-and-forget: never await STT/ask/TTS on the HUD event loop. Bloom rAF
  // must keep running; the daemon pushes the reply onto status for refresh().
  afterPaint().then(() =>
    invoke("hud_talk_stop")
      .then((status) => {
        applyPending(status && status.pending_tool);
        const message = (status && (status.message || status.detail)) || "(no reply text)";
        const detail = (status && status.detail) || "";
        lastReplyKey = message + "\0" + detail;
        lastStatusMessage = message;
        if (isThinking(message) || message === "listening") {
          setLive(message, false);
          return;
        }
        setLive("", false);
        pushAssistant(message, detail, false);
      })
      .catch((error) => {
        const line = errorText(error, "talk failed");
        setLive(line, true);
        pushAssistant(line, "", true);
      })
      .finally(() => {
        talkPending = false;
        markActivity();
        refresh();
      }),
  );
}

talkBtn.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  event.stopPropagation();
  talkBtn.setPointerCapture(event.pointerId);
  beginTalk();
});

talkBtn.addEventListener("pointerup", (event) => {
  event.preventDefault();
  event.stopPropagation();
  try {
    talkBtn.releasePointerCapture(event.pointerId);
  } catch (_error) {
    // Capture may already be released.
  }
  endTalk();
});

talkBtn.addEventListener("pointercancel", (event) => {
  event.stopPropagation();
  try {
    talkBtn.releasePointerCapture(event.pointerId);
  } catch (_error) {
    // ignore
  }
  endTalk();
});

talkBtn.addEventListener("keydown", (event) => {
  event.stopPropagation();
  if (event.key === " " || event.key === "Enter") {
    event.preventDefault();
    if (!event.repeat) {
      beginTalk();
    }
  }
});

talkBtn.addEventListener("keyup", (event) => {
  event.stopPropagation();
  if (event.key === " " || event.key === "Enter") {
    event.preventDefault();
    endTalk();
  }
});

function isTypingTarget(target) {
  if (!target || !(target instanceof Element)) {
    return false;
  }
  return !!target.closest("input, textarea, button, select, [contenteditable], #ask-form");
}

let dragMoved = false;
let dragStartX = 0;
let dragStartY = 0;

function isInteractiveTarget(target) {
  if (!target || !(target instanceof Element)) {
    return false;
  }
  return !!target.closest(
    "#ask-form, #talk, #ask-send, #ask-input, #log, #live, #hud-pending, #hud-allow, button, input, a, .bubble",
  );
}

async function startHudDrag() {
  try {
    await invoke("hud_start_drag");
  } catch (_error) {
    // Drag is best-effort on platforms that refuse it.
  }
}

async function saveHudPosition() {
  try {
    await invoke("hud_save_position");
  } catch (_error) {
    // Persist is best-effort; layout still works from BR default.
  }
}

capsule.addEventListener("pointerdown", (event) => {
  if (event.button !== 0 || isInteractiveTarget(event.target)) {
    return;
  }
  dragMoved = false;
  dragStartX = event.screenX;
  dragStartY = event.screenY;
  startHudDrag();
});

window.addEventListener("pointerup", (event) => {
  if (dragStartX === 0 && dragStartY === 0) {
    return;
  }
  const dx = Math.abs(event.screenX - dragStartX);
  const dy = Math.abs(event.screenY - dragStartY);
  dragMoved = dx > 4 || dy > 4;
  dragStartX = 0;
  dragStartY = 0;
  if (dragMoved) {
    saveHudPosition();
  }
});

capsule.addEventListener("click", (event) => {
  if (dragMoved) {
    dragMoved = false;
    return;
  }
  if (
    event.target.closest("#ask-form") ||
    event.target.closest("#log") ||
    event.target.closest("#live") ||
    event.target.closest("#talk") ||
    event.target.closest("#hud-pending") ||
    event.target.closest("#hud-allow")
  ) {
    return;
  }
  setExpanded(!expanded);
});

capsule.addEventListener("keydown", (event) => {
  if (event.key !== "Enter" && event.key !== " ") {
    return;
  }
  // Space/Enter on the capsule toggles expand. Never steal keys from the ask field.
  if (event.target !== capsule || isTypingTarget(event.target)) {
    return;
  }
  event.preventDefault();
  setExpanded(!expanded);
});

capsule.addEventListener("pointerenter", () => {
  pointerOver = true;
  markActivity();
});

capsule.addEventListener("pointerleave", () => {
  pointerOver = false;
  markActivity();
});

capsule.addEventListener("pointermove", () => {
  pointerOver = true;
  markActivity();
});

input.addEventListener("input", () => markActivity());
input.addEventListener("focus", () => markActivity());
input.addEventListener("keydown", (event) => {
  // Stop capsule handlers from seeing Space/Enter while typing.
  event.stopPropagation();
  markActivity();
});

form.addEventListener("submit", (event) => {
  event.preventDefault();
  const asked = input.value.trim();
  if (!asked || talkPending) {
    return;
  }
  setExpanded(true);
  markActivity();
  pushUser(asked);
  setLive("thinking…", false);
  lastStatusMessage = "thinking…";
  input.value = "";
  talkPending = true;
  invoke("hud_ask", { text: asked })
    .then((status) => {
      applyPending(status && status.pending_tool);
      const message = (status && (status.message || status.detail)) || "(no reply text)";
      const detail = (status && status.detail) || "";
      lastReplyKey = message + "\0" + detail;
      lastStatusMessage = message;
      if (isThinking(message)) {
        setLive(message, false);
        return;
      }
      setLive("", false);
      pushAssistant(message, detail, false);
    })
    .catch((error) => {
      const line = errorText(error, "ask failed");
      setLive(line, true);
      pushAssistant(line, "", true);
    })
    .finally(() => {
      talkPending = false;
      markActivity();
      refresh();
    });
});

if (approveBtn) {
  approveBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    approvePending();
  });
}
if (denyBtn) {
  denyBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    denyPending();
  });
}
if (allowYes) {
  allowYes.addEventListener("click", (event) => {
    event.stopPropagation();
    acceptAlwaysAllow();
  });
}
if (allowNo) {
  allowNo.addEventListener("click", (event) => {
    event.stopPropagation();
    hideAllow();
  });
}

capsule.dataset.idleMs = String(configuredIdleMs);
capsule.dataset.profile = profileName;
updateHint();
refresh();
void refreshProfileName(true);
window.setInterval(refresh, 900);
raf = requestAnimationFrame(tick);
