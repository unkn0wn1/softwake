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
const vaultGate = document.querySelector("#vault-gate");
const vaultTitle = document.querySelector("#vault-title");
const vaultHint = document.querySelector("#vault-hint");
const vaultPassWrap = document.querySelector("#vault-pass-wrap");
const vaultPass = document.querySelector("#vault-pass");
const vaultError = document.querySelector("#vault-error");
const vaultUnlockBtn = document.querySelector("#vault-unlock");
const vaultSetBtn = document.querySelector("#vault-set");
const vaultSkipBtn = document.querySelector("#vault-skip");
const contextMeter = document.querySelector("#context-meter");
const contextMeterFill = document.querySelector("#context-meter-fill");
const contextMeterMark = document.querySelector("#context-meter-mark");
const contextMeterLabel = document.querySelector("#context-meter-label");
const pinBtn = document.querySelector("#pin");
const micMuteBtn = document.querySelector("#mic-mute");
const resizeGrip = document.querySelector("#resize-grip");
const chatToolbar = document.querySelector("#chat-toolbar");
const selectToggle = document.querySelector("#select-toggle");
const deleteSelectedBtn = document.querySelector("#delete-selected");
const selectCountEl = document.querySelector("#select-count");
const profileRail = document.querySelector("#profile-rail");
const profileRailList = document.querySelector("#profile-rail-list");

const IDLE_MIN_MS = 1000;
const IDLE_MAX_MS = 30000;
const IDLE_DEFAULT_MS = 3000;
const MAX_TURNS = 40;

const particles = [];
let level = 0.02;
let state = "sleep";
let previousVoiceState = "sleep";
let captureRunning = false;
let expanded = false;
let idleTimer = null;
let configuredIdleMs = IDLE_DEFAULT_MS;
let hudPinned = false;
let lastActivity = Date.now();
let pointerOver = false;
let raf = 0;
let holding = false;
let talkPending = false;
let autoListening = false;
let lastReplyKey = "";
let refreshInFlight = false;
let lastStatusMessage = "";
let micMuted = false;
let lastPhase = "";
let profileName = "Softwake";
let profilePollAt = 0;
let pendingToolId = null;
let pendingBusy = false;
let allowOfferName = "";
let viewW = 120;
let viewH = 120;
const turns = [];
let profileId = "";
let vaultUnlocked = false;
let chatPersistReady = false;
let chatSaveTimer = null;
/** True after we successfully dispatched SeedChat for this awake period. */
let sessionHudSeeded = false;
let selecting = false;
/** Indices into `turns` currently selected for delete. */
const selectedIdx = new Set();
/** True while a left-rail profile switch is in flight. */
let profileSwitchBusy = false;

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
  if (hudPinned || pointerOver || holding || talkPending || pendingToolId) {
    return true;
  }
  return !!input.value.trim();
}

