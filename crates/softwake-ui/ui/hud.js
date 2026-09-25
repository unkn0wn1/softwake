const capsule = document.querySelector("#capsule");
const canvas = document.querySelector("#bloom");
const ctx = canvas.getContext("2d");
const strip = document.querySelector("#strip");
const form = document.querySelector("#ask-form");
const input = document.querySelector("#ask-input");
const replyEl = document.querySelector("#reply");
const talkBtn = document.querySelector("#talk");

const IDLE_MS = 4000;
const POST_ASK_IDLE_MS = 12000;
const particles = [];
let level = 0.02;
let state = "sleep";
let captureRunning = false;
let expanded = false;
let idleTimer = null;
let idleMs = IDLE_MS;
let raf = 0;
let holding = false;
let talkPending = false;
let autoListening = false;
let lastReplyKey = "";
let refreshInFlight = false;
let lastStatusMessage = "";

function invoke(command, args) {
  const core = window.__TAURI__ && window.__TAURI__.core;
  if (!core) {
    return Promise.reject(new Error("window bridge is not available"));
  }
  return core.invoke(command, args);
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

function spawnBurstWithLevel(drawLevel) {
  const dens = Math.floor(2 + drawLevel * 14);
  const colors = palette();
  const cx = canvas.width * 0.5;
  const cy = canvas.height * 0.55;
  for (let i = 0; i < dens; i += 1) {
    const color = colors[Math.floor(Math.random() * colors.length)];
    const angle = Math.random() * Math.PI * 2;
    const speed = 0.2 + Math.random() * (0.6 + drawLevel * 1.8);
    particles.push({
      x: cx + (Math.random() - 0.5) * 18,
      y: cy + (Math.random() - 0.5) * 10,
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

function spawnBurst() {
  spawnBurstWithLevel(bloomLevel());
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
  // Bloom is decoupled from status fetch: always paint from last-known level.
  const drawLevel = bloomLevel();
  ctx.clearRect(0, 0, canvas.width, canvas.height);
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

function setExpanded(next) {
  if (expanded === next) {
    if (next) {
      bumpIdle();
    }
    return;
  }
  expanded = next;
  capsule.classList.toggle("expanded", next);
  void applyWindowLayout(next);
  if (next) {
    strip.hidden = false;
    strip.classList.remove("fading");
    input.focus();
    bumpIdle();
  } else {
    strip.classList.add("fading");
    idleMs = IDLE_MS;
    window.setTimeout(() => {
      if (!expanded) {
        strip.hidden = true;
        strip.classList.remove("fading");
        replyEl.textContent = "";
        replyEl.classList.remove("error");
      }
    }, 450);
  }
}

function bumpIdle(ms) {
  if (typeof ms === "number") {
    idleMs = ms;
  }
  if (idleTimer) {
    window.clearTimeout(idleTimer);
  }
  idleTimer = window.setTimeout(() => {
    if (document.activeElement === input && input.value.trim()) {
      bumpIdle();
      return;
    }
    setExpanded(false);
  }, idleMs);
}

function updateHint() {
  const hint = document.querySelector("#hint");
  if (!hint) {
    return;
  }
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

async function refresh() {
  // Never stack status polls: a stuck GetStatus must not queue behind itself.
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
    const key = message + "\0" + detail;
    // Surface free-speech / async replies without awaiting the pipeline.
    if (
      message &&
      key !== lastReplyKey &&
      message !== "listening" &&
      !(talkPending && message === "thinking…")
    ) {
      lastReplyKey = key;
      if (message === "thinking…") {
        showReply(detail ? "thinking… (" + detail + ")" : "thinking…", false);
        setExpanded(true);
        bumpIdle(POST_ASK_IDLE_MS);
      } else if (!holding) {
        showReply(detail && detail !== message ? message + " — " + detail : message, false);
        setExpanded(true);
        bumpIdle(POST_ASK_IDLE_MS);
      }
    }
  } catch (_error) {
    state = "sleep";
    captureRunning = false;
    level = 0.02;
    autoListening = false;
  } finally {
    refreshInFlight = false;
  }
  updateHint();
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

function showReply(text, isError) {
  replyEl.classList.toggle("error", !!isError);
  replyEl.textContent = text;
}

async function beginTalk() {
  if (holding || talkPending || state === "hibernate") {
    if (state === "hibernate") {
      setExpanded(true);
      showReply(
        "Softwake is hibernating — leave hibernate from Settings (Wake) or the tray first",
        true,
      );
    }
    return;
  }
  holding = true;
  talkBtn.classList.add("holding");
  talkBtn.textContent = "…";
  setExpanded(true);
  bumpIdle(POST_ASK_IDLE_MS);
  showReply("listening…", false);
  try {
    await invoke("hud_talk_start");
    await refresh();
  } catch (error) {
    holding = false;
    talkBtn.classList.remove("holding");
    talkBtn.textContent = "Hold";
    showReply(errorText(error, "could not start listening"), true);
  }
}

function paintReleasedMic(label) {
  holding = false;
  talkBtn.classList.remove("holding");
  talkBtn.textContent = "Hold";
  lastStatusMessage = label === "thinking…" || label === "thinking..." ? "thinking…" : lastStatusMessage;
  showReply(label, false);
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
  bumpIdle(POST_ASK_IDLE_MS);
  // Fire-and-forget: never await STT/ask/TTS on the HUD event loop. Bloom rAF
  // must keep running; the daemon pushes the reply onto status for refresh().
  afterPaint().then(() =>
    invoke("hud_talk_stop")
      .then((status) => {
        const message =
          (status && (status.message || status.detail)) || "(no reply text)";
        const speechNote =
          status && status.detail && status.message ? status.detail : "";
        const line = speechNote ? message + " — " + speechNote : message;
        lastReplyKey = message + "\0" + (status && status.detail ? status.detail : "");
        showReply(line, false);
      })
      .catch((error) => {
        showReply(errorText(error, "talk failed"), true);
      })
      .finally(() => {
        talkPending = false;
        bumpIdle(POST_ASK_IDLE_MS);
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
  // Release chrome before the long STT→ask→TTS round trip.
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
  return !!target.closest("#ask-form, #talk, #ask-send, #ask-input, button, input, a, .reply");
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
  // Native drag; click-to-expand still runs on pointerup if we did not move.
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
  if (event.target.closest("#ask-form") || event.target.closest(".reply") || event.target.closest("#talk")) {
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

input.addEventListener("input", () => bumpIdle());
input.addEventListener("focus", () => bumpIdle());
input.addEventListener("keydown", (event) => {
  // Stop capsule handlers from seeing Space/Enter while typing.
  event.stopPropagation();
});

form.addEventListener("submit", (event) => {
  event.preventDefault();
  const asked = input.value.trim();
  if (!asked || talkPending) {
    return;
  }
  setExpanded(true);
  bumpIdle(POST_ASK_IDLE_MS);
  replyEl.classList.remove("error");
  replyEl.textContent = "thinking…";
  lastStatusMessage = "thinking…";
  input.value = "";
  talkPending = true;
  // Same as PTT: do not block the particle loop on ask + Eve playback.
  invoke("hud_ask", { text: asked })
    .then((status) => {
      const message =
        (status && (status.message || status.detail)) || "(no reply text)";
      lastReplyKey = message + "\0" + (status && status.detail ? status.detail : "");
      replyEl.textContent = message;
    })
    .catch((error) => {
      replyEl.classList.add("error");
      replyEl.textContent =
        typeof error === "string"
          ? error
          : error && error.message
            ? error.message
            : "ask failed";
    })
    .finally(() => {
      talkPending = false;
      bumpIdle(POST_ASK_IDLE_MS);
      refresh();
    });
});

refresh();
window.setInterval(refresh, 900);
raf = requestAnimationFrame(tick);
