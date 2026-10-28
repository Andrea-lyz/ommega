
// Global boot-time declarations. Saving never changes the running framework.
let ommegaCapabilityState = null, ommegaCapabilityDraft = null;
let ommegaCapabilityBusy = false, ommegaCapabilitySaving = false, ommegaCapabilitySession = 0;

function ommegaCapabilityValidate(config) {
  if (!config || typeof config.strongbox !== "boolean" ||
      typeof config.device_id_attestation !== "boolean") throw new Error("Invalid system capability settings");
  return config;
}

function ommegaCapabilityText(config) {
  ommegaCapabilityValidate(config);
  return "version=1\nstrongbox=" + config.strongbox +
    "\ndevice_id_attestation=" + config.device_id_attestation + "\n";
}

function ommegaCapabilityCommand(command, payload = null) {
  if (!["status", "write"].includes(command) || (command === "write") !== (payload !== null))
    throw new Error("Unsupported system capability command");
  const lines = ['set -e',
    'ommega_capability_writer="/data/adb/modules/ommega/libs/arm64-v8a/ommega-capability-policy"',
    'if [ ! -x "$ommega_capability_writer" ]; then ommega_capability_writer="/data/adb/modules/ommega/libs/x86_64/ommega-capability-policy"; fi',
    '[ -x "$ommega_capability_writer" ] || { echo "System capability writer unavailable; install the complete A module" >&2; exit 1; }'];
  lines.push(command === "status" ? '"$ommega_capability_writer" status' :
    "printf '%s' '" + ommegaSoterRemoteBase64(JSON.stringify(payload)) + "' | base64 -d | \"$ommega_capability_writer\" write");
  return lines.join("\n");
}

async function ommegaCapabilityRead() {
  const response = await ommegaReadOnly(ommegaCapabilityCommand("status"));
  if (Number(response.errno) !== 0) throw new Error(response.stderr || "Could not read system capability settings");
  const state = JSON.parse(response.stdout);
  ommegaCapabilityValidate(state.config);
  if (state.mounted !== null) ommegaCapabilityValidate(state.mounted);
  if (state.original !== null && typeof state.original !== "string" ||
      state.mount_error !== null && typeof state.mount_error !== "string") throw new Error("Invalid system capability response");
  return state;
}

function ommegaCapabilityStatus(message) {
  const status = document.getElementById("ommega-capability-status");
  if (status) status.textContent = message;
}

function ommegaCapabilitySync() {
  const get = id => document.getElementById(id), dialog = get("ommega-capability-dialog");
  if (!dialog) return;
  dialog.setAttribute("aria-busy", String(ommegaCapabilityBusy));
  for (const control of dialog.querySelectorAll("button,input")) {
    control.disabled = control.id === "ommega-capability-back" ? ommegaCapabilitySaving :
      ommegaCapabilityBusy || !ommegaCapabilityDraft && control.id !== "ommega-capability-reload";
  }
  if (ommegaCapabilityDraft) {
    get("ommega-capability-strongbox").checked = ommegaCapabilityDraft.strongbox;
    get("ommega-capability-device-id").checked = ommegaCapabilityDraft.device_id_attestation;
  }
  if (ommegaCapabilityState) {
    const count = Number(ommegaCapabilityState.config.strongbox) + Number(ommegaCapabilityState.config.device_id_attestation);
    get("ommega-capability-summary").textContent = count ?
      ommegaUiLocalize("已选屏蔽项 / Selected masks") + ": " + count + " / 2" :
      ommegaUiLocalize("未启用屏蔽 / No masks enabled");
    get("ommega-capability-mount-note").textContent = ommegaCapabilityState.mount_error ||
      (ommegaCapabilityState.mounted === null ? ommegaUiLocalize(
        "系统配置尚未挂载。请确认模块管理器已启用系统挂载支持，并重启设备。 / System configuration is not mounted. Enable system file mounting in your module manager and restart the device.") : "");
  }
}