function armIdle() {
  if (idleTimer) {
    window.clearTimeout(idleTimer);
    idleTimer = null;
  }
  if (!expanded || hudPinned) {
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

function applyHudOpacityPercent(percent) {
  const n = Math.max(35, Math.min(100, Number(percent) || 55));
  capsule.style.setProperty("--hud-bg-alpha", String(n / 100));
}

function applyMicMuted(next, syncDaemon) {
  micMuted = !!next;
  if (micMuteBtn) {
    micMuteBtn.hidden = !expanded;
    micMuteBtn.setAttribute("aria-pressed", micMuted ? "true" : "false");
    micMuteBtn.setAttribute(
      "aria-label",
      micMuted ? "Unmute microphone" : "Mute microphone",
    );
    micMuteBtn.title = micMuted
      ? "Mic muted — click to unmute (text ask still works)"
      : "Mute microphone listening";
  }
  if (syncDaemon) {
    void invoke("hud_set_mic_mute", { muted: micMuted })
      .then((status) => {
        if (status && typeof status.mic_muted === "boolean") {
          micMuted = !!status.mic_muted;
        }
        if (micMuted) {
          setLive("Mic muted — type to ask", false);
        }
      })
      .catch((error) => {
        setLive(errorText(error, "mic mute failed"), true);
      });
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
    if (chatToolbar) {
      chatToolbar.hidden = false;
    }
    if (pinBtn) {
      pinBtn.hidden = false;
    }
    if (micMuteBtn) {
      micMuteBtn.hidden = false;
    }
    if (resizeGrip) {
      resizeGrip.hidden = false;
    }
    input.focus();
    markActivity();
    void refreshProfileName(true);
  } else {
    strip.hidden = true;
    liveEl.hidden = true;
    if (chatToolbar) {
      chatToolbar.hidden = true;
    }
    setSelecting(false);
    if (pinBtn) {
      pinBtn.hidden = true;
    }
    if (micMuteBtn) {
      micMuteBtn.hidden = true;
    }
    if (resizeGrip) {
      resizeGrip.hidden = true;
    }
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
  capsule.classList.toggle("thinking", !!line && !isError);
  // Keep the last bubble above the thinking / live status line.
  if (line && logEl.lastElementChild) {
    logEl.lastElementChild.scrollIntoView({ block: "end", behavior: "smooth" });
  }
}

function formatClock(ts) {
  return new Date(ts).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function appendBubble(turn, index) {
  const article = document.createElement("article");
  article.className = "bubble " + (turn.role === "user" ? "user" : "assistant");
  article.dataset.index = String(index);
  if (turn.error) {
    article.classList.add("error");
  }
  if (selectedIdx.has(index)) {
    article.classList.add("selected");
  }
  if (selecting) {
    const pick = document.createElement("input");
    pick.type = "checkbox";
    pick.className = "pick";
    pick.checked = selectedIdx.has(index);
    pick.tabIndex = -1;
    pick.addEventListener("click", (event) => {
      event.stopPropagation();
      toggleSelectIndex(index);
    });
    article.append(pick);
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
  article.addEventListener("click", (event) => {
    if (!selecting) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    toggleSelectIndex(index);
  });
  logEl.append(article);
  logEl.scrollTop = logEl.scrollHeight;
}

function renderLog() {
  logEl.replaceChildren();
  turns.forEach((turn, index) => {
    appendBubble(turn, index);
  });
  syncSelectChrome();
}

function setSelecting(next) {
  selecting = !!next;
  selectedIdx.clear();
  capsule.classList.toggle("selecting", selecting);
  if (selectToggle) {
    selectToggle.setAttribute("aria-pressed", selecting ? "true" : "false");
    selectToggle.textContent = selecting ? "Cancel" : "Select";
  }
  renderLog();
  syncSelectChrome();
}

function toggleSelectIndex(index) {
  if (selectedIdx.has(index)) {
    selectedIdx.delete(index);
  } else {
    selectedIdx.add(index);
  }
  renderLog();
}

function syncSelectChrome() {
  const n = selectedIdx.size;
  if (deleteSelectedBtn) {
    deleteSelectedBtn.disabled = !selecting || n === 0;
  }
  if (selectCountEl) {
    if (selecting && n > 0) {
      selectCountEl.hidden = false;
      selectCountEl.textContent = n + " selected";
    } else {
      selectCountEl.hidden = true;
      selectCountEl.textContent = "";
    }
  }
}

async function deleteSelectedTurns() {
  if (!selecting || selectedIdx.size === 0) {
    return;
  }
  const doomed = Array.from(selectedIdx).sort((a, b) => a - b);
  const removed = doomed.map((i) => turns[i]).filter(Boolean);
  for (let i = doomed.length - 1; i >= 0; i -= 1) {
    turns.splice(doomed[i], 1);
  }
  selectedIdx.clear();
  renderLog();
  scheduleChatSave();
  setSelecting(false);
  markActivity();
  // Best-effort trim matching model-session turns while awake.
  if (state === "awake" && removed.length) {
    try {
      await invoke("hud_drop_session_turns", {
        turns: removed.map((turn) => ({
          role: turn.role,
          text: turn.text,
          error: !!turn.error,
        })),
      });
    } catch (_error) {
      // HUD history is already updated; /clear still clears the model session.
    }
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
  } else {
    appendBubble(turn, turns.length - 1);
  }
  scheduleChatSave();
}

function scheduleChatSave() {
  if (!chatPersistReady || !vaultUnlocked) {
    return;
  }
  if (chatSaveTimer) {
    window.clearTimeout(chatSaveTimer);
  }
  chatSaveTimer = window.setTimeout(() => {
    chatSaveTimer = null;
    void persistChat();
  }, 250);
}

async function persistChat() {
  if (!chatPersistReady || !vaultUnlocked) {
    return;
  }
  try {
    await invoke("hud_chat_save", {
      profileId: profileId || null,
      turns: turns.map((turn) => ({
        role: turn.role,
        name: turn.name,
        text: turn.text,
        ts: turn.ts,
        error: !!turn.error,
        note: turn.note || "",
      })),
    });
  } catch (_error) {
    // Keep the on-screen log; next successful save retries.
  }
}

function replaceTurns(next) {
  turns.length = 0;
  for (const turn of next || []) {
    turns.push({
      role: turn.role || "assistant",
      name: turn.name || "",
      text: turn.text || "",
      ts: Number(turn.ts) || Date.now(),
      error: !!turn.error,
      note: turn.note || "",
    });
  }
  while (turns.length > MAX_TURNS) {
    turns.shift();
  }
  renderLog();
}

function showVaultError(message) {
  if (!vaultError) {
    return;
  }
  const text = String(message || "").trim();
  if (!text) {
    vaultError.hidden = true;
    vaultError.textContent = "";
    return;
  }
  vaultError.hidden = false;
  vaultError.textContent = text;
}

function setVaultGate(visible, mode) {
  if (!vaultGate) {
    return;
  }
  vaultGate.hidden = !visible;
  if (!visible) {
    return;
  }
  const unset = mode === "unset";
  const locked = mode === "passphrase";
  if (vaultTitle) {
    vaultTitle.textContent = locked ? "Unlock chat history" : "Chat lock";
  }
  if (vaultHint) {
    if (locked) {
      vaultHint.textContent = "Enter your Softwake passphrase to show saved HUD chats for this profile.";
    } else if (unset) {
      vaultHint.textContent =
        "Optional: set a passphrase to encrypt HUD chat history at rest. Skip keeps chats as plaintext on disk.";
    } else {
      vaultHint.textContent = "HUD chats are stored as plaintext. Set a passphrase anytime in Settings → General.";
    }
  }
  if (vaultPassWrap) {
    vaultPassWrap.hidden = !(locked || unset);
  }
  if (vaultUnlockBtn) {
    vaultUnlockBtn.hidden = !locked;
  }
  if (vaultSetBtn) {
    vaultSetBtn.hidden = !(unset || mode === "plaintext");
  }
  if (vaultSkipBtn) {
    vaultSkipBtn.hidden = !unset;
  }
  showVaultError("");
  if (visible) {
    setExpanded(true);
  }
}

async function loadChatForActiveProfile() {
  if (!vaultUnlocked) {
    return;
  }
  try {
    const snap = await invoke("hud_chat_snapshot", { profileId: profileId || null });
    if (snap && snap.profile_id) {
      profileId = String(snap.profile_id);
    }
    replaceTurns((snap && snap.turns) || []);
    chatPersistReady = true;
    // Wake may have raced ahead of unlock/load; seed once turns are ready.
    maybeSeedSessionFromHud();
  } catch (error) {
    chatPersistReady = false;
    const line = errorText(error, "could not load chat history");
    if (String(line).includes("locked")) {
      vaultUnlocked = false;
      setVaultGate(true, "passphrase");
    }
  }
}

async function bootstrapVault() {
  try {
    await invoke("ui_vault_try_keyring");
  } catch (_error) {
    // Keyring missing is fine; fall through to status.
  }
  let status;
  try {
    status = await invoke("ui_vault_status");
  } catch (_error) {
    vaultUnlocked = true;
    setVaultGate(false, "plaintext");
    await loadChatForActiveProfile();
    return;
  }
  const mode = (status && status.mode) || "unset";
  const unlocked = !!(status && status.unlocked);
  if (mode === "passphrase" && !unlocked) {
    vaultUnlocked = false;
    chatPersistReady = false;
    setVaultGate(true, "passphrase");
    return;
  }
  vaultUnlocked = true;
  if (mode === "unset") {
    setVaultGate(true, "unset");
  } else if (mode === "plaintext" && status && status.plaintext_warning) {
    // Soft one-line warning stays in Settings; HUD loads immediately.
    setVaultGate(false, mode);
  } else {
    setVaultGate(false, mode);
  }
  await loadChatForActiveProfile();
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

function syncLastBubbleNote(note) {
  const article = logEl && logEl.lastElementChild;
  if (!article || !article.classList.contains("assistant")) {
    return;
  }
  let noteEl = article.querySelector(".note");
  if (!note) {
    if (noteEl) {
      noteEl.remove();
    }
    return;
  }
  if (!noteEl) {
    noteEl = document.createElement("p");
    noteEl.className = "note";
    article.append(noteEl);
  }
  noteEl.textContent = note;
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
  // Pre-TTS status poll (#101 reply-before-speak) and post-TTS ask/talk settle
  // deliver the same reply seconds apart. Dedupe consecutive identical assistant
  // text until a user turn intervenes — do not time-gate (TTS often exceeds 2s).
  // A later turn that repeats the words after the user speaks still gets its own bubble.
  if (
    last &&
    last.role === "assistant" &&
    last.text === clean &&
    !!last.error === !!isError
  ) {
    if (!isError) {
      const note = replyNote(detail, clean);
      if (note && note !== (last.note || "")) {
        last.note = note;
        syncLastBubbleNote(note);
        scheduleChatSave();
      }
    }
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

function phaseLabel(phase, detail) {
  switch (phase) {
    case "listening":
      return "Listening…";
    case "thinking":
      return detail && detail !== "thinking" && detail !== "ask" && detail !== "press to talk"
        ? "Thinking… (" + detail + ")"
        : "Thinking…";
    case "calling_tools":
      return detail && detail.indexOf("calling tools") === 0
        ? detail.replace(/^calling tools/, "Calling tools")
        : "Calling tools…";
    case "speaking":
      return "Speaking…";
    case "awaiting_approve":
      return "Waiting for approve…";
    default:
      return "";
  }
}

function isPhaseToken(text) {
  const t = (text || "").trim().toLowerCase();
  return (
    t === "listening" ||
    t === "listening…" ||
    t === "listening..." ||
    isThinking(t) ||
    t === "speaking" ||
    t === "speaking…" ||
    t.indexOf("calling tools") === 0 ||
    t === "waiting for approve" ||
    t === "mic muted — type to ask" ||
    t === "mic unmuted"
  );
}

function applyPhase(phase, message, detail) {
  lastPhase = phase || "";
  const label = phaseLabel(phase, detail);
  if (label) {
    setLive(label, false);
    return;
  }
  if (isThinking(message)) {
    setLive(detail ? "Thinking… (" + detail + ")" : "Thinking…", false);
  }
}

/** Clear stuck Thinking/Speaking live line after a turn settles. */
function clearThinkingLive() {
  lastPhase = "";
  if (!holding) {
    setLive("", false);
  }
}

/**
 * Ask/talk promise settled: never leave talkPending or Thinking stuck.
 * Keep Waiting for approve when a confirm card is up.
 */
function endTurnUi(status, hadError) {
  talkPending = false;
  const phase = (status && status.phase) || "";
  const pending = status && status.pending_tool;
  if (hadError) {
    clearThinkingLive();
    return;
  }
  if (phase === "awaiting_approve" || pending) {
    lastPhase = "awaiting_approve";
    setLive("Waiting for approve…", false);
    return;
  }
  if (phase === "speaking") {
    lastPhase = "speaking";
    setLive("Speaking…", false);
    return;
  }
  // Turn finished (incl. soft-finalize): clear thinking/calling_tools overlay.
  clearThinkingLive();
}

function considerStatus(message, detail, phase) {
  const text = message || "";
  if (holding) {
    return;
  }
  if (phase) {
    applyPhase(phase, text, detail);
  }
  if (!text) {
    return;
  }
  const key = text + "\0" + (detail || "") + "\0" + (phase || "");
  if (key === lastReplyKey) {
    return;
  }
  if (isPhaseToken(text) && phase !== "speaking") {
    lastReplyKey = key;
    // Stale thinking resurrection: daemon GetStatus after reject used to keep
    // thinking… with phase cleared. Do not re-stick Thinking when idle.
    if (!phase && isThinking(text) && !talkPending && !holding) {
      clearThinkingLive();
      return;
    }
    if (!phase) {
      if (text === "listening" || text === "listening…") {
        setLive("Listening…", false);
      } else if (isThinking(text)) {
        setLive(detail ? "Thinking… (" + detail + ")" : "Thinking…", false);
        setExpanded(true);
      }
    }
    return;
  }
  // Real assistant reply (including while phase is speaking) — clear thinking and show bubble ASAP.
  lastReplyKey = key;
  if (phase === "speaking") {
    setLive("Speaking…", false);
  } else if (phase === "awaiting_approve") {
    setLive("Waiting for approve…", false);
  } else {
    setLive("", false);
  }
  pushAssistant(text, detail, false);
  setExpanded(true);
}

function applyPinned(next) {
  hudPinned = !!next;
  capsule.dataset.pinned = hudPinned ? "1" : "0";
  if (pinBtn) {
    pinBtn.setAttribute("aria-pressed", hudPinned ? "true" : "false");
    pinBtn.setAttribute("aria-label", hudPinned ? "Unpin chat" : "Pin chat open");
    pinBtn.title = hudPinned ? "Pinned — click to unpin" : "Pin chat open";
    pinBtn.classList.toggle("is-on", hudPinned);
    const off = pinBtn.querySelector(".pin-icon-off");
    const on = pinBtn.querySelector(".pin-icon-on");
    if (off) {
      off.hidden = hudPinned;
    }
    if (on) {
      on.hidden = !hudPinned;
    }
  }
  if (hudPinned) {
    if (idleTimer) {
      window.clearTimeout(idleTimer);
      idleTimer = null;
    }
    if (!expanded) {
      setExpanded(true);
    }
  } else if (expanded) {
    armIdle();
  }
}

async function refreshHudPrefs() {
  try {
    const snap = await invoke("ui_prefs_snapshot");
    if (snap && typeof snap.hud_idle_collapse_ms === "number") {
      setConfiguredIdle(snap.hud_idle_collapse_ms);
    }
    if (snap && typeof snap.hud_pinned === "boolean") {
      applyPinned(snap.hud_pinned);
    }
    if (snap && typeof snap.hud_opacity === "number") {
      applyHudOpacityPercent(snap.hud_opacity);
    }
    if (snap && typeof snap.hud_mic_muted === "boolean" && snap.hud_mic_muted !== micMuted) {
      // Restore daemon latch from prefs once (avoid loop).
      void invoke("hud_set_mic_mute", { muted: !!snap.hud_mic_muted })
        .then((status) => {
          applyMicMuted(!!(status && status.mic_muted), false);
        })
        .catch(() => {
          applyMicMuted(!!snap.hud_mic_muted, false);
        });
    }
  } catch (_error) {
    // Prefs are best-effort; keep defaults when the snapshot fails.
  }
  capsule.dataset.idleMs = String(configuredIdleMs);
}

if (pinBtn) {
  pinBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    const next = !hudPinned;
    applyPinned(next);
    invoke("ui_prefs_set_hud_pinned", { pinned: next }).catch(() => {
      applyPinned(!next);
    });
  });
}

if (micMuteBtn) {
  micMuteBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    applyMicMuted(!micMuted, true);
  });
}


async function startHudResize() {
  try {
    const api = window.__TAURI__ && window.__TAURI__.window;
    const current = api && api.getCurrentWindow && api.getCurrentWindow();
    if (current && typeof current.startResizeDragging === "function") {
      await current.startResizeDragging("SouthEast");
      return;
    }
  } catch (_error) {
    // Fall through to manual size drag below when the plugin refuses.
  }
}

if (resizeGrip) {
  resizeGrip.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    event.stopPropagation();
    void startHudResize();
  });
}

window.addEventListener("pointerup", () => {
  if (!expanded) {
    return;
  }
  invoke("hud_save_size").catch(() => {
    // Persist is best-effort.
  });
});

function profileChipLabel(row) {
  const name = row && row.name ? String(row.name).trim() : "";
  const id = row && row.id ? String(row.id) : "";
  return name || id || "?";
}

function renderProfileRail(snap) {
  if (!profileRailList) {
    return;
  }
  const rows = (snap && snap.profiles) || [];
  const activeId =
    (snap && snap.active_id && String(snap.active_id)) ||
    profileId ||
    "";
  profileRailList.innerHTML = "";
  for (const row of rows) {
    if (!row || !row.id) {
      continue;
    }
    const id = String(row.id);
    const li = document.createElement("li");
    li.setAttribute("role", "presentation");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-rail-item";
    btn.setAttribute("role", "option");
    btn.dataset.profileId = id;
    const selected = id === activeId;
    btn.setAttribute("aria-selected", selected ? "true" : "false");
    btn.title = profileChipLabel(row) + " [" + id + "]";
    btn.textContent = profileChipLabel(row);
    btn.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      void switchHudProfile(id);
    });
    li.appendChild(btn);
    profileRailList.appendChild(li);
  }
  if (profileRail) {
    profileRail.hidden = rows.length === 0;
  }
}

