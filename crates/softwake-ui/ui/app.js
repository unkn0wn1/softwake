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

const panes = ["general", "profiles", "providers", "tools", "email", "status"];

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
const oauthLink = document.querySelector("#oauth-link");
const oauthUrlText = document.querySelector("#oauth-url-text");
const oauthOpen = document.querySelector("#oauth-open");
const oauthBrowserNote = document.querySelector("#oauth-browser-note");
const oauthStartBtn = document.querySelector("#oauth-start");
const oauthPollBtn = document.querySelector("#oauth-poll");
const oauthSignOutBtn = document.querySelector("#oauth-sign-out");
const providerTestBtn = document.querySelector("#provider-test");
const testStatus = document.querySelector("#test-status");
const modelSelect = document.querySelector("#model-select");
const voiceModelSelect = document.querySelector("#voice-model-select");
const ttsVoiceSelect = document.querySelector("#tts-voice-select");
const ttsNote = document.querySelector("#tts-note");
const providerError = document.querySelector("#provider-error");
const emailLiveEnabled = document.querySelector("#email-live-enabled");
const emailSmtpHost = document.querySelector("#email-smtp-host");
const emailSmtpPort = document.querySelector("#email-smtp-port");
const emailUsername = document.querySelector("#email-username");
const emailFrom = document.querySelector("#email-from");
const emailMode = document.querySelector("#email-mode");
const emailPassword = document.querySelector("#email-password");
const emailPasswordStatus = document.querySelector("#email-password-status");
const emailTestStatus = document.querySelector("#email-test-status");
const emailStorage = document.querySelector("#email-storage");
const emailError = document.querySelector("#email-error");
const emailSaveBtn = document.querySelector("#email-save");
const toolsShellEnabled = document.querySelector("#tools-shell-enabled");
const toolsConfirmPolicy = document.querySelector("#tools-confirm-policy");
const toolsSaveBtn = document.querySelector("#tools-save");
const toolsStatus = document.querySelector("#tools-status");
const toolsError = document.querySelector("#tools-error");
const emailClearPasswordBtn = document.querySelector("#email-clear-password");
const emailTestBtn = document.querySelector("#email-test");
const plaintextWarning = document.querySelector("#plaintext-warning");
const usePlaintextBtn = document.querySelector("#use-plaintext-file");

