const stateEl = document.querySelector("#state");
const captureEl = document.querySelector("#capture");
const reloadEl = document.querySelector("#reload");
const detailEl = document.querySelector("#detail");
const errorEl = document.querySelector("#error");

const commands = {
  hibernate: "hibernate",
  wake: "resume",
  sleep: "sleep",
  "reload-soul": "reload_soul",
};

function invoke(command) {
  const core = window.__TAURI__ && window.__TAURI__.core;
  if (!core) {
    return Promise.reject(new Error("window bridge is not available"));
  }
  return core.invoke(command);
}

function show(status, keepError) {
  stateEl.textContent = status.state;
  captureEl.textContent = status.capture_running ? "capture: running" : "capture: stopped";
  reloadEl.textContent = status.soul_reload_pending
    ? "soul reload: pending — applies on next awake"
    : "soul reload: not pending";
  detailEl.textContent = status.detail || status.message || "";
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
    reloadEl.textContent = "";
    detailEl.textContent = "";
    showError(error);
  }
}

async function send(command) {
  try {
    show(await invoke(command));
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

refresh();
setInterval(refresh, 1000);