async function switchHudProfile(nextId) {
  const id = String(nextId || "").trim();
  if (!id || profileSwitchBusy) {
    return;
  }
  if (id === profileId) {
    return;
  }
  profileSwitchBusy = true;
  markActivity();
  try {
    if (vaultUnlocked) {
      await persistChat();
    }
    const result = await invoke("hud_switch_profile", { id });
    const snap = result && result.snapshot ? result.snapshot : result;
    const rows = (snap && snap.profiles) || [];
    const active = rows.find((row) => row && row.active) || null;
    profileId = (snap && snap.active_id && String(snap.active_id)) || id;
    const name =
      (active && active.name && String(active.name).trim()) ||
      (snap && snap.selected_name && String(snap.selected_name).trim()) ||
      "";
    profileName = name || profileId || "Softwake";
    capsule.dataset.profile = profileName;
    profilePollAt = Date.now();
    renderProfileRail(snap);
    chatPersistReady = false;
    replaceTurns([]);
    if (vaultUnlocked) {
      await loadChatForActiveProfile();
    }
    const msg =
      (result && result.refresh_message && String(result.refresh_message)) ||
      "Switched profile";
    const failed = result && result.refresh_ok === false;
    setLive(msg, !!failed);
    sessionHudSeeded = false;
  } catch (error) {
    setLive(errorText(error, "profile switch failed"), true);
  } finally {
    profileSwitchBusy = false;
    markActivity();
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
    const nextId = active && active.id ? String(active.id) : "";
    profileName = name || "Softwake";
    capsule.dataset.profile = profileName;
    renderProfileRail(snap);
    if (nextId && nextId !== profileId) {
      const previous = profileId;
      profileId = nextId;
      if (previous && vaultUnlocked) {
        await persistChat();
        chatPersistReady = false;
        replaceTurns([]);
        await loadChatForActiveProfile();
      }
    } else if (nextId) {
      profileId = nextId;
    }
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

function applyContextMeter(snap) {
  if (!contextMeter || !contextMeterFill || !contextMeterLabel) {
    return;
  }
  const awake = (snap && snap.state) === "awake";
  const used = snap && snap.context_used != null ? Number(snap.context_used) : null;
  const limit = snap && snap.context_limit != null ? Number(snap.context_limit) : null;
  if (!awake || used == null || limit == null || !(limit > 0)) {
    contextMeter.hidden = true;
    contextMeterLabel.textContent = "";
    contextMeterFill.style.width = "0%";
    contextMeterFill.classList.remove("warn", "hot");
    return;
  }
  const pct = Math.min(100, Math.round((100 * used) / limit));
  const threshold =
    snap.context_compact_at != null && Number(snap.context_compact_at) > 0
      ? Math.min(100, Number(snap.context_compact_at))
      : 80;
  contextMeter.hidden = false;
  contextMeterFill.style.width = pct + "%";
  contextMeterFill.classList.toggle("warn", pct >= threshold - 10 && pct < threshold);
  contextMeterFill.classList.toggle("hot", pct >= threshold);
  if (contextMeterMark) {
    contextMeterMark.style.left = threshold + "%";
    contextMeterMark.title = "Auto-compact at " + threshold + "%";
  }
  let line = "context ~" + used + " / " + limit + " (" + pct + "%) · auto @" + threshold + "%";
  if (snap.context_compacted) {
    line += " · compacted";
  }
  contextMeterLabel.textContent = line;
}


/**
 * Dispatch SeedChat once when awake + vault unlocked + turns loaded.
 * Not only on the woke edge — unlock/load often finish after wake for encrypted vaults.
 */
function maybeSeedSessionFromHud() {
  if (sessionHudSeeded) {
    return;
  }
  if (state !== "awake") {
    return;
  }
  if (!vaultUnlocked || !chatPersistReady || !turns.length) {
    return;
  }
  const payload = turns
    .filter((turn) => turn && !turn.error && (turn.role === "user" || turn.role === "assistant"))
    .map((turn) => ({
      role: turn.role,
      text: turn.text || "",
      error: false,
    }));
  if (!payload.length) {
    return;
  }
  // Latch before the IPC round-trip so overlapping refresh/load calls do not double-send.
  sessionHudSeeded = true;
  invoke("hud_seed_session", { turns: payload }).catch(() => {
    // Allow retry (session may not be open yet; plaintext softwaked may already have seeded).
    sessionHudSeeded = false;
  });
}

/** @deprecated name kept for unlock handlers; delegates to maybeSeedSessionFromHud */
function seedSessionFromHud() {
  maybeSeedSessionFromHud();
}

async function refresh() {
  if (refreshInFlight) {
    return;
  }
  refreshInFlight = true;
  try {
    const snap = await invoke("hud_snapshot");
    const nextState = snap.state || "sleep";
    const woke = nextState === "awake" && previousVoiceState !== "awake";
    state = nextState;
    previousVoiceState = nextState;
    if (nextState !== "awake") {
      sessionHudSeeded = false;
    }
    // Seed whenever awake+unlocked+turns — not only on the woke edge (vault race).
    if (woke || nextState === "awake") {
      maybeSeedSessionFromHud();
    }
    captureRunning = !!snap.capture_running;
    level = typeof snap.level === "number" ? snap.level : 0.02;
    autoListening = !!snap.auto_listening;
    const message = (snap && snap.message) || "";
    const detail = (snap && snap.detail) || "";
    const phase = (snap && snap.phase) || "";
    lastStatusMessage = message;
    considerStatus(message, detail, phase);
    applyPending(snap && snap.pending_tool);
    applyContextMeter(snap);
  } catch (_error) {
    state = "sleep";
    previousVoiceState = "sleep";
    captureRunning = false;
    level = 0.02;
    autoListening = false;
    applyContextMeter({ state: "sleep" });
  } finally {
    refreshInFlight = false;
  }
  updateHint();
  void refreshHudPrefs();
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
        applyContextMeter({
          state: (status && status.state) || state,
          context_used: status && status.context_used,
          context_limit: status && status.context_limit,
          context_compact_at: status && status.context_compact_at,
          context_compacted: !!(status && status.context_compacted),
        });
        const message = (status && (status.message || status.detail)) || "(no reply text)";
        const detail = (status && status.detail) || "";
        lastReplyKey = message + "\0" + detail;
        lastStatusMessage = message;
        const phase = (status && status.phase) || "";
        if (phase) {
          applyPhase(phase, message, detail);
        }
        // Promise settled: stale thinking/phase tokens must not keep the overlay.
        if (isThinking(message) || isPhaseToken(message)) {
          endTurnUi(status, false);
          return;
        }
        if (phase === "speaking") {
          setLive("Speaking…", false);
        } else if (phase === "awaiting_approve") {
          setLive("Waiting for approve…", false);
        } else {
          setLive("", false);
        }
        pushAssistant(message, detail, false);
        endTurnUi(status, false);
      })
      .catch((error) => {
        const line = errorText(error, "talk failed");
        setLive(line, true);
        pushAssistant(line, "", true);
        endTurnUi(null, true);
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
    "#ask-form, #talk, #ask-send, #ask-input, #log, #live, #hud-pending, #hud-allow, #pin, #resize-grip, #chat-toolbar, #profile-rail, #vault-gate, button, input, textarea, a, .bubble",
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
    event.target.closest("#hud-allow") ||
    event.target.closest("#pin") ||
    event.target.closest("#resize-grip") ||
    event.target.closest("#profile-rail") ||
    event.target.closest("#vault-gate")
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

input.addEventListener("input", () => {
  markActivity();
  // Grow the textarea with content up to CSS max-height.
  input.style.height = "auto";
  input.style.height = Math.min(input.scrollHeight, 120) + "px";
});
input.addEventListener("focus", () => markActivity());
input.addEventListener("keydown", (event) => {
  // Stop capsule handlers from seeing Space/Enter while typing.
  event.stopPropagation();
  markActivity();
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    if (typeof form.requestSubmit === "function") {
      form.requestSubmit();
    } else {
      form.dispatchEvent(new Event("submit", { cancelable: true, bubbles: true }));
    }
  }
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
      applyContextMeter({
        state: (status && status.state) || state,
        context_used: status && status.context_used,
        context_limit: status && status.context_limit,
        context_compact_at: status && status.context_compact_at,
        context_compacted: !!(status && status.context_compacted),
      });
      const message = (status && (status.message || status.detail)) || "(no reply text)";
      const detail = (status && status.detail) || "";
      lastReplyKey = message + "\0" + detail;
      lastStatusMessage = message;
      const phase = (status && status.phase) || "";
      if (phase) {
        applyPhase(phase, message, detail);
      }
      // Settled ask: do not keep Thinking from a stale phase-token status
      // (soft-finalize / reject / multi-tool completion).
      if (isThinking(message) || isPhaseToken(message)) {
        endTurnUi(status, false);
        return;
      }
      if (phase === "speaking") {
        setLive("Speaking…", false);
      } else if (phase === "awaiting_approve") {
        setLive("Waiting for approve…", false);
      } else {
        setLive("", false);
      }
      pushAssistant(message, detail, false);
      endTurnUi(status, false);
    })
    .catch((error) => {
      const line = errorText(error, "ask failed");
      setLive(line, true);
      pushAssistant(line, "", true);
      endTurnUi(null, true);
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

if (vaultUnlockBtn) {
  vaultUnlockBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    const passphrase = vaultPass ? vaultPass.value : "";
    invoke("ui_vault_unlock", { passphrase })
      .then(async () => {
        vaultUnlocked = true;
        setVaultGate(false, "passphrase");
        if (vaultPass) {
          vaultPass.value = "";
        }
        await loadChatForActiveProfile();
        if (state === "awake") {
          seedSessionFromHud();
        }
      })
      .catch((error) => {
        showVaultError(errorText(error, "unlock failed"));
      });
  });
}
if (vaultSetBtn) {
  vaultSetBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    const passphrase = vaultPass ? vaultPass.value : "";
    invoke("ui_vault_set_passphrase", { passphrase, keyringWrap: true })
      .then(async () => {
        vaultUnlocked = true;
        setVaultGate(false, "passphrase");
        if (vaultPass) {
          vaultPass.value = "";
        }
        await loadChatForActiveProfile();
        if (state === "awake") {
          seedSessionFromHud();
        }
      })
      .catch((error) => {
        showVaultError(errorText(error, "could not set passphrase"));
      });
  });
}
if (vaultSkipBtn) {
  vaultSkipBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    invoke("ui_vault_skip_plaintext")
      .then(async () => {
        vaultUnlocked = true;
        setVaultGate(false, "plaintext");
        await loadChatForActiveProfile();
        if (state === "awake") {
          seedSessionFromHud();
        }
      })
      .catch((error) => {
        showVaultError(errorText(error, "could not skip"));
      });
  });
}
if (vaultPass) {
  vaultPass.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      if (vaultUnlockBtn && !vaultUnlockBtn.hidden) {
        vaultUnlockBtn.click();
      } else if (vaultSetBtn && !vaultSetBtn.hidden) {
        vaultSetBtn.click();
      }
    }
  });
}

