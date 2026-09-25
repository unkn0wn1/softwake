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
const navStatus = document.querySelector("#nav-status");

const panes = ["general", "providers", "email", "status"];

const providerSelect = document.querySelector("#provider-select");
const keyPanel = document.querySelector("#key-panel");
const oauthPanel = document.querySelector("#oauth-panel");
const apiKeyInput = document.querySelector("#api-key");
const baseUrlField = document.querySelector("#base-url-field");
const baseUrlInput = document.querySelector("#base-url");
const saveBaseUrlBtn = document.querySelector("#save-base-url");
const saveKeyBtn = document.querySelector("#save-key");
const clearKeyBtn = document.querySelector("#clear-key");
const oauthStatus = document.querySelector("#oauth-status");
const oauthCode = document.querySelector("#oauth-code");
const oauthStartBtn = document.querySelector("#oauth-start");
const oauthPollBtn = document.querySelector("#oauth-poll");
const oauthSignOutBtn = document.querySelector("#oauth-sign-out");
const providerTestBtn = document.querySelector("#provider-test");
const testStatus = document.querySelector("#test-status");
const modelSelect = document.querySelector("#model-select");
const providerError = document.querySelector("#provider-error");
const plaintextWarning = document.querySelector("#plaintext-warning");

const commands = {
  hibernate: "hibernate",
  wake: "resume",
  sleep: "sleep",
  "reload-soul": "reload_soul",
};

let pendingId = null;
let providerSnap = null;
let oauthPollTimer = null;

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
  if (status.soul.ok) {
    return "soul: ok";
  }
  if (status.soul.reason) {
    return "soul: missing — " + status.soul.reason;
  }
  return "soul: missing";
}