const packFiles = ["soul", "user", "rules", "glossary"];
const packDirEl = document.querySelector("#pack-dir");
const packValidityEl = document.querySelector("#pack-validity");
const packStatusEl = document.querySelector("#pack-status");
const packErrorEl = document.querySelector("#pack-error");
const packEditors = {
  soul: document.querySelector("#pack-soul"),
  user: document.querySelector("#pack-user"),
  rules: document.querySelector("#pack-rules"),
  glossary: document.querySelector("#pack-glossary"),
};

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
  plaintextWarning.textContent = snap.storage_message || "";
  const showPlaintextOptIn = snap.storage_backend === "unavailable";
  usePlaintextBtn.classList.toggle("hidden", !showPlaintextOptIn);
  if (showPlaintextOptIn) {
    usePlaintextBtn.removeAttribute("hidden");
  } else {
    usePlaintextBtn.setAttribute("hidden", "");
  }

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
      const pending = snap.oauth_pending;
      oauthStatus.textContent = "Sign-in in progress.";
      oauthCode.textContent = "Code " + pending.user_code;
      if (pending.link_openable) {
        oauthLink.setAttribute("href", pending.verification_url);
        oauthLink.textContent = pending.verification_url;
        oauthUrlText.textContent = "";
        oauthOpen.setAttribute("href", pending.verification_url);
        oauthOpen.textContent = "Open link";
        oauthOpen.classList.remove("hidden");
      } else {
        oauthLink.removeAttribute("href");
        oauthLink.textContent = "";
        oauthOpen.removeAttribute("href");
        oauthUrlText.textContent = pending.verification_url;
        oauthOpen.classList.add("hidden");
      }
      oauthBrowserNote.textContent = pending.browser_note || "";
      oauthPollBtn.disabled = false;
      scheduleOauthPoll(pending.interval_sec);
    } else if (snap.has_xai_oauth) {
      clearOauthPoll();
      oauthStatus.textContent = "Signed in.";
      clearOauthLink(snap);
      oauthPollBtn.disabled = true;
    } else {
      clearOauthPoll();
      oauthStatus.textContent = "Not signed in.";
      clearOauthLink(snap);
      oauthPollBtn.disabled = true;
    }
  } else {
    clearOauthPoll();
    clearOauthLink(snap);
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

  const voiceModels = snap.voice_models || [];
  voiceModelSelect.innerHTML = "";
  if (voiceModels.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "Test to load models";
    voiceModelSelect.appendChild(option);
    voiceModelSelect.disabled = true;
  } else {
    for (const id of voiceModels) {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = id;
      voiceModelSelect.appendChild(option);
    }
    voiceModelSelect.disabled = false;
    voiceModelSelect.value = voiceModels.includes(snap.selected_voice_model)
      ? snap.selected_voice_model
      : voiceModels[0];
  }

  const ttsVoices = snap.tts_voices || [];
  ttsVoiceSelect.innerHTML = "";
  if (!snap.tts_available || ttsVoices.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "xAI only";
    ttsVoiceSelect.appendChild(option);
    ttsVoiceSelect.disabled = true;
    ttsNote.textContent =
      "TTS voice is for xAI. Eve speaks when the selected provider is xAI sign-in or an xAI API key.";
  } else {
    const fallback = document.createElement("option");
    fallback.value = "";
    fallback.textContent = "eve (default)";
    ttsVoiceSelect.appendChild(fallback);
    for (const id of ttsVoices) {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = id;
      ttsVoiceSelect.appendChild(option);
    }
    ttsVoiceSelect.disabled = false;
    const selected = snap.selected_tts_voice || "";
    ttsVoiceSelect.value = ttsVoices.includes(selected) ? selected : "";
    ttsNote.textContent =
      "Ask replies are spoken with this xAI voice. Empty uses Eve.";
  }
}

