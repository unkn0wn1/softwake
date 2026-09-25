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

function spawnBurst() {
  const dens = Math.floor(2 + level * 14);
  const colors = palette();
  const cx = canvas.width * 0.5;
  const cy = canvas.height * 0.55;
  for (let i = 0; i < dens; i += 1) {
    const color = colors[Math.floor(Math.random() * colors.length)];
    const angle = Math.random() * Math.PI * 2;
    const speed = 0.2 + Math.random() * (0.6 + level * 1.8);
    particles.push({
      x: cx + (Math.random() - 0.5) * 18,
      y: cy + (Math.random() - 0.5) * 10,
      vx: Math.cos(angle) * speed,
      vy: Math.sin(angle) * speed - 0.15,
      r: 3 + Math.random() * (4 + level * 10),
      life: 1,
      decay: 0.008 + Math.random() * 0.012,
      color,
      alpha: 0.25 + level * 0.55,
    });
  }
}

function tick() {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  const listening = captureRunning || level > 0.08;
  if (listening && Math.random() < 0.08 + level * 0.35) {
    spawnBurst();
  } else if (!listening && Math.random() < 0.02) {
    spawnBurst();
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

async function refresh() {
  try {
    const snap = await invoke("hud_snapshot");
    state = snap.state || "sleep";
    captureRunning = !!snap.capture_running;
    level = typeof snap.level === "number" ? snap.level : 0.02;
  } catch (_error) {
    state = "sleep";
    captureRunning = false;
    level = 0.02;
  }
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

async function endTalk() {
  if (!holding || talkPending) {
    return;
  }
  holding = false;
  talkPending = true;
  talkBtn.classList.remove("holding");
  talkBtn.textContent = "Hold";
  showReply("…", false);
  bumpIdle(POST_ASK_IDLE_MS);
  try {
    const status = await invoke("hud_talk_stop");
    const message =
      (status && (status.message || status.detail)) || "(no reply text)";
    const speechNote = status && status.detail && status.message ? status.detail : "";
    showReply(speechNote ? message + " — " + speechNote : message, false);
    await refresh();
  } catch (error) {
    showReply(errorText(error, "talk failed"), true);
  }
  talkPending = false;
  bumpIdle(POST_ASK_IDLE_MS);
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
  endTalk();
});

talkBtn.addEventListener("pointercancel", (event) => {
  event.stopPropagation();
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

capsule.addEventListener("click", (event) => {
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

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const text = input.value.trim();
  if (!text) {
    return;
  }
  setExpanded(true);
  bumpIdle(POST_ASK_IDLE_MS);
  replyEl.classList.remove("error");
  replyEl.textContent = "…";
  try {
    const status = await invoke("hud_ask", { text });
    const message =
      (status && (status.message || status.detail)) || "(no reply text)";
    replyEl.textContent = message;
    input.value = "";
    await refresh();
  } catch (error) {
    replyEl.classList.add("error");
    replyEl.textContent =
      typeof error === "string"
        ? error
        : error && error.message
          ? error.message
          : "ask failed";
  }
  bumpIdle(POST_ASK_IDLE_MS);
});

refresh();
window.setInterval(refresh, 900);
raf = requestAnimationFrame(tick);