async function ommegaCapabilityRefresh() {
  if (ommegaCapabilityBusy) return;
  const session = ++ommegaCapabilitySession;
  ommegaCapabilityBusy = true; ommegaCapabilitySync();
  ommegaCapabilityStatus(ommegaUiLocalize("正在读取… / Loading…"));
  try {
    const state = await ommegaCapabilityRead();
    if (session !== ommegaCapabilitySession) return;
    ommegaCapabilityState = state; ommegaCapabilityDraft = {...state.config};
    ommegaCapabilityStatus("");
  } catch (error) {
    if (session === ommegaCapabilitySession) {
      ommegaCapabilityState = null; ommegaCapabilityDraft = null;
      ommegaCapabilityStatus(ommegaUiLocalize("读取失败，请重试 / Could not load settings; retry") + ": " + error.message);
      document.getElementById("ommega-capability-summary").textContent = ommegaUiLocalize("状态不可用 / Status unavailable");
    }
  } finally {
    if (session === ommegaCapabilitySession) { ommegaCapabilityBusy = false; ommegaCapabilitySync(); }
  }
}

async function ommegaCapabilityOpen() {
  const dialog = document.getElementById("ommega-capability-dialog");
  if (dialog.open || ommegaCapabilitySaving) return;
  dialog.showModal();
  await ommegaCapabilityRefresh();
  if (dialog.open) document.getElementById(ommegaCapabilityDraft ? "ommega-capability-strongbox" : "ommega-capability-reload").focus();
}

async function ommegaCapabilitySave() {
  if (ommegaCapabilityBusy || !ommegaCapabilityState || !ommegaCapabilityDraft) return;
  const config = {...ommegaCapabilityDraft};
  ommegaCapabilityBusy = true; ommegaCapabilitySaving = true; ommegaCapabilitySync();
  ommegaCapabilityStatus(ommegaUiLocalize("正在保存… / Saving…"));
  try {
    const response = await _(ommegaCapabilityCommand("write", {
      expected:ommegaCapabilityState.original, replacement:ommegaCapabilityText(config)
    }));
    if (Number(response.errno) !== 0) throw new Error(response.stderr || "Could not save system capability settings");
    const state = await ommegaCapabilityRead();
    if (ommegaCapabilityText(state.config) !== ommegaCapabilityText(config)) throw new Error("System capability readback did not match");
    ommegaCapabilityState = state; ommegaCapabilityDraft = {...state.config};
    ommegaCapabilityStatus(ommegaUiLocalize("已保存，重启设备后生效。 / Saved. Restart the device to apply these settings."));
  } catch (error) {
    ommegaCapabilityStatus(ommegaUiLocalize("保存失败，请重新读取后重试 / Could not save; reload and retry") + ": " + error.message);
  } finally { ommegaCapabilityBusy = false; ommegaCapabilitySaving = false; ommegaCapabilitySync(); }
}