function clearOauthLink(snap) {
  if (!snap || !snap.oauth_pending) {
    oauthCode.textContent = "";
  }
  oauthLink.removeAttribute("href");
  oauthLink.textContent = "";
  oauthUrlText.textContent = "";
  oauthOpen.removeAttribute("href");
  oauthOpen.textContent = "";
  oauthOpen.classList.add("hidden");
  oauthBrowserNote.textContent = "";
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

usePlaintextBtn.addEventListener("click", () => {
  providerAction("provider_opt_in_plaintext");
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

voiceModelSelect.addEventListener("change", () => {
  if (!voiceModelSelect.value) {
    return;
  }
  providerAction("provider_set_voice_model", { modelId: voiceModelSelect.value });
});

ttsVoiceSelect.addEventListener("change", () => {
  providerAction("provider_set_tts_voice", { voiceId: ttsVoiceSelect.value });
});

let profilesLoaded = false;
let profilesSnap = null;
let selectedProfileId = "";

const profilesListEl = document.querySelector("#profiles-list");
const profilesSubnavEl = document.querySelector("#profiles-subnav");
const profilesConfigDirEl = document.querySelector("#profiles-config-dir");
const profileNameInput = document.querySelector("#profile-name");

function setPackEditable(on) {
  document.querySelector("#pack-save").disabled = !on;
  for (const file of packFiles) {
    packEditors[file].disabled = !on;
  }
}

function errorText(error) {
  return typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function showPackTab(name) {
  for (const file of packFiles) {
    const editor = packEditors[file];
    const tab = document.querySelector(`#pack-tab-${file}`);
    const on = file === name;
    editor.classList.toggle("hidden", !on);
    editor.hidden = !on;
    tab.setAttribute("aria-selected", on ? "true" : "false");
  }
}

function applyPackSnapshot(snap, statusText) {
  packDirEl.textContent = "Directory: " + (snap.dir || "");
  for (const file of packFiles) {
    packEditors[file].value = snap[file] || "";
  }
  if (snap.ok) {
    packValidityEl.textContent = "Pack: ok";
    packValidityEl.classList.remove("error");
  } else {
    const reason = snap.reason ? " — " + snap.reason : "";
    packValidityEl.textContent = "Pack: invalid" + reason;
    packValidityEl.classList.add("error");
  }
  packErrorEl.textContent = "";
  packStatusEl.textContent = statusText || "";
  setPackEditable(true);
}

function setProfilesSubnavVisible(on) {
  if (!profilesSubnavEl) return;
  profilesSubnavEl.classList.toggle("hidden", !on);
  profilesSubnavEl.hidden = !on;
}

function renderProfilesList(snap) {
  profilesListEl.innerHTML = "";
  for (const row of snap.profiles || []) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-chip";
    btn.setAttribute("role", "option");
    btn.setAttribute("aria-selected", row.id === snap.selected_id ? "true" : "false");
    btn.dataset.profileId = row.id;
    const label = (row.name && String(row.name).trim()) || row.id;
    btn.title = row.id + (row.pack_ok ? "" : " (invalid pack)");
    if (row.active) {
      const dot = document.createElement("span");
      dot.className = "active-dot";
      dot.setAttribute("aria-label", "Active profile");
      btn.appendChild(dot);
    }
    const title = document.createElement("span");
    title.className = "profile-chip-label";
    title.textContent = label;
    btn.appendChild(title);
    if (!row.pack_ok) {
      const warn = document.createElement("span");
      warn.className = "profile-chip-warn";
      warn.textContent = "!";
      warn.title = "Invalid pack";
      btn.appendChild(warn);
    }
    btn.addEventListener("click", () => {
      loadProfiles(row.id);
    });
    li.appendChild(btn);
    profilesListEl.appendChild(li);
  }
}

function applyProfilesSnapshot(snap, statusText) {
  profilesSnap = snap;
  selectedProfileId = snap.selected_id || "";
  profilesConfigDirEl.textContent = "Config: " + (snap.config_dir || "");
  profileNameInput.value = snap.selected_name || "";
  renderProfilesList(snap);
  setProfilesSubnavVisible(true);
  applyPackSnapshot(snap.pack || {}, statusText || "");
  profilesLoaded = true;
}

async function loadProfiles(selectedId, statusText) {
  try {
    const args = {};
    if (selectedId) args.selectedId = selectedId;
    applyProfilesSnapshot(await invoke("profiles_snapshot", args), statusText || "");
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
}

async function createProfile() {
  packErrorEl.textContent = "";
  try {
    applyProfilesSnapshot(
      await invoke("profile_create", { name: "" }),
      "Created blank profile — set an agent name when you save."
    );
    profileNameInput.focus();
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
}

async function saveProfileName() {
  if (!selectedProfileId) return;
  const name = (profileNameInput.value || "").trim();
  if (!name) {
    packErrorEl.textContent = "Agent name cannot be empty.";
    return;
  }
  packErrorEl.textContent = "";
  try {
    applyProfilesSnapshot(
      await invoke("profile_rename", { id: selectedProfileId, name }),
      "Saved agent name."
    );
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
}

async function setActiveProfile() {
  if (!selectedProfileId) return;
  packErrorEl.textContent = "";
  try {
    const snap = await invoke("profile_set_active", { id: selectedProfileId });
    applyProfilesSnapshot(snap, "Active profile updated.");
    if (snap.pack && snap.pack.ok) {
      try {
        await invoke("reload_soul");
        packStatusEl.textContent = "Active profile set; reload recorded — applies on next awake.";
        refresh();
      } catch (error) {
        packErrorEl.textContent = "Active set, but reload soul failed: " + errorText(error);
      }
    }
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
}

async function savePack() {
  if (document.querySelector("#pack-save").disabled) {
    return;
  }
  packErrorEl.textContent = "";
  try {
    const snap = await invoke("pack_save", {
      profileId: selectedProfileId || null,
      soul: packEditors.soul.value,
      user: packEditors.user.value,
      rules: packEditors.rules.value,
      glossary: packEditors.glossary.value,
    });
    const profiles = await invoke("profiles_snapshot", {
      selectedId: selectedProfileId || null,
    });
    profiles.pack = snap;
    if (snap.ok && profiles.active_id === selectedProfileId) {
      try {
        await invoke("reload_soul");
        applyProfilesSnapshot(profiles, "Saved and reload recorded — applies on next awake.");
        refresh();
      } catch (error) {
        applyProfilesSnapshot(profiles, "");
        packErrorEl.textContent = "Saved, but reload soul failed: " + errorText(error);
      }
      return;
    }
    applyProfilesSnapshot(
      profiles,
      snap.ok ? "Saved. Reload soul was not called (profile is not active)." : "Saved. Reload soul was not called."
    );
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
}

async function reloadSoulFromProfiles() {
  packErrorEl.textContent = "";
  try {
    await invoke("reload_soul");
    packStatusEl.textContent = "Reload soul recorded — applies on next awake.";
    refresh();
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
}

for (const file of packFiles) {
  document.querySelector(`#pack-tab-${file}`).addEventListener("click", () => {
    showPackTab(file);
  });
}

document.querySelector("#pack-save").addEventListener("click", () => {
  savePack();
});

document.querySelector("#pack-reload-disk").addEventListener("click", () => {
  loadProfiles(selectedProfileId, "Reloaded from disk.");
});

document.querySelector("#pack-reload-soul").addEventListener("click", () => {
  reloadSoulFromProfiles();
});

document.querySelector("#profile-create").addEventListener("click", () => {
  createProfile();
});

document.querySelector("#profile-save-name").addEventListener("click", () => {
  saveProfileName();
});

document.querySelector("#profile-set-active").addEventListener("click", () => {
  setActiveProfile();
});


let emailSnap = null;

function showEmailError(error) {
  const text = error && error.message ? error.message : String(error || "");
  emailError.textContent = text;
}

function renderEmail(snap) {
  emailSnap = snap;
  emailError.textContent = "";
  emailLiveEnabled.checked = !!snap.live_enabled;
  emailSmtpHost.value = snap.smtp_host || "";
  emailSmtpPort.value = String(snap.smtp_port || 587);
  emailUsername.value = snap.username || "";
  emailFrom.value = snap.from_address || "";
  emailMode.value = snap.mode || "draft_only";
  emailPassword.value = "";
  emailPasswordStatus.textContent = snap.has_password
    ? "Password: saved in the secret bag"
    : "Password: not saved";
  if (snap.last_test_ok === true) {
    emailTestStatus.textContent = "Test: ok — " + (snap.last_test_message || "");
  } else if (snap.last_test_ok === false) {
    emailTestStatus.textContent = "Test: failed — " + (snap.last_test_message || "");
  } else {
    emailTestStatus.textContent = "Test: not run";
  }
  emailStorage.textContent = snap.storage_message || "";
}

async function refreshEmail() {
  try {
    renderEmail(await invoke("email_snapshot"));
  } catch (error) {
    showEmailError(error);
  }
}

async function emailAction(command, args) {
  try {
    renderEmail(await invoke(command, args));
  } catch (error) {
    showEmailError(error);
    try {
      renderEmail(await invoke("email_snapshot"));
      showEmailError(error);
    } catch (snapError) {
      showEmailError(snapError);
    }
  }
}

emailSaveBtn.addEventListener("click", () => {
  const port = Number(emailSmtpPort.value);
  const password = emailPassword.value;
  emailAction("email_save", {
    liveEnabled: emailLiveEnabled.checked,
    smtpHost: emailSmtpHost.value,
    smtpPort: Number.isFinite(port) && port > 0 ? port : 587,
    username: emailUsername.value,
    fromAddress: emailFrom.value,
    mode: emailMode.value,
    password: password ? password : null,
  });
});

emailClearPasswordBtn.addEventListener("click", () => {
  emailAction("email_clear_password");
});

emailTestBtn.addEventListener("click", () => {
  emailAction("email_test");
});


function showToolsError(error) {
  toolsError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function renderTools(snap) {
  toolsShellEnabled.checked = !!snap.shell_enabled;
  toolsConfirmPolicy.value = snap.confirm_policy || "always";
  toolsStatus.textContent = snap.shell_enabled
    ? "Shell enabled — confirm policy: " + (snap.confirm_policy || "always")
    : "Shell off (default). Softwake cannot run shell until you enable it.";
  toolsError.textContent = "";
}

async function refreshTools() {
  try {
    renderTools(await invoke("tools_snapshot"));
  } catch (error) {
    showToolsError(error);
  }
}

toolsSaveBtn.addEventListener("click", async () => {
  try {
    renderTools(
      await invoke("tools_save", {
        shellEnabled: toolsShellEnabled.checked,
        confirmPolicy: toolsConfirmPolicy.value,
      })
    );
    toolsStatus.textContent = "Tools Settings saved.";
  } catch (error) {
    showToolsError(error);
    try {
      renderTools(await invoke("tools_snapshot"));
      showToolsError(error);
    } catch (statusError) {
      showToolsError(statusError);
    }
  }
});

const uiTextSizeSelect = document.querySelector("#ui-text-size");
const uiPrefsStatus = document.querySelector("#ui-prefs-status");
const uiPrefsError = document.querySelector("#ui-prefs-error");

const TEXT_SIZES = ["xx-small", "x-small", "small", "medium", "large"];

function applyTextSize(size) {
  const value = TEXT_SIZES.includes(size) ? size : "x-small";
  document.documentElement.setAttribute("data-text-size", value);
  if (uiTextSizeSelect) {
    uiTextSizeSelect.value = value;
  }
}

function showUiPrefsError(error) {
  if (!uiPrefsError) return;
  uiPrefsError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

async function refreshUiPrefs() {
  if (!uiTextSizeSelect) return;
  try {
    if (uiPrefsError) uiPrefsError.textContent = "";
    const snap = await invoke("ui_prefs_snapshot");
    applyTextSize(snap.text_size || "x-small");
    if (uiPrefsStatus) {
      uiPrefsStatus.textContent = "UI text size: " + (snap.text_size || "x-small");
    }
  } catch (error) {
    applyTextSize("x-small");
    showUiPrefsError(error);
  }
}

if (uiTextSizeSelect) {
  uiTextSizeSelect.addEventListener("change", async () => {
    const textSize = uiTextSizeSelect.value;
    applyTextSize(textSize);
    try {
      if (uiPrefsError) uiPrefsError.textContent = "";
      const snap = await invoke("ui_prefs_set_text_size", { textSize });
      applyTextSize(snap.text_size || textSize);
      if (uiPrefsStatus) {
        uiPrefsStatus.textContent = "Saved UI text size: " + (snap.text_size || textSize);
      }
    } catch (error) {
      showUiPrefsError(error);
      refreshUiPrefs();
    }
  });
}

function showPane(name) {
  for (const pane of panes) {
    const section = document.querySelector(`#pane-${pane}`);
    const nav = document.querySelector(`#nav-${pane}`);
    const on = pane === name;
    section.classList.toggle("hidden", !on);
    section.hidden = !on;
    // Belt-and-suspenders: pane-fill used to override .hidden via display:flex.
    section.style.display = on ? "" : "none";
    if (on) {
      nav.setAttribute("aria-current", "page");
    } else {
      nav.removeAttribute("aria-current");
    }
  }
  setProfilesSubnavVisible(name === "profiles");
  if (name === "profiles") {
    if (!profilesLoaded) {
      loadProfiles(selectedProfileId);
    } else {
      setProfilesSubnavVisible(true);
    }
  }
  if (name === "email") {
    refreshEmail();
  }
  if (name === "tools") {
    refreshTools();
  }
}

for (const pane of panes) {
  document.querySelector(`#nav-${pane}`).addEventListener("click", () => {
    showPane(pane);
  });
}

applyTextSize("x-small");
refreshUiPrefs();
showPane("status");
refresh();
setInterval(refresh, 1000);
refreshProviders();
refreshEmail();
