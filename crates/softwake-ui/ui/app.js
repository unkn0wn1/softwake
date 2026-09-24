const stateEl = document.querySelector("#state");
const captureEl = document.querySelector("#capture");
const soulEl = document.querySelector("#soul");
const reloadEl = document.querySelector("#reload");
const detailEl = document.querySelector("#detail");
const lastToolEl = document.querySelector("#last-tool");
const pendingEl = document.querySelector("#pending");
const errorEl = document.querySelector("#error");
const confirmBtn = document.querySelector("#confirm");
const cancelBtn = document.querySelector("#cancel");

const commands = {
  hibernate: "hibernate",
  wake: "resume",
  sleep: "sleep",
  "reload-soul": "reload_soul",
};

let pendingId = null;

function invoke(command, args) {
  const core = window.__TAURI__ && window.__TAURI__.core;
  if (!core) {
    return Promise.reject(new Error("window bridge is not available"));
  }
  return core.invoke(command, args);
}

function soulLine(status) {
  if (!status.soul) {
    return "soul: unknown";
  }
  return status.soul.ok ? "soul: ok" : "soul: missing";
}

function showPending(status) {
  const pending = status.pending_tool;
  if (!pending) {
    pendingId = null;
    pendingEl.textContent = "pending: none";
    confirmBtn.disabled = true;
    cancelBtn.disabled = true;
    return;
  }
  pendingId = pending.pending_id;
  const args = (pending.args || []).join(" ");
  const tail = args ? ` ${args}` : "";
  pendingEl.textContent = `pending ${pending.pending_id}: ${pending.name}${tail} — ${pending.description}`;
  confirmBtn.disabled = false;
  cancelBtn.disabled = false;
}

function show(status, keepError) {
  stateEl.textContent = status.state;
  captureEl.textContent = status.capture_running ? "capture: running" : "capture: stopped";
  soulEl.textContent = soulLine(status);
  reloadEl.textContent = status.soul_reload_pending
    ? "soul reload: pending — applies on next awake"
    : "soul reload: not pending";
  detailEl.textContent = status.detail || status.message || "";
  lastToolEl.textContent = status.last_tool ? `last tool: ${status.last_tool}` : "last tool: none";
  showPending(status);
  if (!keepError) {
    errorEl.textContent = "";
  }
}

function showError(error) {
  const text = typeof error === "string" ? error : error && error.message ? error.message : "request failed";
  errorEl.textContent = text;
}

async function refresh() {
  try {
    show(await invoke("status"), true);
  } catch (error) {
    stateEl.textContent = "—";
    captureEl.textContent = "";
    soulEl.textContent = "";
    reloadEl.textContent = "";
    detailEl.textContent = "";
    lastToolEl.textContent = "";
    pendingEl.textContent = "";
    pendingId = null;
    confirmBtn.disabled = true;
    cancelBtn.disabled = true;
    showError(error);
  }
}

async function send(command, args) {
  try {
    show(await invoke(command, args));
  } catch (error) {
    showError(error);
    try {
      show(await invoke("status"), true);
      showError(error);
    } catch (statusError) {
      showError(statusError);
    }
  }
}

for (const [id, command] of Object.entries(commands)) {
  document.querySelector(`#${id}`).addEventListener("click", () => {
    send(command);
  });
}

confirmBtn.addEventListener("click", () => {
  if (!pendingId) {
    return;
  }
  send("confirm_tool", { pendingId });
});

cancelBtn.addEventListener("click", () => {
  if (!pendingId) {
    return;
  }
  send("cancel_tool", { pendingId });
});

refresh();
setInterval(refresh, 1000);