function showPending(status) {
  const pending = status.pending_tool;
  if (!pending) {
    pendingId = null;
    pendingEl.textContent = "pending: none";
    confirmBtn.disabled = true;
    cancelBtn.disabled = true;
    navStatus.classList.remove("has-pending");
    return;
  }
  navStatus.classList.add("has-pending");
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

function showProviderError(error) {
  const text = typeof error === "string" ? error : error && error.message ? error.message : "request failed";
  providerError.textContent = text;
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
    navStatus.classList.remove("has-pending");
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

function selectedRow() {
  if (!providerSnap) {
    return null;
  }
  return providerSnap.providers.find((row) => row.id === providerSnap.selected_provider) || null;
}

function renderProviders(snap) {
  providerSnap = snap;
  providerError.textContent = "";
  plaintextWarning.textContent = snap.plaintext_warning || "";

  const previous = providerSelect.value;
  providerSelect.innerHTML = "";
  for (const row of snap.providers) {
    const option = document.createElement("option");
    option.value = row.id;
    option.textContent = row.label;
    providerSelect.appendChild(option);
  }
  providerSelect.value = snap.selected_provider || previous;

  const row = selectedRow();
  const isOauth = row && row.credential === "xai-oauth";
  keyPanel.classList.toggle("hidden", !!isOauth);
  oauthPanel.classList.toggle("hidden", !isOauth);

  if (isOauth) {
    if (snap.oauth_pending) {
      oauthStatus.textContent = "Sign-in in progress.";
      oauthCode.textContent =
        "Code " + snap.oauth_pending.user_code + " — open " + snap.oauth_pending.verification_url;
      oauthPollBtn.disabled = false;
      scheduleOauthPoll(snap.oauth_pending.interval_sec);
    } else if (snap.has_xai_oauth) {
      clearOauthPoll();
      oauthStatus.textContent = "Signed in.";
      oauthCode.textContent = "";
      oauthPollBtn.disabled = true;
    } else {
      clearOauthPoll();
      oauthStatus.textContent = "Not signed in.";
      oauthCode.textContent = "";
      oauthPollBtn.disabled = true;
    }
  } else {
    clearOauthPoll();
    apiKeyInput.value = "";
    const needsBase = row && row.credential === "openai-compatible-key";
    baseUrlField.classList.toggle("hidden", !needsBase);
    saveBaseUrlBtn.classList.toggle("hidden", !needsBase);
    if (needsBase) {
      baseUrlInput.value = snap.openai_compatible_base_url || "";
    }
    if (row && row.credential === "xai-key") {
      clearKeyBtn.textContent = snap.has_xai_key ? "Clear saved" : "Clear saved";
    }
    if (row && row.credential === "openai-key") {
      clearKeyBtn.textContent = snap.has_openai_key ? "Clear saved" : "Clear saved";
    }
    if (row && row.credential === "openrouter-key") {
      clearKeyBtn.textContent = snap.has_openrouter_key ? "Clear saved" : "Clear saved";
    }
    if (row && row.credential === "openai-compatible-key") {
      clearKeyBtn.textContent = snap.has_openai_compatible_key
        ? "Clear saved"
        : "Clear saved";
    }
  }

  if (snap.last_test_ok === true) {
    testStatus.textContent = "Test: passed — " + (snap.last_test_message || "");
  } else if (snap.last_test_ok === false) {
    testStatus.textContent = "Test: failed — " + (snap.last_test_message || "");
  } else {
    testStatus.textContent = "Test: not run";
  }

  const models = snap.models || [];
  modelSelect.innerHTML = "";
  if (models.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Test to load models";
    modelSelect.appendChild(option);
    modelSelect.disabled = true;
  } else {
    for (const id of models) {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = id;
      modelSelect.appendChild(option);
    }
    modelSelect.disabled = false;
    modelSelect.value = models.includes(snap.selected_model) ? snap.selected_model : models[0];
  }
}

function clearOauthPoll() {
  if (oauthPollTimer) {
    clearTimeout(oauthPollTimer);
    oauthPollTimer = null;
  }
}

function scheduleOauthPoll(intervalSec) {
  clearOauthPoll();
  const ms = Math.max(1, Number(intervalSec) || 5) * 1000;
  oauthPollTimer = setTimeout(() => {
    providerAction("provider_oauth_poll");
  }, ms);
}

async function refreshProviders() {
  try {
    renderProviders(await invoke("provider_snapshot"));
  } catch (error) {
    showProviderError(error);
  }
}

async function providerAction(command, args) {
  try {
    renderProviders(await invoke(command, args));
  } catch (error) {
    showProviderError(error);
    try {
      renderProviders(await invoke("provider_snapshot"));
      showProviderError(error);
    } catch (snapError) {
      showProviderError(snapError);
    }
  }
}

providerSelect.addEventListener("change", () => {
  providerAction("provider_select", { providerId: providerSelect.value });
});

saveKeyBtn.addEventListener("click", () => {
  providerAction("provider_set_key", {
    providerId: providerSelect.value,
    key: apiKeyInput.value,
  }).then(() => {
    apiKeyInput.value = "";
  });
});

saveBaseUrlBtn.addEventListener("click", () => {
  providerAction("provider_set_base_url", {
    baseUrl: baseUrlInput.value,
  });
});

clearKeyBtn.addEventListener("click", () => {
  providerAction("provider_clear_cred", { providerId: providerSelect.value });
});

oauthStartBtn.addEventListener("click", () => {
  providerAction("provider_oauth_start");
});

oauthPollBtn.addEventListener("click", () => {
  providerAction("provider_oauth_poll");
});

oauthSignOutBtn.addEventListener("click", () => {
  providerAction("provider_oauth_sign_out");
});

providerTestBtn.addEventListener("click", () => {
  providerAction("provider_test");
});

modelSelect.addEventListener("change", () => {
  if (!modelSelect.value) {
    return;
  }
  providerAction("provider_set_model", { modelId: modelSelect.value });
});

function showPane(name) {
  for (const pane of panes) {
    const section = document.querySelector(`#pane-${pane}`);
    const nav = document.querySelector(`#nav-${pane}`);
    const on = pane === name;
    section.classList.toggle("hidden", !on);
    section.hidden = !on;
    if (on) {
      nav.setAttribute("aria-current", "page");
    } else {
      nav.removeAttribute("aria-current");
    }
  }
}

for (const pane of panes) {
  document.querySelector(`#nav-${pane}`).addEventListener("click", () => {
    showPane(pane);
  });
}

showPane("status");
refresh();
setInterval(refresh, 1000);
refreshProviders();