if (selectToggle) {
  selectToggle.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    setSelecting(!selecting);
    markActivity();
  });
}
if (deleteSelectedBtn) {
  deleteSelectedBtn.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    void deleteSelectedTurns();
  });
}

capsule.dataset.idleMs = String(configuredIdleMs);
capsule.dataset.profile = profileName;
updateHint();
void bootstrapVault().finally(() => {
  refresh();
  void refreshProfileName(true);
});

let lastChatReloadMs = 0;
async function maybeReloadChatFromDisk() {
  if (!vaultUnlocked || !chatPersistReady) return;
  const now = Date.now();
  if (now - lastChatReloadMs < 2500) return;
  lastChatReloadMs = now;
  try {
    const snap = await invoke("hud_chat_snapshot", { profileId: profileId || null });
    const next = (snap && snap.turns) || [];
    if (!next.length) return;
    const localLast = turns.length ? turns[turns.length - 1].ts : 0;
    const diskLast = next.length ? Number(next[next.length - 1].ts) || 0 : 0;
    if (diskLast > localLast || next.length > turns.length) {
      replaceTurns(next);
    }
  } catch (_error) {
    // ignore
  }
}

window.setInterval(refresh, 900);
window.setInterval(() => { void maybeReloadChatFromDisk(); }, 2500);
raf = requestAnimationFrame(tick);