function ommegaInstallCapabilityMask() {
  const slot = document.getElementById("om-capability-mask-slot");
  if (!slot || typeof slot.appendChild !== "function" || document.getElementById("ommega-capability-mask")) return false;
  const panel = document.createElement("section"); panel.id = "ommega-capability-mask";
  const back = '<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="m14 5-7 7 7 7"/></svg>';
  const forward = '<svg class="icon" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="m9 5 7 7-7 7"/></svg>';
  const shield = '<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 2.8 19 5.5v5.7c0 4.4-2.6 7.8-7 10-4.4-2.2-7-5.6-7-10V5.5L12 2.8Z"/><path d="m9 12 2 2 4-4"/></svg>';
  panel.innerHTML = ommegaUiLocalizeHtml(
    '<button type="button" id="ommega-capability-open" class="action-row"><span class="row-icon">' + shield + '</span><span class="row-text"><strong>系统能力屏蔽 / System capability mask</strong><small id="ommega-capability-summary">正在读取… / Loading…</small></span>' + forward + '</button>' +
    '<dialog id="ommega-capability-dialog" class="ommega-auth-modal" aria-labelledby="ommega-capability-heading" aria-describedby="ommega-capability-note" data-om-return-focus="ommega-capability-open">' +
    '<header class="ommega-auth-toolbar"><button type="button" id="ommega-capability-back" class="om-button secondary">' + back + '<span>返回 / Back</span></button><h2 id="ommega-capability-heading">系统能力屏蔽 / System capability mask</h2></header>' +
    '<div class="ommega-auth-policy-content"><p id="ommega-capability-note" class="ommega-auth-note">开启开关以隐藏对应能力声明，对所有应用生效。保存后需重启设备。 / Turn on a switch to hide that capability from all apps. Restart the device after saving.</p>' +
    '<label class="ommega-capability-row"><span><strong>屏蔽 StrongBox / Mask StrongBox</strong><small id="ommega-capability-strongbox-id" class="ommega-capability-id">android.hardware.strongbox_keystore</small></span><input type="checkbox" role="switch" class="ommega-capability-switch" id="ommega-capability-strongbox" aria-describedby="ommega-capability-strongbox-id"></label>' +
    '<label class="ommega-capability-row"><span><strong>屏蔽设备 ID 证明 / Mask device ID attestation</strong><small id="ommega-capability-device-id-name" class="ommega-capability-id">android.software.device_id_attestation</small></span><input type="checkbox" role="switch" class="ommega-capability-switch" id="ommega-capability-device-id" aria-describedby="ommega-capability-device-id-name"></label>' +
    '<p id="ommega-capability-mount-note" class="ommega-auth-note"></p>' +
    '<p class="ommega-auth-note">若已启用独立的能力屏蔽模块，请先停用，再使用此处控制。 / Disable any standalone capability mask module before using these controls.</p>' +
    '<div id="ommega-capability-status" role="status" aria-live="polite" aria-atomic="true"></div>' +
    '<div class="ommega-auth-actions"><button type="button" id="ommega-capability-reload" class="om-button secondary">重新读取 / Reload</button><button type="button" id="ommega-capability-save" class="om-button">保存设置 / Save settings</button></div></div></dialog>');
  slot.appendChild(panel);
  const get = id => document.getElementById(id), dialog = get("ommega-capability-dialog");
  get("ommega-capability-open").addEventListener("click", ommegaCapabilityOpen);
  get("ommega-capability-reload").addEventListener("click", ommegaCapabilityRefresh);
  get("ommega-capability-save").addEventListener("click", ommegaCapabilitySave);
  get("ommega-capability-back").addEventListener("click", () => dialog.close());
  dialog.addEventListener("cancel", event => { if (ommegaCapabilitySaving) event.preventDefault(); });
  dialog.addEventListener("close", () => {
    if (!ommegaCapabilitySaving) {
      ++ommegaCapabilitySession; ommegaCapabilityBusy = false; ommegaCapabilityDraft = null;
    }
    get("ommega-capability-open").focus();
  });
  for (const [id, key] of [["strongbox", "strongbox"], ["device-id", "device_id_attestation"]]) {
    get("ommega-capability-" + id).addEventListener("change", event => {
      if (ommegaCapabilityDraft) ommegaCapabilityDraft[key] = event.target.checked;
      ommegaCapabilityStatus(ommegaUiLocalize("设置尚未保存 / Settings not saved yet"));
    });
  }
  document.addEventListener("ommega-app-changes", event => {
    if (dialog.open && ommegaCapabilityDraft && ommegaCapabilityState &&
        ommegaCapabilityText(ommegaCapabilityDraft) !== ommegaCapabilityText(ommegaCapabilityState.config)) event.detail.dirty = true;
  });
  ommegaCapabilityRefresh();
  return true;
}

ommegaInstallCapabilityMask();
if (typeof window !== "undefined") window.addEventListener("ommega-settings-ready", ommegaInstallCapabilityMask);
