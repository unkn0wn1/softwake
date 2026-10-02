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

const panes = ["general", "global", "profiles", "providers", "tools", "rooms", "timers", "skills", "messengers", "mcp", "remote-agent", "email", "status"];

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
const ttsSpeedSelect = document.querySelector("#tts-speed-select");
const ttsSpeedField = document.querySelector("#tts-speed-field");
const ttsVoiceField = document.querySelector("#tts-voice-field");
const voiceAgentS2sPanel = document.querySelector("#voice-agent-s2s-panel");
const contextLimitInput = document.querySelector("#context-limit");
const compactAtInput = document.querySelector("#compact-at");
const saveContextBtn = document.querySelector("#save-context");
const contextLineEl = document.querySelector("#context-line");
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
const toolsList = document.querySelector("#tools-list");
const toolsConfirmPolicy = document.querySelector("#tools-confirm-policy");
const toolsPolicyNote = document.querySelector("#tools-policy-note");
const toolsSaveBtn = document.querySelector("#tools-save");
const toolsStatus = document.querySelector("#tools-status");
const toolsError = document.querySelector("#tools-error");
const emailClearPasswordBtn = document.querySelector("#email-clear-password");
const emailTestBtn = document.querySelector("#email-test");
const emailOauthPending = document.querySelector("#email-oauth-pending");
const emailOauthUrlLine = document.querySelector("#email-oauth-url-line");
const emailOauthLink = document.querySelector("#email-oauth-link");
const emailOauthCancelBtn = document.querySelector("#email-oauth-cancel");
const emailOauthError = document.querySelector("#email-oauth-error");
const emailGoogleAccounts = document.querySelector("#email-google-accounts");
const emailMicrosoftAccounts = document.querySelector("#email-microsoft-accounts");
const emailGoogleConnectBtn = document.querySelector("#email-google-connect");
const emailMicrosoftConnectBtn = document.querySelector("#email-microsoft-connect");
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
  resume: "resume",
  wake: "wake",
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

function statusBody(status) {
  const message = (status && status.message) || "";
  const detail = (status && status.detail) || "";
  if (message && detail && detail !== message) {
    return message + "\n" + detail;
  }
  return message || detail;
}

function show(status, keepError) {
  stateEl.textContent = status.state;
  captureEl.textContent = status.capture_running ? "capture: running" : "capture: stopped";
  soulEl.textContent = soulLine(status);
  reloadEl.textContent = status.soul_reload_pending
    ? "soul reload: pending — applies on next awake"
    : "soul reload: not pending";
  detailEl.textContent = statusBody(status);
  if (contextLineEl) {
    if (status.state === "awake" && status.context_limit != null && status.context_used != null) {
      const pct = status.context_limit
        ? Math.min(100, Math.round((100 * status.context_used) / status.context_limit))
        : 0;
      const threshold =
        status.context_compact_at != null && status.context_compact_at > 0
          ? status.context_compact_at
          : 80;
      let line = `context ~${status.context_used} / ${status.context_limit} (${pct}%) · auto-compact at ${threshold}%`;
      if (status.context_compacted) line += " — compacted";
      contextLineEl.textContent = line;
    } else {
      contextLineEl.textContent = "";
    }
  }
  lastToolEl.textContent = status.last_tool ? `last tool: ${status.last_tool}` : "last tool: none";
  showPending(status);
  if (voiceTestBox && !voiceTestEditing) {
    voiceTestBox.checked = !!status.voice_test;
  }
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
  if (contextLimitInput) {
    contextLimitInput.value =
      snap.context_limit_tokens && snap.context_limit_tokens > 0
        ? String(snap.context_limit_tokens)
        : "";
  }
  if (compactAtInput) {
    compactAtInput.value = String(snap.compact_at_percent || 80);
  }
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
    {
      const none = document.createElement("option");
      none.value = "";
      none.textContent = "None";
      modelSelect.insertBefore(none, modelSelect.firstChild);
    }
    // Distinguish unset vs first catalog entry — never auto-pick models[0] on load.
    const selected = (snap.selected_model || "").trim();
    modelSelect.value = models.includes(selected) ? selected : "";
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
    const none = document.createElement("option");
    none.value = "";
    none.textContent = "None";
    voiceModelSelect.appendChild(none);
    for (const id of voiceModels) {
      const option = document.createElement("option");
      option.value = id;
      option.textContent = id;
      voiceModelSelect.appendChild(option);
    }
    voiceModelSelect.disabled = false;
    const selectedVoice = (snap.selected_voice_model || "").trim();
    voiceModelSelect.value = voiceModels.includes(selectedVoice) ? selectedVoice : "";
  }

  const ttsVoices = snap.tts_voices || [];
  const xaiVoice = !!snap.tts_available;
  ttsVoiceSelect.innerHTML = "";
  if (!xaiVoice || ttsVoices.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "xAI only";
    ttsVoiceSelect.appendChild(option);
    ttsVoiceSelect.disabled = true;
    ttsNote.textContent =
      "TTS voice, speech speed, and Voice Agent S2S are xAI only — hidden for other providers.";
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
      "Ask replies use this xAI voice and speech speed. Empty voice uses Eve. Speed outside 0.7–1.5 is clamped.";
  }
  if (ttsVoiceField) {
    ttsVoiceField.classList.toggle("hidden", !xaiVoice);
    ttsVoiceField.hidden = !xaiVoice;
  }
  const speedPresets = snap.tts_speed_presets || [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
  if (ttsSpeedSelect) {
    ttsSpeedSelect.innerHTML = "";
    for (const speed of speedPresets) {
      const option = document.createElement("option");
      option.value = String(speed);
      const label = speed === 1 || speed === 1.0 ? "1× (default)" : speed + "×";
      option.textContent = label;
      ttsSpeedSelect.appendChild(option);
    }
    const selectedSpeed = typeof snap.selected_tts_speed === "number" ? snap.selected_tts_speed : 1;
    const match = speedPresets.find((p) => Math.abs(p - selectedSpeed) < 0.001);
    ttsSpeedSelect.value = match != null ? String(match) : "1";
    ttsSpeedSelect.disabled = !xaiVoice;
  }
  if (ttsSpeedField) {
    ttsSpeedField.classList.toggle("hidden", !xaiVoice);
    ttsSpeedField.hidden = !xaiVoice;
  }
  if (voiceAgentS2sPanel) {
    voiceAgentS2sPanel.classList.toggle("hidden", !xaiVoice);
    voiceAgentS2sPanel.hidden = !xaiVoice;
  }
  if (xaiVoice) {
    refreshVoiceAgentS2s();
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
  const busy =
    command === "provider_test"
      ? "Testing…"
      : command === "provider_set_key" ||
          command === "provider_set_base_url" ||
          command === "provider_set_context_limit" ||
          command === "provider_set_compact_at" ||
          command === "provider_set_model" ||
          command === "provider_set_voice_model" ||
          command === "provider_set_tts_voice" ||
          command === "provider_set_tts_speed" ||
          command === "provider_clear_cred" ||
          command === "provider_opt_in_plaintext"
        ? "Saving…"
        : "";
  if (busy && testStatus) {
    testStatus.textContent = busy;
  }
  try {
    const snap = await invoke(command, args);
    renderProviders(snap);
    if (busy && testStatus && command !== "provider_test") {
      if (command === "provider_set_key") {
        testStatus.textContent = "Saved API key.";
      } else if (command === "provider_set_base_url") {
        testStatus.textContent = "Saved base URL.";
      } else if (command === "provider_set_context_limit" || command === "provider_set_compact_at") {
        testStatus.textContent = "Saved context settings.";
      } else if (command === "provider_set_model") {
        testStatus.textContent = snap.selected_model
          ? "Chat model: " + snap.selected_model
          : "Chat model: None (select after Test).";
      } else if (command === "provider_set_voice_model") {
        testStatus.textContent = snap.selected_voice_model
          ? "Voice model: " + snap.selected_voice_model
          : "Voice model: None.";
      } else if (command === "provider_clear_cred") {
        testStatus.textContent = "Cleared saved credential.";
      } else {
        testStatus.textContent = "Saved.";
      }
    }
  } catch (error) {
    showProviderError(error);
    if (busy && testStatus) {
      const msg = typeof error === "string" ? error : error && error.message ? error.message : "request failed";
      testStatus.textContent = (busy === "Testing…" ? "Test failed: " : "Save failed: ") + msg;
    }
    try {
      renderProviders(await invoke("provider_snapshot"));
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

if (saveContextBtn) {
  saveContextBtn.addEventListener("click", () => {
    const rawLimit = (contextLimitInput && contextLimitInput.value.trim()) || "0";
    const tokens = Math.max(0, Number.parseInt(rawLimit, 10) || 0);
    const rawPct = (compactAtInput && compactAtInput.value.trim()) || "80";
    let percent = Number.parseInt(rawPct, 10);
    if (!Number.isFinite(percent) || percent < 1) percent = 70;
    if (percent > 100) percent = 100;
    providerAction("provider_set_context_limit", { tokens }).then(() =>
      providerAction("provider_set_compact_at", { percent })
    );
  });
}

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
  providerAction("provider_set_model", { modelId: modelSelect.value || "" });
});

voiceModelSelect.addEventListener("change", () => {
  providerAction("provider_set_voice_model", { modelId: voiceModelSelect.value || "" });
});

ttsVoiceSelect.addEventListener("change", () => {
  providerAction("provider_set_tts_voice", { voiceId: ttsVoiceSelect.value });
});
if (ttsSpeedSelect) {
  ttsSpeedSelect.addEventListener("change", () => {
    const speed = Number(ttsSpeedSelect.value);
    providerAction("provider_set_tts_speed", { speed: Number.isFinite(speed) ? speed : 1 });
  });
}

let profilesLoaded = false;
let profilesSnap = null;
let selectedProfileId = "";

const profilesListEl = document.querySelector("#profiles-list");
const profilesSubnavEl = document.querySelector("#profiles-subnav");
const profilesConfigDirEl = document.querySelector("#profiles-config-dir");
const profileNameInput = document.querySelector("#profile-name");

const INHERIT_DOCS = ["user", "rules", "glossary"];
const DOC_SCAFFOLDS = {
  user: "# User (scaffold)\n\n# Suggested: Name, Timezone, Address as, Preferences, Hard nos.\n",
  rules:
    "# Rules (scaffold)\n\n# List constraints that override soul.md personality.\n# Suggested: no destructive shell; confirm before mutating actions; glossary is not permission.\n",
  glossary:
    "# Glossary (scaffold)\n\n# Add alias rows as: name, then an arrow, then an absolute path.\n# Heading-only (no alias rows) is a valid empty map.\n",
};
let ownDocs = { user: "", rules: "", glossary: "" };
let globalPreview = { user: "", rules: "", glossary: "" };
let profileIsMain = false;
let activePackTab = "soul";

function docBlank(text) {
  return !String(text || "").trim();
}

function useGlobalChecked(kind) {
  const box = document.querySelector("#use-global-" + kind);
  return !!(box && box.checked);
}

function docUsesGlobal(kind) {
  return !profileIsMain && useGlobalChecked(kind);
}

function setPackEditable(on) {
  document.querySelector("#pack-save").disabled = !on;
  packEditors.soul.disabled = !on;
  packEditors.soul.readOnly = false;
  for (const kind of INHERIT_DOCS) {
    const editor = packEditors[kind];
    const inherited = on && docUsesGlobal(kind);
    editor.disabled = !on;
    editor.readOnly = inherited;
    editor.classList.toggle("is-readonly", inherited);
  }
  updateScaffoldButton();
}

function updateScaffoldButton() {
  const btn = document.querySelector("#insert-doc-scaffold");
  if (!btn) return;
  const show =
    profilesLoaded && INHERIT_DOCS.includes(activePackTab) && !docUsesGlobal(activePackTab);
  btn.hidden = !show;
}

function paintInheritEditors() {
  for (const kind of INHERIT_DOCS) {
    packEditors[kind].value = docUsesGlobal(kind)
      ? globalPreview[kind] || ""
      : ownDocs[kind] || "";
  }
}

function syncOwnFromEditors() {
  for (const kind of INHERIT_DOCS) {
    if (!docUsesGlobal(kind)) {
      ownDocs[kind] = packEditors[kind].value;
    }
  }
}

function packSaveBodies() {
  syncOwnFromEditors();
  return {
    profileId: selectedProfileId || null,
    soul: packEditors.soul.value,
    user: ownDocs.user,
    rules: ownDocs.rules,
    glossary: ownDocs.glossary,
  };
}

function applyGlobalDocControls(snap) {
  profileIsMain = !!snap.is_main;
  globalPreview = {
    user: snap.global_user || "",
    rules: snap.global_rules || "",
    glossary: snap.global_glossary || "",
  };
  const pack = snap.pack || {};
  ownDocs = {
    user: pack.user || "",
    rules: pack.rules || "",
    glossary: pack.glossary || "",
  };
  const flags = {
    user: profileIsMain || !!snap.use_global_user,
    glossary: profileIsMain || !!snap.use_global_glossary,
    rules: profileIsMain || !!snap.use_global_rules,
  };
  for (const kind of INHERIT_DOCS) {
    const box = document.querySelector("#use-global-" + kind);
    if (!box) continue;
    box.checked = !!flags[kind];
    box.disabled = profileIsMain;
  }
  const note = document.querySelector("#global-doc-note");
  if (note) {
    note.textContent = profileIsMain
      ? "This profile owns the global user, glossary, and rules."
      : "Checked docs show the global file and cannot be edited here. Uncheck to edit this profile’s copy.";
  }
}

async function persistGlobalFlags() {
  if (!selectedProfileId || profileIsMain) return null;
  return invoke("profile_set_global_flags", {
    id: selectedProfileId,
    useGlobalUser: useGlobalChecked("user"),
    useGlobalGlossary: useGlobalChecked("glossary"),
    useGlobalRules: useGlobalChecked("rules"),
  });
}

function errorText(error) {
  return typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function showPackTab(name) {
  activePackTab = name;
  for (const file of packFiles) {
    const editor = packEditors[file];
    const tab = document.querySelector(`#pack-tab-${file}`);
    const on = file === name;
    editor.classList.toggle("hidden", !on);
    editor.hidden = !on;
    tab.setAttribute("aria-selected", on ? "true" : "false");
  }
  updateScaffoldButton();
}

function applyPackSnapshot(snap, statusText) {
  packDirEl.textContent = "Directory: " + (snap.dir || "");
  packEditors.soul.value = snap.soul || "";
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
  paintInheritEditors();
}


/** Toggle a left-nav `.nav-sub` block (Profiles / Timers / Skills / Messengers). */
function setSubnavVisible(el, on) {
  if (!el) return;
  el.classList.toggle("hidden", !on);
  el.hidden = !on;
}

function setProfilesSubnavVisible(on) {
  setSubnavVisible(profilesSubnavEl, on);
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
  applyGlobalDocControls(snap);
  const allowAll = document.querySelector("#profile-allow-all");
  if (allowAll) allowAll.checked = !!snap.allow_all;
  const role = document.querySelector("#profile-role");
  if (role) role.value = snap.role === "coding" ? "coding" : "general";
  applyPackSnapshot(snap.pack || {}, statusText || "");
  profilesLoaded = true;
  updateScaffoldButton();
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
    await persistGlobalFlags();
    const snap = await invoke("pack_save", packSaveBodies());
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

for (const kind of INHERIT_DOCS) {
  const box = document.querySelector("#use-global-" + kind);
  if (!box) continue;
  box.addEventListener("change", async () => {
    if (profileIsMain) {
      box.checked = true;
      return;
    }
    const checked = box.checked;
    let seeded = false;
    if (checked) {
      if (!packEditors[kind].readOnly) {
        ownDocs[kind] = packEditors[kind].value;
      }
    } else if (docBlank(ownDocs[kind])) {
      ownDocs[kind] = DOC_SCAFFOLDS[kind];
      seeded = true;
    }
    setPackEditable(true);
    paintInheritEditors();
    packErrorEl.textContent = "";
    try {
      await persistGlobalFlags();
      if (seeded) {
        await invoke("pack_save", packSaveBodies());
        await loadProfiles(
          selectedProfileId,
          "Seeded a commented " + kind + " scaffold for this profile."
        );
        return;
      }
      packStatusEl.textContent = checked
        ? "Using the global " + kind + " file."
        : "Editing this profile’s " + kind + " file.";
    } catch (error) {
      packErrorEl.textContent = errorText(error);
      loadProfiles(selectedProfileId);
    }
  });
  packEditors[kind].addEventListener("input", () => {
    if (!docUsesGlobal(kind)) {
      ownDocs[kind] = packEditors[kind].value;
    }
  });
}

const insertScaffoldBtn = document.querySelector("#insert-doc-scaffold");
if (insertScaffoldBtn) {
  insertScaffoldBtn.addEventListener("click", () => {
    const kind = activePackTab;
    if (!INHERIT_DOCS.includes(kind) || docUsesGlobal(kind)) return;
    const current = packEditors[kind].value;
    if (!docBlank(current)) {
      const ok = window.confirm(
        "Replace this profile’s " + kind + " text with the commented scaffold?"
      );
      if (!ok) return;
    }
    ownDocs[kind] = DOC_SCAFFOLDS[kind];
    packEditors[kind].value = DOC_SCAFFOLDS[kind];
    packStatusEl.textContent = "Scaffold inserted. Save pack to write it.";
  });
}

const globalEditors = {
  user: document.querySelector("#global-user"),
  glossary: document.querySelector("#global-glossary"),
  rules: document.querySelector("#global-rules"),
};
let globalTab = "user";

function showGlobalTab(name) {
  globalTab = name;
  for (const kind of INHERIT_DOCS) {
    const editor = globalEditors[kind];
    const tab = document.querySelector("#global-tab-" + kind);
    if (!editor) continue;
    const on = kind === name;
    editor.classList.toggle("hidden", !on);
    editor.hidden = !on;
    if (tab) tab.setAttribute("aria-selected", on ? "true" : "false");
  }
}

async function loadGlobalDocs(statusText) {
  const errEl = document.querySelector("#global-error");
  const statusEl = document.querySelector("#global-status");
  try {
    const snap = await invoke("global_docs_snapshot");
    const idEl = document.querySelector("#global-main-id");
    const dirEl = document.querySelector("#global-dir");
    if (idEl) idEl.textContent = snap.main_id || "default";
    if (dirEl) dirEl.textContent = "Directory: " + (snap.dir || "");
    if (globalEditors.user) globalEditors.user.value = snap.user || "";
    if (globalEditors.rules) globalEditors.rules.value = snap.rules || "";
    if (globalEditors.glossary) globalEditors.glossary.value = snap.glossary || "";
    if (errEl) errEl.textContent = "";
    if (statusEl) {
      statusEl.textContent =
        statusText ||
        (snap.ok ? "Pack: ok" : "Pack: invalid" + (snap.reason ? " — " + snap.reason : ""));
    }
  } catch (error) {
    if (errEl) errEl.textContent = errorText(error);
  }
}

async function saveGlobalDocs() {
  const errEl = document.querySelector("#global-error");
  const statusEl = document.querySelector("#global-status");
  if (errEl) errEl.textContent = "";
  try {
    const snap = await invoke("global_docs_save", {
      user: globalEditors.user ? globalEditors.user.value : "",
      rules: globalEditors.rules ? globalEditors.rules.value : "",
      glossary: globalEditors.glossary ? globalEditors.glossary.value : "",
    });
    if (statusEl) {
      statusEl.textContent = snap.ok ? "Saved global docs." : "Saved. Pack is still invalid.";
    }
    try {
      await invoke("reload_soul");
      if (statusEl) statusEl.textContent = "Saved global docs. Reload recorded — applies on next awake.";
    } catch (error) {
      if (errEl) errEl.textContent = "Saved, but reload soul failed: " + errorText(error);
    }
    if (profilesLoaded) {
      loadProfiles(selectedProfileId);
    }
  } catch (error) {
    if (errEl) errEl.textContent = errorText(error);
  }
}

for (const kind of INHERIT_DOCS) {
  const tab = document.querySelector("#global-tab-" + kind);
  if (tab) {
    tab.addEventListener("click", () => showGlobalTab(kind));
  }
}
const globalSaveBtn = document.querySelector("#global-save");
const globalReloadBtn = document.querySelector("#global-reload");
if (globalSaveBtn) globalSaveBtn.addEventListener("click", () => saveGlobalDocs());
if (globalReloadBtn) globalReloadBtn.addEventListener("click", () => loadGlobalDocs("Reloaded from disk."));


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
  renderAccountList(emailGoogleAccounts, "google", snap.google_accounts || [], snap);
  renderAccountList(emailMicrosoftAccounts, "microsoft", snap.microsoft_accounts || [], snap);
  if (emailGoogleConnectBtn) {
    emailGoogleConnectBtn.textContent = (snap.google_accounts || []).length
      ? "Add account"
      : "Connect";
  }
  if (emailMicrosoftConnectBtn) {
    emailMicrosoftConnectBtn.textContent = (snap.microsoft_accounts || []).length
      ? "Add account"
      : "Connect";
  }
  const pending = snap.oauth_pending || "none";
  if (emailOauthPending) {
    emailOauthPending.textContent =
      pending !== "none"
        ? (snap.oauth_message || "Connecting " + pending + "…")
        : "";
  }
  if (emailOauthUrlLine && emailOauthLink) {
    if (pending !== "none" && snap.oauth_authorize_url) {
      emailOauthUrlLine.classList.remove("hidden");
      emailOauthLink.href = snap.oauth_authorize_url;
      emailOauthLink.textContent = snap.oauth_authorize_url;
    } else {
      emailOauthUrlLine.classList.add("hidden");
      emailOauthLink.removeAttribute("href");
      emailOauthLink.textContent = "";
    }
  }
  if (emailOauthError) {
    emailOauthError.textContent = snap.oauth_error || "";
  }
  const busy = pending !== "none";
  if (emailGoogleConnectBtn) emailGoogleConnectBtn.disabled = busy;
  if (emailMicrosoftConnectBtn) emailMicrosoftConnectBtn.disabled = busy;
  if (emailOauthCancelBtn) emailOauthCancelBtn.disabled = !busy;
  if (busy) {
    ensureEmailOauthPoll();
  } else {
    stopEmailOauthPoll();
  }
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

let emailOauthPollTimer = null;
function stopEmailOauthPoll() {
  if (emailOauthPollTimer != null) {
    clearInterval(emailOauthPollTimer);
    emailOauthPollTimer = null;
  }
}
function ensureEmailOauthPoll() {
  if (emailOauthPollTimer != null) return;
  emailOauthPollTimer = setInterval(() => {
    refreshEmail().catch(() => {});
  }, 1000);
}
function renderAccountList(container, provider, accounts, snap) {
  if (!container) return;
  container.replaceChildren();
  const busy = (snap.oauth_pending || "none") !== "none";
  for (const account of accounts) {
    const row = document.createElement("div");
    row.className = "row account-row";
    const title = document.createElement("strong");
    title.textContent = provider === "google" ? "Google" : "Microsoft";
    const email = document.createElement("span");
    email.className = "meta";
    email.textContent = account.email || "(no email)";
    if (account.id) email.title = account.id;
    row.append(title, email);
    if (account.active) {
      const badge = document.createElement("span");
      badge.className = "account-badge";
      badge.textContent = "Active";
      row.append(badge);
    }
    const identity = account.id || account.email || "";
    const activate = document.createElement("button");
    activate.type = "button";
    activate.className = "btn-compact";
    activate.textContent = "Set active";
    activate.disabled = busy || !!account.active || !identity;
    activate.dataset.provider = provider;
    activate.dataset.id = identity;
    activate.dataset.action = "active";
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "btn-compact";
    remove.textContent = "Remove";
    remove.disabled = busy || !identity;
    remove.dataset.provider = provider;
    remove.dataset.id = identity;
    remove.dataset.action = "remove";
    row.append(activate, remove);
    container.append(row);
  }
}

function onAccountClick(event) {
  const button = event.target.closest("button[data-action]");
  if (!button || button.disabled) return;
  const provider = button.dataset.provider;
  const account = button.dataset.id;
  if (!provider || !account) return;
  if (button.dataset.action === "remove") {
    emailAction("email_oauth_disconnect", { provider, account });
  } else if (button.dataset.action === "active") {
    emailAction("email_oauth_set_active", { provider, account });
  }
}

if (emailGoogleAccounts) {
  emailGoogleAccounts.addEventListener("click", onAccountClick);
}
if (emailMicrosoftAccounts) {
  emailMicrosoftAccounts.addEventListener("click", onAccountClick);
}
if (emailGoogleConnectBtn) {
  emailGoogleConnectBtn.addEventListener("click", () => {
    emailAction("email_oauth_connect", { provider: "google" });
  });
}
if (emailMicrosoftConnectBtn) {
  emailMicrosoftConnectBtn.addEventListener("click", () => {
    emailAction("email_oauth_connect", { provider: "microsoft" });
  });
}
if (emailOauthCancelBtn) {
  emailOauthCancelBtn.addEventListener("click", () => {
    emailAction("email_oauth_cancel");
  });
}


function showToolsError(error) {
  toolsError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

const TOOL_PERMISSIONS = [
  ["always_allow", "Always allow"],
  ["ask", "Ask"],
  ["deny", "Deny"],
];

function permissionLabel(value) {
  const match = TOOL_PERMISSIONS.find((row) => row[0] === value);
  return match ? match[1] : value;
}

function shellPermissionValue() {
  const select = toolsList && toolsList.querySelector('select[data-tool="shell"]');
  return select ? select.value : "deny";
}

function syncPolicyEnabled() {
  const ask = shellPermissionValue() === "ask";
  if (toolsConfirmPolicy) {
    toolsConfirmPolicy.disabled = !ask;
  }
  if (toolsPolicyNote) {
    toolsPolicyNote.textContent = ask
      ? "Confirm policy applies while shell is Ask."
      : "Confirm policy applies when shell is Ask.";
  }
}

function collectToolPermissions() {
  if (!toolsList) {
    return [];
  }
  return Array.from(toolsList.querySelectorAll("select[data-tool]")).map((select) => ({
    name: select.dataset.tool,
    permission: select.value,
  }));
}

function renderTools(snap) {
  if (!toolsList) {
    return;
  }
  toolsList.replaceChildren();
  const tools = (snap && snap.tools) || [];
  for (const tool of tools) {
    const row = document.createElement("label");
    row.className = "field tool-row";
    const copy = document.createElement("span");
    copy.className = "tool-copy";
    const name = document.createElement("strong");
    name.textContent = tool.name || "";
    const description = document.createElement("span");
    description.textContent = tool.description || "";
    const floor = document.createElement("span");
    floor.className = "tool-floor";
    floor.textContent = "registry floor: " + (tool.registry_floor || "");
    copy.append(name, description, floor);
    const select = document.createElement("select");
    select.dataset.tool = tool.name || "";
    select.setAttribute("aria-label", (tool.name || "tool") + " permission");
    for (const [value, label] of TOOL_PERMISSIONS) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = label;
      select.append(option);
    }
    select.value = tool.permission || "deny";
    select.addEventListener("change", syncPolicyEnabled);
    row.append(copy, select);
    toolsList.append(row);
  }
  if (toolsConfirmPolicy) {
    toolsConfirmPolicy.value = snap.confirm_policy || "always";
  }
  syncPolicyEnabled();
  const shell = tools.find((tool) => tool.name === "shell");
  toolsStatus.textContent = shell
    ? "Shell is " +
      permissionLabel(shell.permission) +
      ". Confirm policy: " +
      (snap.confirm_policy || "always") +
      "."
    : "Tools Settings loaded.";
  toolsError.textContent = "";
}


let skillsSnap = null;
let skillsSelectedId = "";
const skillsList = document.querySelector("#skills-list");
const skillsTitleInput = document.querySelector("#skills-title-input");
const skillsProcedure = document.querySelector("#skills-procedure");
const skillsPitfalls = document.querySelector("#skills-pitfalls");
const skillsVerify = document.querySelector("#skills-verify");
const skillsSource = document.querySelector("#skills-source");
const skillsDir = document.querySelector("#skills-dir");
const skillsStatus = document.querySelector("#skills-status");
const skillsError = document.querySelector("#skills-error");

function showSkillsError(error) {
  const text = typeof error === "string" ? error : error && error.message ? error.message : "request failed";
  if (skillsError) skillsError.textContent = text;
}

function renderSkills(snap) {
  skillsSnap = snap;
  if (skillsError) skillsError.textContent = "";
  if (skillsDir) skillsDir.textContent = "Directory: " + (snap.skills_dir || "…");
  if (skillsList) {
    skillsList.innerHTML = "";
    for (const row of snap.skills || []) {
      const opt = document.createElement("option");
      opt.value = row.id;
      opt.textContent = row.title + " (" + row.source + ")";
      if (row.id === snap.selected_id) opt.selected = true;
      skillsList.appendChild(opt);
    }
  }
  skillsSelectedId = snap.selected_id || "";
  if (skillsTitleInput) skillsTitleInput.value = snap.selected_title || "";
  if (skillsProcedure) skillsProcedure.value = snap.procedure || "";
  if (skillsPitfalls) skillsPitfalls.value = snap.pitfalls || "";
  if (skillsVerify) skillsVerify.value = snap.verify || "";
  if (skillsSource) skillsSource.textContent = "Source: " + (snap.selected_source || "—");
  renderSkillsSubnav(snap);
}

function renderSkillsSubnav(snap) {
  const list = document.querySelector("#skills-sub-list");
  if (!list) return;
  list.innerHTML = "";
  for (const row of snap.skills || []) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-chip";
    btn.setAttribute("role", "option");
    btn.setAttribute("aria-selected", row.id === snap.selected_id ? "true" : "false");
    const title = document.createElement("span");
    title.className = "profile-chip-label";
    title.textContent = (row.title || row.id) + " (" + row.source + ")";
    btn.appendChild(title);
    btn.addEventListener("click", () => refreshSkills(row.id));
    li.appendChild(btn);
    list.appendChild(li);
  }
}

async function refreshSkills(selectedId) {
  try {
    renderSkills(await invoke("skills_snapshot", { selectedId: selectedId || skillsSelectedId || null }));
  } catch (error) {
    showSkillsError(error);
  }
}

if (skillsList) {
  skillsList.addEventListener("change", () => {
    refreshSkills(skillsList.value);
  });
}
const skillsNewBtn = document.querySelector("#skills-new");
const skillsSaveBtn = document.querySelector("#skills-save");
const skillsDeleteBtn = document.querySelector("#skills-delete");
if (skillsNewBtn) {
  skillsNewBtn.addEventListener("click", () => {
    skillsSelectedId = "";
    if (skillsList) skillsList.selectedIndex = -1;
    if (skillsTitleInput) skillsTitleInput.value = "";
    if (skillsProcedure) skillsProcedure.value = "";
    if (skillsPitfalls) skillsPitfalls.value = "";
    if (skillsVerify) skillsVerify.value = "";
    if (skillsSource) skillsSource.textContent = "Source: user (new)";
    if (skillsStatus) skillsStatus.textContent = "New skill — enter a title and Save.";
    if (skillsError) skillsError.textContent = "";
  });
}
if (skillsSaveBtn) {
  skillsSaveBtn.addEventListener("click", async () => {
    try {
      const snap = await invoke("skills_save", {
        id: skillsSelectedId || "",
        title: skillsTitleInput ? skillsTitleInput.value : "",
        procedure: skillsProcedure ? skillsProcedure.value : "",
        pitfalls: skillsPitfalls ? skillsPitfalls.value : "",
        verify: skillsVerify ? skillsVerify.value : "",
        forceUser: false,
      });
      renderSkills(snap);
      if (skillsStatus) skillsStatus.textContent = "Skill saved.";
    } catch (error) {
      showSkillsError(error);
    }
  });
}
if (skillsDeleteBtn) {
  skillsDeleteBtn.addEventListener("click", async () => {
    if (!skillsSelectedId) return;
    try {
      const snap = await invoke("skills_delete", { id: skillsSelectedId });
      renderSkills(snap);
      if (skillsStatus) skillsStatus.textContent = "Skill deleted.";
    } catch (error) {
      showSkillsError(error);
    }
  });
}



let remoteAgentCompanionEnabled = false;

function applyTimersRunOnAvailability(hasCompanion) {
  remoteAgentCompanionEnabled = !!hasCompanion;
  if (!timersRunOn) return;
  for (const opt of timersRunOn.options) {
    if (opt.value === "local") {
      opt.disabled = false;
    } else {
      opt.disabled = !remoteAgentCompanionEnabled;
    }
  }
  if (!remoteAgentCompanionEnabled && timersRunOn.value !== "local") {
    timersRunOn.value = "local";
  }
  if (timersRunOnHint) {
    timersRunOnHint.textContent = remoteAgentCompanionEnabled
      ? "Companion enabled. run_on=local fires here; companion/auto use presence + fire leases (slice 2)."
      : "Companion options unlock when a Remote Agent is enabled. Dispatch follows run_on + presence.";
  }
}

/* ---- Timers ---- */
const timersList = document.querySelector("#timers-list");
const timersShowAll = document.querySelector("#timers-show-all");
const timersActive = document.querySelector("#timers-active");
const timersAction = document.querySelector("#timers-action");
const timersRunOn = document.querySelector("#timers-run-on");
const timersRunOnHint = document.querySelector("#timers-run-on-hint");
const timersKind = document.querySelector("#timers-kind");
const timersWhen = document.querySelector("#timers-when");
const timersTitleInput = document.querySelector("#timers-title-input");
const timersMessage = document.querySelector("#timers-message");
const timersMessageLabel = document.querySelector("#timers-message-label");
const timersEnabled = document.querySelector("#timers-enabled");
const timersStatus = document.querySelector("#timers-status");
const timersError = document.querySelector("#timers-error");
const timersNewBtn = document.querySelector("#timers-new");
const timersSaveBtn = document.querySelector("#timers-save");
const timersDeleteBtn = document.querySelector("#timers-delete");
let timersSelectedId = "";
let timersSelectedProfile = "";
let timersRows = [];

function showTimersError(error) {
  if (!timersError) return;
  timersError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}


function syncTimersActionLabel() {
  if (!timersMessageLabel) return;
  const action = timersAction ? timersAction.value : "notify";
  timersMessageLabel.textContent = action === "agent_task" ? "Prompt" : "Message";
}

function clearTimersForm() {
  timersSelectedId = "";
  timersSelectedProfile = "";
  if (timersList) timersList.selectedIndex = -1;
  if (timersAction) timersAction.value = "notify";
  if (timersRunOn) timersRunOn.value = "local";
  if (timersKind) timersKind.value = "daily";
  if (timersWhen) timersWhen.value = "";
  if (timersTitleInput) timersTitleInput.value = "";
  if (timersMessage) timersMessage.value = "";
  if (timersEnabled) timersEnabled.checked = true;
  syncTimersActionLabel();
}

function fillTimersForm(row) {
  timersSelectedId = row.id || "";
  timersSelectedProfile = row.profileId || "";
  if (timersAction) timersAction.value = row.action || "notify";
  if (timersRunOn) timersRunOn.value = row.runOn || "local";
  if (timersKind) timersKind.value = row.kind || "daily";
  if (timersWhen) timersWhen.value = row.when || "";
  if (timersTitleInput) timersTitleInput.value = row.title || "";
  if (timersMessage) timersMessage.value = row.message || "";
  if (timersEnabled) timersEnabled.checked = !!row.enabled;
  syncTimersActionLabel();
}

function renderTimers(snap) {
  if (timersError) timersError.textContent = "";
  timersRows = snap.rows || [];
  if (timersActive) {
    timersActive.textContent = "Active profile: " + (snap.activeProfileId || "—");
  }
  if (timersShowAll) timersShowAll.checked = !!snap.showAll;
  if (timersList) {
    const prev = timersSelectedId;
    timersList.innerHTML = "";
    for (const row of timersRows) {
      const opt = document.createElement("option");
      opt.value = row.profileId + "\t" + row.id;
      const label = (snap.showAll ? "[" + row.profileName + "] " : "") +
        (row.action === "agent_task" ? "agent " : "") +
        row.kind + " " + row.when + " — " + row.title + (row.enabled ? "" : " (off)");
      opt.textContent = label;
      timersList.appendChild(opt);
    }
    if (prev) {
      for (const opt of timersList.options) {
        if (opt.value.endsWith("\t" + prev) || opt.value === timersSelectedProfile + "\t" + prev) {
          opt.selected = true;
          break;
        }
      }
    }
  }
  renderTimersSubnav(snap);
}

function renderTimersSubnav(snap) {
  const list = document.querySelector("#timers-sub-list");
  if (!list) return;
  list.innerHTML = "";
  for (const row of timersRows) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-chip";
    btn.setAttribute("role", "option");
    const selected = row.id === timersSelectedId && row.profileId === (timersSelectedProfile || row.profileId);
    btn.setAttribute("aria-selected", selected ? "true" : "false");
    const title = document.createElement("span");
    title.className = "profile-chip-label";
    const label = (snap.showAll ? "[" + row.profileName + "] " : "") + (row.title || row.kind);
    title.textContent = label + (row.enabled ? "" : " (off)");
    btn.appendChild(title);
    btn.title = row.kind + " " + row.when;
    btn.addEventListener("click", () => {
      timersSelectedId = row.id;
      timersSelectedProfile = row.profileId;
      fillTimersForm(row);
      renderTimersSubnav(snap);
      if (timersList) {
        for (const opt of timersList.options) {
          if (opt.value === row.profileId + "\t" + row.id) {
            opt.selected = true;
            break;
          }
        }
      }
    });
    li.appendChild(btn);
    list.appendChild(li);
  }
}

async function refreshTimers() {
  try {
    const showAll = timersShowAll ? timersShowAll.checked : false;
    renderTimers(await invoke("timers_snapshot", { showAll }));
    try {
      const ra = await invoke("remote_agent_snapshot", {});
      applyTimersRunOnAvailability(!!ra.hasEnabledCompanion);
    } catch (_) {
      applyTimersRunOnAvailability(false);
    }
  } catch (error) {
    showTimersError(error);
  }
}

if (timersShowAll) {
  timersShowAll.addEventListener("change", () => refreshTimers());
}
if (timersAction) {
  timersAction.addEventListener("change", () => syncTimersActionLabel());
}
if (timersList) {
  timersList.addEventListener("change", () => {
    const val = timersList.value || "";
    const parts = val.split("\t");
    const profileId = parts[0] || "";
    const id = parts[1] || "";
    const row = timersRows.find((r) => r.id === id && r.profileId === profileId);
    if (row) fillTimersForm(row);
  });
}
if (timersNewBtn) {
  timersNewBtn.addEventListener("click", () => {
    clearTimersForm();
    if (timersStatus) timersStatus.textContent = "New timer — fill when + message, then Save.";
  });
}
if (timersSaveBtn) {
  timersSaveBtn.addEventListener("click", async () => {
    try {
      const snap = await invoke("timers_upsert", {
        profileId: timersSelectedProfile || null,
        id: timersSelectedId || null,
        kind: timersKind ? timersKind.value : "daily",
        action: timersAction ? timersAction.value : "notify",
        runOn: timersRunOn ? timersRunOn.value : "local",
        when: timersWhen ? timersWhen.value : "",
        title: timersTitleInput ? timersTitleInput.value : "",
        message: timersMessage ? timersMessage.value : "",
        enabled: timersEnabled ? timersEnabled.checked : true,
      });
      renderTimers(snap);
      if (timersStatus) timersStatus.textContent = "Timer saved.";
    } catch (error) {
      showTimersError(error);
    }
  });
}
if (timersDeleteBtn) {
  timersDeleteBtn.addEventListener("click", async () => {
    if (!timersSelectedId || !timersSelectedProfile) return;
    try {
      const snap = await invoke("timers_delete", {
        profileId: timersSelectedProfile,
        id: timersSelectedId,
      });
      clearTimersForm();
      renderTimers(snap);
      if (timersStatus) timersStatus.textContent = "Timer deleted.";
    } catch (error) {
      showTimersError(error);
    }
  });
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
        confirmPolicy: toolsConfirmPolicy.value,
        permissions: collectToolPermissions(),
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
const hudShrunkRange = document.querySelector("#hud-shrunk-range");
const hudShrunkNumber = document.querySelector("#hud-shrunk-px");
const hudShrunkStatus = document.querySelector("#hud-shrunk-status");
const hudBloomRange = document.querySelector("#hud-bloom-range");
const hudBloomNumber = document.querySelector("#hud-bloom-percent");
const hudBloomStatus = document.querySelector("#hud-bloom-status");
const hudOpacityRange = document.querySelector("#hud-opacity-range");
const hudOpacityNumber = document.querySelector("#hud-opacity-percent");
const hudOpacityStatus = document.querySelector("#hud-opacity-status");
const voiceTestBox = document.querySelector("#voice-test");
let voiceTestEditing = false;
let hudShrunkTimer = null;
let hudBloomTimer = null;

const TEXT_SIZES = ["xx-small", "x-small", "small", "medium", "large"];
const HUD_SHRUNK_MIN = 96;
const HUD_SHRUNK_MAX = 280;
const HUD_SHRUNK_DEFAULT = 120;
const HUD_BLOOM_MIN = 25;
const HUD_BLOOM_MAX = 200;
const HUD_BLOOM_DEFAULT = 100;

function clampShrunkPx(value) {
  const n = Math.round(Number(value));
  if (!Number.isFinite(n)) return HUD_SHRUNK_DEFAULT;
  return Math.min(HUD_SHRUNK_MAX, Math.max(HUD_SHRUNK_MIN, n));
}

function clampBloomPercent(value) {
  const n = Math.round(Number(value));
  if (!Number.isFinite(n)) return HUD_BLOOM_DEFAULT;
  return Math.min(HUD_BLOOM_MAX, Math.max(HUD_BLOOM_MIN, n));
}

function applyHudShrunk(px) {
  const clamped = clampShrunkPx(px);
  if (hudShrunkRange) hudShrunkRange.value = String(clamped);
  if (hudShrunkNumber) hudShrunkNumber.value = String(clamped);
}

function applyHudBloom(percent) {
  const clamped = clampBloomPercent(percent);
  if (hudBloomRange) hudBloomRange.value = String(clamped);
  if (hudBloomNumber) hudBloomNumber.value = String(clamped);
}

function applyTextSize(size) {
  const value = TEXT_SIZES.includes(size) ? size : "x-small";
  document.documentElement.setAttribute("data-text-size", value);
  if (uiTextSizeSelect) {
    uiTextSizeSelect.value = value;
  }
}

if (voiceTestBox) {
  voiceTestBox.addEventListener("change", async () => {
    const enabled = voiceTestBox.checked;
    voiceTestEditing = true;
    try {
      const status = await invoke("set_voice_test", { enabled });
      voiceTestEditing = false;
      show(status, true);
      if (uiPrefsError) uiPrefsError.textContent = "";
    } catch (error) {
      voiceTestEditing = false;
      showUiPrefsError(error);
      try {
        show(await invoke("status"), true);
      } catch (statusError) {
        showUiPrefsError(statusError);
      }
    }
  });
}

const voiceAgentS2sBox = document.querySelector("#voice-agent-s2s");
const voiceAgentS2sStatus = document.querySelector("#voice-agent-s2s-status");
const voiceAgentS2sError = document.querySelector("#voice-agent-s2s-error");
let voiceAgentS2sEditing = false;

function showVoiceAgentS2sError(error) {
  if (!voiceAgentS2sError) return;
  voiceAgentS2sError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function applyVoiceAgentS2sSnapshot(snap) {
  if (voiceAgentS2sBox && snap && typeof snap.enabled === "boolean" && !voiceAgentS2sEditing) {
    voiceAgentS2sBox.checked = !!snap.enabled;
  }
  if (voiceAgentS2sStatus) {
    voiceAgentS2sStatus.textContent = (snap && snap.message) || "";
  }
}

async function refreshVoiceAgentS2s() {
  if (!voiceAgentS2sBox) return;
  try {
    if (voiceAgentS2sError) voiceAgentS2sError.textContent = "";
    const snap = await invoke("voice_agent_s2s_snapshot");
    applyVoiceAgentS2sSnapshot(snap);
  } catch (error) {
    showVoiceAgentS2sError(error);
  }
}

if (voiceAgentS2sBox) {
  voiceAgentS2sBox.addEventListener("change", async () => {
    const enabled = voiceAgentS2sBox.checked;
    voiceAgentS2sEditing = true;
    try {
      if (voiceAgentS2sError) voiceAgentS2sError.textContent = "";
      const snap = await invoke("voice_agent_s2s_set", { enabled });
      voiceAgentS2sEditing = false;
      applyVoiceAgentS2sSnapshot(snap);
    } catch (error) {
      voiceAgentS2sEditing = false;
      showVoiceAgentS2sError(error);
      refreshVoiceAgentS2s();
    }
  });
  refreshVoiceAgentS2s();
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
    applyHudShrunk(typeof snap.hud_shrunk_px === "number" ? snap.hud_shrunk_px : HUD_SHRUNK_DEFAULT);
    applyHudBloom(
      typeof snap.hud_bloom_intensity === "number" ? snap.hud_bloom_intensity : HUD_BLOOM_DEFAULT,
    );
    applyHudOpacity(
      typeof snap.hud_opacity === "number" ? snap.hud_opacity : 55,
    );
    if (uiPrefsStatus) {
      uiPrefsStatus.textContent = "UI text size: " + (snap.text_size || "x-small");
    }
  } catch (error) {
    applyTextSize("x-small");
    applyHudShrunk(HUD_SHRUNK_DEFAULT);
    applyHudBloom(HUD_BLOOM_DEFAULT);
    applyHudOpacity(55);
    showUiPrefsError(error);
  }
}

async function saveHudShrunk(px) {
  const clamped = clampShrunkPx(px);
  applyHudShrunk(clamped);
  if (hudShrunkStatus) hudShrunkStatus.textContent = "Saving…";
  try {
    const snap = await invoke("ui_prefs_set_hud_shrunk_px", { px: clamped });
    applyHudShrunk(typeof snap.hud_shrunk_px === "number" ? snap.hud_shrunk_px : clamped);
    if (hudShrunkStatus) {
      hudShrunkStatus.textContent = "Shrunk HUD: " + (snap.hud_shrunk_px || clamped) + " px";
    }
    try {
      await invoke("hud_apply_shrunk_size");
    } catch (_layoutError) {
      // Settings can save the pref when the HUD window is not open yet.
    }
  } catch (error) {
    if (hudShrunkStatus) hudShrunkStatus.textContent = "Save failed: " + errorText(error);
    refreshUiPrefs();
  }
}

async function saveHudBloom(percent) {
  const clamped = clampBloomPercent(percent);
  applyHudBloom(clamped);
  if (hudBloomStatus) hudBloomStatus.textContent = "Saving…";
  try {
    const snap = await invoke("ui_prefs_set_hud_bloom_intensity", { percent: clamped });
    applyHudBloom(
      typeof snap.hud_bloom_intensity === "number" ? snap.hud_bloom_intensity : clamped,
    );
    if (hudBloomStatus) {
      hudBloomStatus.textContent = "Bloom intensity: " + (snap.hud_bloom_intensity || clamped) + "%";
    }
  } catch (error) {
    if (hudBloomStatus) hudBloomStatus.textContent = "Save failed: " + errorText(error);
    refreshUiPrefs();
  }
}


function clampHudOpacity(percent) {
  const n = Number(percent);
  if (!Number.isFinite(n)) return 55;
  return Math.max(35, Math.min(100, Math.round(n)));
}

function applyHudOpacity(percent) {
  const clamped = clampHudOpacity(percent);
  if (hudOpacityRange) hudOpacityRange.value = String(clamped);
  if (hudOpacityNumber) hudOpacityNumber.value = String(clamped);
}

async function saveHudOpacity(percent) {
  const clamped = clampHudOpacity(percent);
  applyHudOpacity(clamped);
  if (hudOpacityStatus) hudOpacityStatus.textContent = "Saving…";
  try {
    const snap = await invoke("ui_prefs_set_hud_opacity", { percent: clamped });
    applyHudOpacity(typeof snap.hud_opacity === "number" ? snap.hud_opacity : clamped);
    if (hudOpacityStatus) {
      hudOpacityStatus.textContent = "HUD opacity: " + (snap.hud_opacity || clamped) + "%";
    }
  } catch (error) {
    if (hudOpacityStatus) {
      hudOpacityStatus.textContent =
        "Save failed: " + (typeof error === "string" ? error : error && error.message ? error.message : "request failed");
    }
  }
}

let hudOpacitySaveTimer = null;
function scheduleHudOpacitySave(percent) {
  applyHudOpacity(percent);
  if (hudOpacitySaveTimer) clearTimeout(hudOpacitySaveTimer);
  hudOpacitySaveTimer = setTimeout(() => {
    hudOpacitySaveTimer = null;
    void saveHudOpacity(percent);
  }, 200);
}

function scheduleHudShrunkSave(px) {
  applyHudShrunk(px);
  if (hudShrunkTimer) clearTimeout(hudShrunkTimer);
  hudShrunkTimer = setTimeout(() => {
    hudShrunkTimer = null;
    void saveHudShrunk(px);
  }, 200);
}

function scheduleHudBloomSave(percent) {
  applyHudBloom(percent);
  if (hudBloomTimer) clearTimeout(hudBloomTimer);
  hudBloomTimer = setTimeout(() => {
    hudBloomTimer = null;
    void saveHudBloom(percent);
  }, 200);
}

if (hudShrunkRange && hudShrunkNumber) {
  hudShrunkRange.addEventListener("input", () => {
    hudShrunkNumber.value = hudShrunkRange.value;
    scheduleHudShrunkSave(hudShrunkRange.value);
  });
  hudShrunkNumber.addEventListener("change", () => {
    const px = clampShrunkPx(hudShrunkNumber.value);
    hudShrunkNumber.value = String(px);
    hudShrunkRange.value = String(px);
    scheduleHudShrunkSave(px);
  });
}

if (hudBloomRange && hudBloomNumber) {
  hudBloomRange.addEventListener("input", () => {
    hudBloomNumber.value = hudBloomRange.value;
    scheduleHudBloomSave(hudBloomRange.value);
  });
  hudBloomNumber.addEventListener("change", () => {
    const percent = clampBloomPercent(hudBloomNumber.value);
    hudBloomNumber.value = String(percent);
    hudBloomRange.value = String(percent);
    scheduleHudBloomSave(percent);
  });
}

if (hudOpacityRange && hudOpacityNumber) {
  hudOpacityRange.addEventListener("input", () => {
    hudOpacityNumber.value = hudOpacityRange.value;
    scheduleHudOpacitySave(hudOpacityRange.value);
  });
  hudOpacityNumber.addEventListener("input", () => {
    const percent = clampHudOpacity(hudOpacityNumber.value);
    hudOpacityRange.value = String(percent);
    scheduleHudOpacitySave(percent);
  });
  hudOpacityNumber.addEventListener("change", () => {
    const percent = clampHudOpacity(hudOpacityNumber.value);
    hudOpacityRange.value = String(percent);
    scheduleHudOpacitySave(percent);
  });
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


const kwsGlobalInput = document.querySelector("#kws-global");
const kwsShortInput = document.querySelector("#kws-short");
const kwsStatus = document.querySelector("#kws-status");
const kwsError = document.querySelector("#kws-error");
let kwsSaveTimer = null;

function showKwsError(error) {
  if (!kwsError) return;
  kwsError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function applyKwsSnapshot(snap) {
  if (kwsGlobalInput && typeof snap.global === "number") {
    kwsGlobalInput.value = snap.global.toFixed(2);
  }
  if (kwsShortInput && typeof snap.short === "number") {
    kwsShortInput.value = snap.short.toFixed(2);
  }
  if (kwsStatus) {
    kwsStatus.textContent = snap.message || "";
  }
}

async function refreshKwsThresholds() {
  if (!kwsGlobalInput || !kwsShortInput) return;
  try {
    if (kwsError) kwsError.textContent = "";
    const snap = await invoke("kws_thresholds_snapshot");
    applyKwsSnapshot(snap);
  } catch (error) {
    showKwsError(error);
  }
}

async function saveKwsThresholds() {
  if (!kwsGlobalInput || !kwsShortInput) return;
  const global = Number(kwsGlobalInput.value);
  const short = Number(kwsShortInput.value);
  try {
    if (kwsError) kwsError.textContent = "";
    const snap = await invoke("kws_thresholds_set", { global, short });
    applyKwsSnapshot(snap);
  } catch (error) {
    showKwsError(error);
    refreshKwsThresholds();
  }
}

function scheduleKwsSave() {
  // Debounce live ReloadKws — range drag steps 0.15→0.05 would otherwise
  // rebuild ONNX weights on every tick. change used to flush immediately and
  // defeated the input debounce on some browsers.
  if (kwsSaveTimer) clearTimeout(kwsSaveTimer);
  kwsSaveTimer = setTimeout(() => {
    kwsSaveTimer = null;
    saveKwsThresholds();
  }, 450);
}

if (kwsGlobalInput && kwsShortInput) {
  for (const el of [kwsGlobalInput, kwsShortInput]) {
    el.addEventListener("input", scheduleKwsSave);
    el.addEventListener("change", scheduleKwsSave);
  }
  refreshKwsThresholds();
}

const freeSpeechSilenceInput = document.querySelector("#free-speech-silence");
const freeSpeechSilenceStatus = document.querySelector("#free-speech-silence-status");
const freeSpeechSilenceError = document.querySelector("#free-speech-silence-error");
let freeSpeechSilenceTimer = null;

function showFreeSpeechSilenceError(error) {
  if (!freeSpeechSilenceError) return;
  freeSpeechSilenceError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function applyFreeSpeechSilenceSnapshot(snap) {
  if (freeSpeechSilenceInput && typeof snap.seconds === "number") {
    freeSpeechSilenceInput.value = snap.seconds.toFixed(1);
  }
  if (freeSpeechSilenceStatus) {
    freeSpeechSilenceStatus.textContent = snap.message || "";
  }
}

async function refreshFreeSpeechSilence() {
  if (!freeSpeechSilenceInput) return;
  try {
    if (freeSpeechSilenceError) freeSpeechSilenceError.textContent = "";
    const snap = await invoke("free_speech_silence_snapshot");
    applyFreeSpeechSilenceSnapshot(snap);
  } catch (error) {
    showFreeSpeechSilenceError(error);
  }
}

async function saveFreeSpeechSilence() {
  if (!freeSpeechSilenceInput) return;
  const seconds = Number(freeSpeechSilenceInput.value);
  try {
    if (freeSpeechSilenceError) freeSpeechSilenceError.textContent = "";
    const snap = await invoke("free_speech_silence_set", { seconds });
    applyFreeSpeechSilenceSnapshot(snap);
  } catch (error) {
    showFreeSpeechSilenceError(error);
    refreshFreeSpeechSilence();
  }
}

function scheduleFreeSpeechSilenceSave() {
  // Debounce live ReloadUtterance the same way KWS debounces ReloadKws.
  if (freeSpeechSilenceTimer) clearTimeout(freeSpeechSilenceTimer);
  freeSpeechSilenceTimer = setTimeout(() => {
    freeSpeechSilenceTimer = null;
    saveFreeSpeechSilence();
  }, 450);
}

if (freeSpeechSilenceInput) {
  freeSpeechSilenceInput.addEventListener("input", scheduleFreeSpeechSilenceSave);
  freeSpeechSilenceInput.addEventListener("change", scheduleFreeSpeechSilenceSave);
  refreshFreeSpeechSilence();
}

const ttsPlaybackRange = document.querySelector("#tts-playback-range");
const ttsPlaybackSeconds = document.querySelector("#tts-playback-seconds");
const ttsPlaybackStatus = document.querySelector("#tts-playback-status");
const ttsPlaybackError = document.querySelector("#tts-playback-error");
let ttsPlaybackTimer = null;

function showTtsPlaybackError(error) {
  if (!ttsPlaybackError) return;
  ttsPlaybackError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function applyTtsPlaybackSnapshot(snap) {
  if (snap && typeof snap.seconds === "number") {
    const seconds = String(Math.round(snap.seconds));
    if (ttsPlaybackRange) ttsPlaybackRange.value = seconds;
    if (ttsPlaybackSeconds) ttsPlaybackSeconds.value = seconds;
  }
  if (ttsPlaybackStatus) {
    ttsPlaybackStatus.textContent = (snap && snap.message) || "";
  }
}

async function refreshTtsPlaybackTimeout() {
  if (!ttsPlaybackRange && !ttsPlaybackSeconds) return;
  try {
    if (ttsPlaybackError) ttsPlaybackError.textContent = "";
    const snap = await invoke("tts_playback_timeout_snapshot");
    applyTtsPlaybackSnapshot(snap);
  } catch (error) {
    showTtsPlaybackError(error);
  }
}

async function saveTtsPlaybackTimeout(seconds) {
  try {
    if (ttsPlaybackError) ttsPlaybackError.textContent = "";
    const snap = await invoke("tts_playback_timeout_set", {
      seconds: Number(seconds),
    });
    applyTtsPlaybackSnapshot(snap);
  } catch (error) {
    showTtsPlaybackError(error);
    refreshTtsPlaybackTimeout();
  }
}

function scheduleTtsPlaybackSave(seconds) {
  // Debounce live ReloadPlayback the same way free-speech debounces ReloadUtterance.
  if (ttsPlaybackTimer) clearTimeout(ttsPlaybackTimer);
  ttsPlaybackTimer = setTimeout(() => {
    ttsPlaybackTimer = null;
    saveTtsPlaybackTimeout(seconds);
  }, 450);
}

if (ttsPlaybackRange && ttsPlaybackSeconds) {
  ttsPlaybackRange.addEventListener("input", () => {
    ttsPlaybackSeconds.value = ttsPlaybackRange.value;
    scheduleTtsPlaybackSave(ttsPlaybackRange.value);
  });
  ttsPlaybackSeconds.addEventListener("input", () => {
    const seconds = Math.round(Number(ttsPlaybackSeconds.value));
    if (Number.isFinite(seconds)) {
      ttsPlaybackRange.value = String(seconds);
    }
    scheduleTtsPlaybackSave(ttsPlaybackSeconds.value);
  });
  ttsPlaybackSeconds.addEventListener("change", () => {
    const seconds = Math.round(Number(ttsPlaybackSeconds.value));
    if (Number.isFinite(seconds)) {
      ttsPlaybackSeconds.value = String(seconds);
      ttsPlaybackRange.value = String(seconds);
      scheduleTtsPlaybackSave(seconds);
    } else {
      scheduleTtsPlaybackSave(ttsPlaybackSeconds.value);
    }
  });
  refreshTtsPlaybackTimeout();
}


/* ---- Messengers ---- */
const messengersSubnav = document.querySelector("#messengers-subnav");
const messengersProfile = document.querySelector("#messengers-profile");
const messengersStorage = document.querySelector("#messengers-storage");
const messengersStatus = document.querySelector("#messengers-status");
const messengersError = document.querySelector("#messengers-error");
const msgTgEnabled = document.querySelector("#msg-tg-enabled");
const msgTgDefault = document.querySelector("#msg-tg-default");
const msgTgReceive = document.querySelector("#msg-tg-receive");
const msgTgVoice = document.querySelector("#msg-tg-voice");
const msgTgChatId = document.querySelector("#msg-tg-chat-id");
const msgTgToken = document.querySelector("#msg-tg-token");
const msgTgTokenStatus = document.querySelector("#msg-tg-token-status");
const msgTgSave = document.querySelector("#msg-tg-save");
const msgTgClear = document.querySelector("#msg-tg-clear-token");
const msgDeskDefault = document.querySelector("#msg-desk-default");
const msgDeskReceive = document.querySelector("#msg-desk-receive");
const msgDeskVoice = document.querySelector("#msg-desk-voice");
const msgDeskSave = document.querySelector("#msg-desk-save");
const messengersTelegramDetail = document.querySelector("#messengers-telegram-detail");
const messengersDesktopDetail = document.querySelector("#messengers-desktop-detail");
let messengersSnap = null;
let messengersChannel = "telegram";

function showMessengersError(error) {
  if (!messengersError) return;
  messengersError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function applyMessengersSnapshot(snap, statusText) {
  messengersSnap = snap;
  if (messengersError) messengersError.textContent = "";
  if (messengersStatus) messengersStatus.textContent = statusText || "";
  if (messengersProfile) {
    messengersProfile.textContent =
      "Profile: " + (snap.profileName || snap.profileId || "—") + " (" + (snap.profileId || "") + ")";
  }
  if (messengersStorage) {
    messengersStorage.textContent =
      "Storage: " + (snap.storageBackend || "—") + (snap.storageMessage ? " — " + snap.storageMessage : "");
  }
  messengersChannel = snap.selectedChannel || "telegram";
  const isTg = messengersChannel === "telegram";
  if (messengersTelegramDetail) {
    messengersTelegramDetail.classList.toggle("hidden", !isTg);
    messengersTelegramDetail.hidden = !isTg;
  }
  if (messengersDesktopDetail) {
    messengersDesktopDetail.classList.toggle("hidden", isTg);
    messengersDesktopDetail.hidden = isTg;
  }
  if (msgTgEnabled) msgTgEnabled.checked = !!snap.telegramEnabled;
  if (msgTgDefault) msgTgDefault.checked = !!snap.telegramDefault;
  if (msgTgReceive) msgTgReceive.checked = !!snap.telegramReceiveAll;
  if (msgTgVoice) msgTgVoice.checked = !!snap.telegramVoice;
  if (msgTgChatId) msgTgChatId.value = snap.telegramChatId || "";
  if (msgTgTokenStatus) {
    msgTgTokenStatus.textContent = snap.hasBotToken ? "Token: saved" : "Token: not set";
  }
  if (msgTgToken) msgTgToken.value = "";
  if (msgDeskDefault) msgDeskDefault.checked = !!snap.desktopDefault;
  if (msgDeskReceive) msgDeskReceive.checked = !!snap.desktopReceiveAll;
  if (msgDeskVoice) msgDeskVoice.checked = !!snap.desktopVoice;
  renderMessengersSubnav(snap);
}

function renderMessengersSubnav(snap) {
  const list = document.querySelector("#messengers-sub-list");
  if (!list) return;
  list.innerHTML = "";
  for (const row of snap.channels || []) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-chip";
    btn.setAttribute("role", "option");
    btn.setAttribute("aria-selected", row.id === (snap.selectedChannel || "") ? "true" : "false");
    const title = document.createElement("span");
    title.className = "profile-chip-label";
    title.textContent = row.label + (row.bound ? "" : " · unbound");
    btn.appendChild(title);
    btn.addEventListener("click", () => refreshMessengers(snap.profileId, row.id));
    li.appendChild(btn);
    list.appendChild(li);
  }
}

async function refreshMessengers(profileId, channel) {
  try {
    const args = {};
    if (profileId) args.profileId = profileId;
    if (channel) args.selectedChannel = channel;
    applyMessengersSnapshot(await invoke("messengers_snapshot", args), "");
  } catch (error) {
    showMessengersError(error);
  }
}

async function saveMessengers(fromDesktop) {
  if (!messengersSnap) return;
  try {
    const args = {
      profileId: messengersSnap.profileId,
      telegramEnabled: msgTgEnabled ? !!msgTgEnabled.checked : false,
      telegramDefault: msgTgDefault ? !!msgTgDefault.checked : false,
      telegramReceiveAll: msgTgReceive ? !!msgTgReceive.checked : false,
      telegramVoice: msgTgVoice ? !!msgTgVoice.checked : false,
      telegramChatId: msgTgChatId ? msgTgChatId.value : "",
      desktopDefault: msgDeskDefault ? !!msgDeskDefault.checked : true,
      desktopReceiveAll: msgDeskReceive ? !!msgDeskReceive.checked : true,
      desktopVoice: msgDeskVoice ? !!msgDeskVoice.checked : true,
      botToken: msgTgToken && msgTgToken.value ? msgTgToken.value : null,
      clearBotToken: false,
    };
    if (fromDesktop) {
      // keep telegram fields from snap
      args.telegramEnabled = !!messengersSnap.telegramEnabled;
      args.telegramDefault = !!messengersSnap.telegramDefault;
      args.telegramReceiveAll = !!messengersSnap.telegramReceiveAll;
      args.telegramVoice = !!messengersSnap.telegramVoice;
      args.telegramChatId = messengersSnap.telegramChatId || "";
      args.botToken = null;
    }
    applyMessengersSnapshot(await invoke("messengers_save", { args }), "Saved.");
  } catch (error) {
    showMessengersError(error);
  }
}

if (msgTgSave) msgTgSave.addEventListener("click", () => saveMessengers(false));
if (msgDeskSave) msgDeskSave.addEventListener("click", () => saveMessengers(true));
if (msgTgClear) {
  msgTgClear.addEventListener("click", async () => {
    try {
      applyMessengersSnapshot(
        await invoke("messengers_clear_token", {
          profileId: messengersSnap ? messengersSnap.profileId : null,
        }),
        "Token cleared."
      );
    } catch (error) {
      showMessengersError(error);
    }
  });
}

const timersSubnav = document.querySelector("#timers-subnav");
const skillsSubnav = document.querySelector("#skills-subnav");
const timersSubNew = document.querySelector("#timers-sub-new");
const skillsSubNew = document.querySelector("#skills-sub-new");
if (timersSubNew && timersNewBtn) {
  timersSubNew.addEventListener("click", () => timersNewBtn.click());
}
if (skillsSubNew && skillsNewBtn) {
  skillsSubNew.addEventListener("click", () => skillsNewBtn.click());
}




/* ---- Remote Agent ---- */
const remoteAgentSubnav = document.querySelector("#remote-agent-subnav");
const remoteAgentStorage = document.querySelector("#remote-agent-storage");
const remoteAgentStatus = document.querySelector("#remote-agent-status");
const remoteAgentError = document.querySelector("#remote-agent-error");
const remoteAgentId = document.querySelector("#remote-agent-id");
const remoteAgentName = document.querySelector("#remote-agent-name");
const remoteAgentHostname = document.querySelector("#remote-agent-hostname");
const remoteAgentSshUser = document.querySelector("#remote-agent-ssh-user");
const remoteAgentRoleTimers = document.querySelector("#remote-agent-role-timers");
const remoteAgentRoleOutbox = document.querySelector("#remote-agent-role-outbox");
const remoteAgentRoleWebhook = document.querySelector("#remote-agent-role-webhook");
const remoteAgentRoleTelegram = document.querySelector("#remote-agent-role-telegram");
const remoteAgentConflict = document.querySelector("#remote-agent-conflict");
const remoteAgentSecret = document.querySelector("#remote-agent-secret");
const remoteAgentSecretStatus = document.querySelector("#remote-agent-secret-status");
const remoteAgentEnabled = document.querySelector("#remote-agent-enabled");
const remoteAgentOauthMirror = document.querySelector("#remote-agent-oauth-mirror");
const remoteAgentSave = document.querySelector("#remote-agent-save");
const remoteAgentTest = document.querySelector("#remote-agent-test");
const remoteAgentInstall = document.querySelector("#remote-agent-install");
const remoteAgentClearSecret = document.querySelector("#remote-agent-clear-secret");
const remoteAgentDelete = document.querySelector("#remote-agent-delete");
const remoteAgentSubNew = document.querySelector("#remote-agent-sub-new");
let remoteAgentSnap = null;

function showRemoteAgentError(error) {
  if (!remoteAgentError) return;
  remoteAgentError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function applyRemoteAgentSnapshot(snap, statusText) {
  remoteAgentSnap = snap;
  if (remoteAgentError) remoteAgentError.textContent = "";
  if (remoteAgentStatus) remoteAgentStatus.textContent = statusText || snap.testStatus || "";
  if (remoteAgentStorage) {
    remoteAgentStorage.textContent =
      "Storage: " + (snap.storageBackend || "—") + (snap.storageMessage ? " — " + snap.storageMessage : "");
  }
  if (remoteAgentId) remoteAgentId.value = snap.id || "";
  if (remoteAgentName) remoteAgentName.value = snap.name || "";
  if (remoteAgentHostname) remoteAgentHostname.value = snap.tailscaleHostname || "";
  if (remoteAgentSshUser) remoteAgentSshUser.value = snap.sshUser || "root";
  if (remoteAgentRoleTimers) remoteAgentRoleTimers.checked = !!snap.roleTimers;
  if (remoteAgentRoleOutbox) remoteAgentRoleOutbox.checked = !!snap.roleOutbox;
  if (remoteAgentRoleWebhook) remoteAgentRoleWebhook.checked = !!snap.roleWebhookWake;
  if (remoteAgentRoleTelegram) remoteAgentRoleTelegram.checked = false;
  if (remoteAgentConflict) remoteAgentConflict.value = snap.conflictPolicy || "prefer_local";
  if (remoteAgentSecret) remoteAgentSecret.value = "";
  if (remoteAgentSecretStatus) {
    remoteAgentSecretStatus.textContent = snap.hasSecret ? "Secret: saved" : "Secret: not set";
  }
  if (remoteAgentEnabled) remoteAgentEnabled.checked = !!snap.enabled;
  if (remoteAgentOauthMirror) remoteAgentOauthMirror.checked = !!snap.oauthMirror;
  applyTimersRunOnAvailability(!!snap.hasEnabledCompanion);
  renderRemoteAgentSubnav(snap);
}

function renderRemoteAgentSubnav(snap) {
  const list = document.querySelector("#remote-agent-sub-list");
  if (!list) return;
  list.innerHTML = "";
  for (const row of snap.agents || []) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-chip";
    btn.setAttribute("role", "option");
    btn.setAttribute("aria-selected", row.id === (snap.selectedId || "") ? "true" : "false");
    const title = document.createElement("span");
    title.className = "profile-chip-label";
    title.textContent = (row.name || row.id) + (row.enabled ? "" : " · off");
    btn.appendChild(title);
    btn.addEventListener("click", () => refreshRemoteAgent(row.id));
    li.appendChild(btn);
    list.appendChild(li);
  }
}

async function refreshRemoteAgent(selectedId) {
  try {
    const args = {};
    if (selectedId) args.selectedId = selectedId;
    applyRemoteAgentSnapshot(await invoke("remote_agent_snapshot", args), "");
  } catch (error) {
    showRemoteAgentError(error);
  }
}

async function saveRemoteAgent() {
  if (remoteAgentStatus) remoteAgentStatus.textContent = "Saving…";
  if (remoteAgentError) remoteAgentError.textContent = "";
  try {
    const args = {
      id: remoteAgentId ? remoteAgentId.value : "",
      name: remoteAgentName ? remoteAgentName.value : "",
      tailscaleHostname: remoteAgentHostname ? remoteAgentHostname.value : "",
      sshUser: remoteAgentSshUser ? remoteAgentSshUser.value : "root",
      roleTimers: remoteAgentRoleTimers ? !!remoteAgentRoleTimers.checked : true,
      roleOutbox: remoteAgentRoleOutbox ? !!remoteAgentRoleOutbox.checked : true,
      roleWebhookWake: remoteAgentRoleWebhook ? !!remoteAgentRoleWebhook.checked : false,
      roleTelegramOwner: false,
      conflictPolicy: remoteAgentConflict ? remoteAgentConflict.value : "prefer_local",
      enabled: remoteAgentEnabled ? !!remoteAgentEnabled.checked : false,
      oauthMirror: !!(remoteAgentOauthMirror && remoteAgentOauthMirror.checked),
      secret: remoteAgentSecret && remoteAgentSecret.value ? remoteAgentSecret.value : null,
      clearSecret: false,
    };
    applyRemoteAgentSnapshot(await invoke("remote_agent_save", { args }), "Saved.");
  } catch (error) {
    showRemoteAgentError(error);
  }
}

if (remoteAgentSave) remoteAgentSave.addEventListener("click", () => saveRemoteAgent());
if (remoteAgentTest) {
  remoteAgentTest.addEventListener("click", async () => {
    if (remoteAgentStatus) remoteAgentStatus.textContent = "Testing…";
    if (remoteAgentError) remoteAgentError.textContent = "";
    remoteAgentTest.disabled = true;
    try {
      applyRemoteAgentSnapshot(
        await invoke("remote_agent_test", { agentId: remoteAgentId ? remoteAgentId.value : null }),
        "",
      );
    } catch (error) {
      showRemoteAgentError(error);
      if (remoteAgentStatus) remoteAgentStatus.textContent = "Test failed.";
    } finally {
      remoteAgentTest.disabled = false;
    }
  });
}

if (remoteAgentInstall) {
  remoteAgentInstall.addEventListener("click", async () => {
    if (remoteAgentStatus) remoteAgentStatus.textContent = "Starting…";
    if (remoteAgentError) remoteAgentError.textContent = "";
    remoteAgentInstall.disabled = true;
    try {
      applyRemoteAgentSnapshot(
        await invoke("remote_agent_install", { agentId: remoteAgentId ? remoteAgentId.value : null }),
        "",
      );
    } catch (error) {
      showRemoteAgentError(error);
      if (remoteAgentStatus) remoteAgentStatus.textContent = "Install failed.";
    } finally {
      remoteAgentInstall.disabled = false;
    }
  });
}

if (remoteAgentClearSecret) {
  remoteAgentClearSecret.addEventListener("click", async () => {
    try {
      applyRemoteAgentSnapshot(
        await invoke("remote_agent_clear_secret", { agentId: remoteAgentId ? remoteAgentId.value : "" }),
        "Secret cleared.",
      );
    } catch (error) {
      showRemoteAgentError(error);
    }
  });
}
if (remoteAgentDelete) {
  remoteAgentDelete.addEventListener("click", async () => {
    try {
      applyRemoteAgentSnapshot(
        await invoke("remote_agent_delete", { agentId: remoteAgentId ? remoteAgentId.value : "" }),
        "Deleted.",
      );
    } catch (error) {
      showRemoteAgentError(error);
    }
  });
}
if (remoteAgentSubNew) {
  remoteAgentSubNew.addEventListener("click", async () => {
    try {
      applyRemoteAgentSnapshot(await invoke("remote_agent_add"), "Draft created — fill hostname and Save.");
    } catch (error) {
      showRemoteAgentError(error);
    }
  });
}

/* ---- MCP ---- */
const mcpSubnav = document.querySelector("#mcp-subnav");
const mcpStorage = document.querySelector("#mcp-storage");
const mcpStatus = document.querySelector("#mcp-status");
const mcpError = document.querySelector("#mcp-error");
const mcpId = document.querySelector("#mcp-id");
const mcpLabel = document.querySelector("#mcp-label");
const mcpEnabled = document.querySelector("#mcp-enabled");
const mcpTransport = document.querySelector("#mcp-transport");
const mcpCommand = document.querySelector("#mcp-command");
const mcpArgs = document.querySelector("#mcp-args");
const mcpUrl = document.querySelector("#mcp-url");
const mcpAuthHeader = document.querySelector("#mcp-auth-header");
const mcpPermission = document.querySelector("#mcp-permission");
const mcpSecret = document.querySelector("#mcp-secret");
const mcpSecretStatus = document.querySelector("#mcp-secret-status");
const mcpSave = document.querySelector("#mcp-save");
const mcpClearSecret = document.querySelector("#mcp-clear-secret");
const mcpDelete = document.querySelector("#mcp-delete");
const mcpSubNew = document.querySelector("#mcp-sub-new");
let mcpSnap = null;

function showMcpError(error) {
  if (!mcpError) return;
  mcpError.textContent =
    typeof error === "string" ? error : error && error.message ? error.message : "request failed";
}

function applyMcpSnapshot(snap, statusText) {
  mcpSnap = snap;
  if (mcpError) mcpError.textContent = "";
  if (mcpStatus) mcpStatus.textContent = statusText || "";
  if (mcpStorage) {
    mcpStorage.textContent =
      "Storage: " + (snap.storageBackend || "—") + (snap.storageMessage ? " — " + snap.storageMessage : "");
  }
  if (mcpId) mcpId.value = snap.id || "";
  if (mcpLabel) mcpLabel.value = snap.label || "";
  if (mcpEnabled) mcpEnabled.checked = !!snap.enabled;
  if (mcpTransport) mcpTransport.value = snap.transport || "stdio";
  if (mcpCommand) mcpCommand.value = snap.command || "";
  if (mcpArgs) mcpArgs.value = snap.argsText || "";
  if (mcpUrl) mcpUrl.value = snap.url || "";
  if (mcpAuthHeader) mcpAuthHeader.value = snap.authHeaderName || "Authorization";
  if (mcpPermission) mcpPermission.value = snap.permission || "ask";
  if (mcpSecret) mcpSecret.value = "";
  if (mcpSecretStatus) {
    mcpSecretStatus.textContent = snap.hasSecret ? "Secret: saved" : "Secret: not set";
  }
  renderMcpSubnav(snap);
}

function renderMcpSubnav(snap) {
  const list = document.querySelector("#mcp-sub-list");
  if (!list) return;
  list.innerHTML = "";
  for (const row of snap.servers || []) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-chip";
    btn.setAttribute("role", "option");
    btn.setAttribute("aria-selected", row.id === (snap.selectedId || "") ? "true" : "false");
    const title = document.createElement("span");
    title.className = "profile-chip-label";
    title.textContent = (row.label || row.id) + (row.enabled ? "" : " · off");
    btn.appendChild(title);
    btn.addEventListener("click", () => refreshMcp(row.id));
    li.appendChild(btn);
    list.appendChild(li);
  }
}

async function refreshMcp(selectedId) {
  try {
    const args = {};
    if (selectedId) args.selectedId = selectedId;
    applyMcpSnapshot(await invoke("mcp_snapshot", args), "");
  } catch (error) {
    showMcpError(error);
  }
}

async function saveMcp() {
  try {
    const args = {
      id: mcpId ? mcpId.value : "",
      label: mcpLabel ? mcpLabel.value : "",
      enabled: mcpEnabled ? !!mcpEnabled.checked : false,
      transport: mcpTransport ? mcpTransport.value : "stdio",
      command: mcpCommand ? mcpCommand.value : "",
      argsText: mcpArgs ? mcpArgs.value : "",
      url: mcpUrl ? mcpUrl.value : "",
      authHeaderName: mcpAuthHeader ? mcpAuthHeader.value : "Authorization",
      permission: mcpPermission ? mcpPermission.value : "ask",
      secret: mcpSecret && mcpSecret.value ? mcpSecret.value : null,
      clearSecret: false,
    };
    applyMcpSnapshot(await invoke("mcp_save", { args }), "Saved. Awake /refresh to rediscover tools.");
  } catch (error) {
    showMcpError(error);
  }
}

if (mcpSave) mcpSave.addEventListener("click", () => saveMcp());
if (mcpClearSecret) {
  mcpClearSecret.addEventListener("click", async () => {
    try {
      applyMcpSnapshot(
        await invoke("mcp_clear_secret", { serverId: mcpSnap ? mcpSnap.id : "" }),
        "Secret cleared."
      );
    } catch (error) {
      showMcpError(error);
    }
  });
}
if (mcpDelete) {
  mcpDelete.addEventListener("click", async () => {
    if (!mcpSnap || !mcpSnap.id) return;
    try {
      applyMcpSnapshot(await invoke("mcp_delete", { serverId: mcpSnap.id }), "Deleted.");
    } catch (error) {
      showMcpError(error);
    }
  });
}
if (mcpSubNew) {
  mcpSubNew.addEventListener("click", async () => {
    try {
      applyMcpSnapshot(await invoke("mcp_add_server"), "Added.");
    } catch (error) {
      showMcpError(error);
    }
  });
}



let roomsSnap = null;
let roomsComposingNew = false;

function roomsMembersFromInput() {
  return (document.querySelector("#room-members")?.value || "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

function renderRoomsSubnav(snap) {
  const list = document.querySelector("#rooms-sub-list");
  if (!list) return;
  list.innerHTML = "";
  for (const room of snap.rooms || []) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "profile-chip";
    btn.setAttribute("role", "option");
    btn.setAttribute(
      "aria-selected",
      !roomsComposingNew && room.id === (snap.selected_id || "") ? "true" : "false"
    );
    const title = document.createElement("span");
    title.className = "profile-chip-label";
    title.textContent = room.title || room.id;
    btn.appendChild(title);
    btn.addEventListener("click", () => {
      roomsComposingNew = false;
      refreshRooms(room.id);
    });
    li.appendChild(btn);
    list.appendChild(li);
  }
}

function renderRoomsChatLog(snap) {
  const logEl = document.querySelector("#rooms-chat-log");
  if (!logEl) return;
  logEl.innerHTML = "";
  const linesArr = snap.log || [];
  if (!linesArr.length) {
    logEl.textContent = roomsComposingNew
      ? "New room — save admin fields first, then chat."
      : "No messages yet. Send as operator to kick the room.";
    return;
  }
  for (const line of linesArr) {
    const row = document.createElement("div");
    const kind = line.kind || "say";
    row.className =
      "rooms-chat-line " +
      kind +
      (line.profile_id === "operator" ? " operator" : "");
    const who = document.createElement("span");
    who.className = "who";
    const label = line.name || line.profile_id || "?";
    const phase = line.phase ? ` ${line.phase}` : "";
    const iter = line.iteration != null ? `#${line.iteration}` : "";
    who.textContent = `[${kind}${iter}${phase}] ${label}`;
    row.appendChild(who);
    row.appendChild(document.createTextNode(": " + (line.text || "")));
    logEl.appendChild(row);
  }
  logEl.scrollTop = logEl.scrollHeight;
}

function applyRoomsSnapshot(snap, statusText) {
  roomsSnap = snap;
  if (snap && snap.composing_new) roomsComposingNew = true;
  const pathEl = document.querySelector("#rooms-path");
  if (pathEl) pathEl.textContent = "Path: " + (snap.config_hint || "");
  const list = document.querySelector("#rooms-list");
  if (list) {
    list.innerHTML = "";
    for (const room of snap.rooms || []) {
      const opt = document.createElement("option");
      opt.value = room.id;
      opt.textContent =
        (room.title || room.id) + " (" + (room.members || []).join(", ") + ")";
      if (!roomsComposingNew && snap.selected_id && room.id === snap.selected_id) {
        opt.selected = true;
      }
      list.appendChild(opt);
    }
    if (roomsComposingNew) list.selectedIndex = -1;
  }
  renderRoomsSubnav(snap);
  const selected = (snap.rooms || []).find((r) => r.id === snap.selected_id);
  const idEl = document.querySelector("#room-id");
  const titleEl = document.querySelector("#room-title");
  const membersEl = document.querySelector("#room-members");
  if (roomsComposingNew) {
    if (idEl) idEl.readOnly = false;
  } else if (selected) {
    if (idEl) {
      idEl.value = selected.id;
      idEl.readOnly = true;
    }
    if (titleEl) titleEl.value = selected.title || "";
    if (membersEl) membersEl.value = (selected.members || []).join(", ");
  } else {
    if (idEl) {
      idEl.value = "";
      idEl.readOnly = false;
    }
    if (titleEl) titleEl.value = "";
    if (membersEl) membersEl.value = "";
  }
  renderRoomsChatLog(snap);
  const status = document.querySelector("#rooms-status");
  if (status) status.textContent = statusText || "";
  const err = document.querySelector("#rooms-error");
  if (err) err.textContent = "";
}

async function refreshRooms(selectedId, statusText) {
  try {
    const args = {};
    if (selectedId) args.selectedId = selectedId;
    const snap = await invoke("rooms_snapshot", args);
    roomsComposingNew = false;
    applyRoomsSnapshot(snap, statusText || "");
  } catch (error) {
    const err = document.querySelector("#rooms-error");
    if (err) err.textContent = errorText(error);
  }
}

function beginRoomsNew() {
  roomsComposingNew = true;
  if (roomsSnap) {
    roomsSnap = Object.assign({}, roomsSnap, {
      selected_id: null,
      log: [],
      composing_new: true,
    });
    applyRoomsSnapshot(roomsSnap, "New room — set id/title/members, then Save.");
  }
  const idEl = document.querySelector("#room-id");
  const titleEl = document.querySelector("#room-title");
  const membersEl = document.querySelector("#room-members");
  const list = document.querySelector("#rooms-list");
  if (list) list.selectedIndex = -1;
  if (idEl) {
    idEl.value = "";
    idEl.readOnly = false;
    idEl.focus();
  }
  if (titleEl) titleEl.value = "";
  if (membersEl) membersEl.value = "";
  const chat = document.querySelector("#rooms-chat-log");
  if (chat) chat.textContent = "New room — save admin fields first, then chat.";
  const status = document.querySelector("#rooms-status");
  if (status) status.textContent = "New room — set id/title/members, then Save.";
  const err = document.querySelector("#rooms-error");
  if (err) err.textContent = "";
  const admin = document.querySelector("#rooms-admin");
  if (admin) admin.open = true;
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
  setSubnavVisible(timersSubnav, name === "timers");
  setSubnavVisible(skillsSubnav, name === "skills");
  setSubnavVisible(messengersSubnav, name === "messengers");
  setSubnavVisible(mcpSubnav, name === "mcp");
  setSubnavVisible(remoteAgentSubnav, name === "remote-agent");
  if (name === "profiles") {
    if (!profilesLoaded) {
      loadProfiles(selectedProfileId);
    } else {
      setProfilesSubnavVisible(true);
    }
  }
  if (name === "global") {
    loadGlobalDocs();
  }
  if (name === "email") {
    refreshEmail();
  }
  if (name === "tools") {
    refreshTools();
  }
  if (name === "rooms") {
    setSubnavVisible(document.querySelector("#rooms-subnav"), true);
    if (!roomsComposingNew) {
      refreshRooms(roomsSnap && roomsSnap.selected_id ? roomsSnap.selected_id : null);
    }
  } else {
    setSubnavVisible(document.querySelector("#rooms-subnav"), false);
  }
  if (name === "skills") {
    refreshSkills();
  }
  if (name === "timers") {
    refreshTimers();
  }
  if (name === "messengers") {
    refreshMessengers(null, messengersChannel || "telegram");
  }
  if (name === "mcp") {
    refreshMcp(mcpSnap ? mcpSnap.selectedId : null);
  }
  if (name === "remote-agent") {
    refreshRemoteAgent(remoteAgentSnap ? remoteAgentSnap.selectedId : null);
  }

}

for (const pane of panes) {
  document.querySelector(`#nav-${pane}`).addEventListener("click", () => {
    showPane(pane);
  });
}

applyTextSize("x-small");
refreshUiPrefs();

// Expandable left-nav lists replace the in-pane <select> lists for Timers/Skills.
(function hideLegacyLists() {
  const timersField = timersList && timersList.closest("label.field");
  if (timersField) timersField.hidden = true;
  const skillsField = skillsList && skillsList.closest("label.field");
  if (skillsField) skillsField.hidden = true;
})();


document.querySelector("#profile-allow-all")?.addEventListener("change", async (ev) => {
  if (!selectedProfileId) return;
  try {
    applyProfilesSnapshot(
      await invoke("profile_set_allow_all", {
        id: selectedProfileId,
        allowAll: !!ev.target.checked,
      }),
      ev.target.checked ? "Allow all enabled for this profile." : "Allow all disabled."
    );
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
});
document.querySelector("#profile-role")?.addEventListener("change", async (ev) => {
  if (!selectedProfileId) return;
  try {
    applyProfilesSnapshot(
      await invoke("profile_set_role", { id: selectedProfileId, role: ev.target.value }),
      "Role saved."
    );
  } catch (error) {
    packErrorEl.textContent = errorText(error);
  }
});
document.querySelector("#rooms-list")?.addEventListener("change", (ev) => {
  roomsComposingNew = false;
  refreshRooms(ev.target.value);
});
document.querySelector("#rooms-new")?.addEventListener("click", () => beginRoomsNew());
document.querySelector("#rooms-sub-new")?.addEventListener("click", () => beginRoomsNew());
document.querySelector("#rooms-save")?.addEventListener("click", async () => {
  const id = (document.querySelector("#room-id")?.value || "").trim();
  const title = (document.querySelector("#room-title")?.value || "").trim();
  const members = roomsMembersFromInput();
  try {
    const snap = await invoke("room_save", { id, title, members });
    roomsComposingNew = false;
    applyRoomsSnapshot(snap, "Room saved.");
  } catch (error) {
    const err = document.querySelector("#rooms-error");
    if (err) err.textContent = errorText(error);
  }
});
document.querySelector("#rooms-delete")?.addEventListener("click", async () => {
  const id = (document.querySelector("#room-id")?.value || "").trim();
  try {
    roomsComposingNew = false;
    applyRoomsSnapshot(await invoke("room_delete", { id }), "Room deleted.");
  } catch (error) {
    const err = document.querySelector("#rooms-error");
    if (err) err.textContent = errorText(error);
  }
});
document.querySelector("#rooms-send")?.addEventListener("click", async () => {
  const roomId =
    (roomsSnap && roomsSnap.selected_id) ||
    (document.querySelector("#rooms-list")?.value || "").trim();
  const composeText = (document.querySelector("#rooms-compose")?.value || "").trim();
  try {
    const snap = await invoke("room_post", { roomId, text: composeText });
    roomsComposingNew = false;
    applyRoomsSnapshot(snap, "Posted.");
    const compose = document.querySelector("#rooms-compose");
    if (compose) compose.value = "";
  } catch (error) {
    const err = document.querySelector("#rooms-error");
    if (err) err.textContent = errorText(error);
  }
});
document.querySelector("#rooms-refresh")?.addEventListener("click", () => {
  const id =
    (roomsSnap && roomsSnap.selected_id) ||
    (document.querySelector("#rooms-list")?.value || null);
  refreshRooms(id, "Refreshed.");
});


showPane("status");
refresh();
setInterval(refresh, 1000);
refreshProviders();
refreshEmail();


/* --- Chat lock (HUD history passphrase vault) --- */
const chatLockStatus = document.querySelector("#chat-lock-status");
const chatLockWarning = document.querySelector("#chat-lock-warning");
const chatLockPass = document.querySelector("#chat-lock-pass");
const chatLockKeyring = document.querySelector("#chat-lock-keyring");
const chatLockSet = document.querySelector("#chat-lock-set");
const chatLockLock = document.querySelector("#chat-lock-lock");
const chatLockError = document.querySelector("#chat-lock-error");

function applyChatLockStatus(snap) {
  if (!chatLockStatus || !snap) {
    return;
  }
  const mode = snap.mode || "unset";
  const unlocked = !!snap.unlocked;
  const parts = ["Mode: " + mode];
  if (mode === "passphrase") {
    parts.push(unlocked ? "unlocked" : "locked");
  }
  if (snap.keyring_wrap) {
    parts.push(snap.keyring_available ? "keyring wrap on" : "keyring wrap on (unavailable)");
  }
  chatLockStatus.textContent = parts.join(" · ");
  if (chatLockWarning) {
    chatLockWarning.hidden = !(snap.plaintext_warning && mode !== "passphrase");
  }
  if (chatLockKeyring) {
    chatLockKeyring.checked = !!snap.keyring_wrap || mode === "unset";
    chatLockKeyring.disabled = mode === "unset";
  }
  if (chatLockError) {
    chatLockError.textContent = "";
  }
}

async function refreshChatLock() {
  if (!chatLockStatus) {
    return;
  }
  try {
    const snap = await invoke("ui_vault_status");
    applyChatLockStatus(snap);
  } catch (error) {
    if (chatLockError) {
      chatLockError.textContent = errorText(error, "could not read chat lock");
    }
  }
}

if (chatLockSet) {
  chatLockSet.addEventListener("click", () => {
    const passphrase = chatLockPass ? chatLockPass.value : "";
    const keyringWrap = chatLockKeyring ? !!chatLockKeyring.checked : false;
    invoke("ui_vault_set_passphrase", { passphrase, keyringWrap })
      .then((snap) => {
        applyChatLockStatus(snap);
        if (chatLockPass) {
          chatLockPass.value = "";
        }
      })
      .catch((error) => {
        if (chatLockError) {
          chatLockError.textContent = errorText(error, "could not set passphrase");
        }
      });
  });
}

if (chatLockLock) {
  chatLockLock.addEventListener("click", () => {
    invoke("ui_vault_lock")
      .then((snap) => applyChatLockStatus(snap))
      .catch((error) => {
        if (chatLockError) {
          chatLockError.textContent = errorText(error, "could not lock");
        }
      });
  });
}

if (chatLockKeyring) {
  chatLockKeyring.addEventListener("change", () => {
    const enabled = !!chatLockKeyring.checked;
    invoke("ui_vault_set_keyring_wrap", { enabled })
      .then((snap) => applyChatLockStatus(snap))
      .catch((error) => {
        if (chatLockError) {
          chatLockError.textContent = errorText(error, "could not update keyring wrap");
        }
        void refreshChatLock();
      });
  });
}

void refreshChatLock();

async function refreshAppBuild() {
  const el = document.querySelector("#app-build");
  if (!el) {
    return;
  }
  try {
    const info = await invoke("app_build_info");
    let line = "UI " + (info.label || info.version);
    try {
      const st = await invoke("status");
      if (st && st.build) {
        line += " · daemon " + st.build;
      }
    } catch (_daemonErr) {
      // daemon optional for UI stamp
    }
    el.textContent = "Build: " + line;
    el.title = line;
  } catch (error) {
    el.textContent = "Build: (unavailable)";
  }
}

void refreshAppBuild();
