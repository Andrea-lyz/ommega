
// Per-app settings are an adapter around the existing A-side policy writers.
// Scope, security mode, auth signing and CPU scope remain separate policies.
const OMMEGA_APP_TARGET = "/data/adb/ommega/ommegadata/target.txt";
const OMMEGA_SYSTEM_APPS = "/data/adb/ommega/ommegadata/system_app";
let ommegaTargetSnapshot = undefined;
let ommegaTargetSnapshotGeneration = 0;
let ommegaDetail = null;
let ommegaDetailSession = 0;
let ommegaAppScopeSaving = false;
let ommegaCertificateCounts = null;
let ommegaCountsBusy = false;

async function ommegaReadAppFile(path) {
  if (![OMMEGA_APP_TARGET, OMMEGA_TARGET_SECURITY, OMMEGA_SYSTEM_APPS].includes(path)) throw new Error("Unsupported app policy");
  const result = await ommegaReadOnly('if [ ! -e "' + path + '" ]; then printf "M:"; elif [ -f "' + path + '" ]; then printf "F:"; base64 < "' + path + '"; else exit 1; fi');
  if (Number(result.errno) !== 0) throw new Error(result.stderr || "Read failed");
  const encoded = String(result.stdout).replace(/\s/g, "");
  if (encoded === "M:") return null;
  if (!encoded.startsWith("F:")) throw new Error("Invalid policy response");
  return new TextDecoder("utf-8", {fatal:true, ignoreBOM:true})
    .decode(Uint8Array.from(atob(encoded.slice(2)), byte => byte.charCodeAt(0)));
}

function ommegaAppFileWriteCommand(path, expected, replacement) {
  if (![OMMEGA_APP_TARGET, OMMEGA_TARGET_SECURITY, OMMEGA_SYSTEM_APPS].includes(path) ||
      (expected !== null && typeof expected !== "string") || typeof replacement !== "string") throw new Error("Invalid policy write");
  const operation = path === OMMEGA_APP_TARGET ? "write-target" : path === OMMEGA_TARGET_SECURITY ? "write-target-security" : "write-system-apps";
  const payload = ommegaSoterRemoteBase64(JSON.stringify({expected, replacement}));
  return ["set -e",
    'ommega_app_writer="/data/adb/modules/ommega/libs/arm64-v8a/ommega-auth-policy"',
    'if [ ! -x "$ommega_app_writer" ]; then ommega_app_writer="/data/adb/modules/ommega/libs/x86_64/ommega-auth-policy"; fi',
    '[ -x "$ommega_app_writer" ] || { echo "Policy writer unavailable; install the complete A module" >&2; exit 1; }',
    "printf '%s' '" + payload + "' | base64 -d | \"$ommega_app_writer\" " + operation].join("\n");
}

// The existing target-security format is intentionally narrow. Reject unknown
// TOML syntax before editing, so a newer or manually edited policy is not lost.
function ommegaEditSecurityPolicy(original, pkg, mode) {
  if (!ommegaAuthPackage(pkg) || (mode !== null && !["global_default", "strongbox", "tee"].includes(mode))) throw new Error("Invalid app mode");
  const input = original === null ? "version = 1\n\n[packages]\n" : original;
  const lines = input.split("\n"), modes = new Map();
  let section = -1, entry = -1, version = false, inPackages = false;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line || line.startsWith("#")) continue;
    if (line === "[packages]") {
      if (section !== -1) throw new Error("Duplicate packages section");
      section = i; inPackages = true; continue;
    }
    if (!inPackages) {
      if (!/^version\s*=\s*1\s*(?:#.*)?$/.test(line) || version) throw new Error("Unsupported security policy");
      version = true; continue;
    }
    const match = line.match(/^"([A-Za-z0-9_.]+)"\s*=\s*"(global_default|strongbox|tee)"\s*(?:#.*)?$/);
    if (!match || !ommegaAuthPackage(match[1]) || modes.has(match[1])) throw new Error("Unsupported security policy");
    modes.set(match[1], match[2]);
    if (match[1] === pkg) entry = i;
  }
  if (!version || section < 0) throw new Error("Incomplete security policy");
  if (mode === null) return {replacement:input, modes, changed:false};
  const old = modes.get(pkg) || "global_default";
  if (mode === old) return {replacement:input, modes, changed:false};
  const ending = input.includes("\r\n") ? "\r" : "";
  if (entry >= 0) lines.splice(entry, 1);
  if (mode === "global_default") modes.delete(pkg);
  else {
    lines.splice(section + 1, 0, '"' + pkg + '" = "' + mode + '"' + ending);
    modes.set(pkg, mode);
  }
  return {replacement:lines.join("\n"), modes, changed:true};
}

function ommegaScopePackages(contents) {
  return String(contents || "").split(/\r?\n/)
    .map(line => line.trim().replace(/[!?]+$/, ""))
    .filter(line => line && !line.startsWith("#") && !line.startsWith("["));
}

function ommegaEditAppScope(original, pkg, enabled) {
  if (!ommegaAuthPackage(pkg) || typeof enabled !== "boolean") throw new Error("Invalid app scope");
  const input = original || "", ending = input.includes("\r\n") ? "\r\n" : "\n";
  const lines = input.split(/\r?\n/);
  const matches = line => {
    const value = line.trim();
    return value && !value.startsWith("#") && !value.startsWith("[") && value.replace(/[!?]+$/, "").trim() === pkg;
  };
  const present = lines.some(matches);
  if (present === enabled) return {replacement:original, changed:false};
  if (!enabled) return {replacement:lines.filter(line => !matches(line)).join(ending), changed:true};
  if (lines.at(-1) === "") lines.pop();
  lines.push(pkg, "");
  return {replacement:lines.join(ending), changed:true};
}

function ommegaDetailText(zh, en) { return ommegaUiLocalize(zh + " / " + en); }
const ommegaDetailGlyphs = {
  apps: '<rect x="3" y="3" width="8" height="8" rx="2.5"/><rect x="13" y="3" width="8" height="8" rx="2.5"/><rect x="3" y="13" width="8" height="8" rx="2.5"/><rect x="13" y="13" width="8" height="8" rx="2.5"/>',
  shield: '<path d="M12 2.8 19 5.5v5.7c0 4.4-2.6 7.8-7 10-4.4-2.2-7-5.6-7-10V5.5L12 2.8Z"/><path d="m9 12 2 2 4-4"/>',
  lock: '<rect x="4.5" y="10" width="15" height="11" rx="3"/><path d="M8 10V7a4 4 0 0 1 8 0v3m-4 4v3"/>',
  cpu: '<rect x="6" y="6" width="12" height="12" rx="3"/><path d="M9 9h6v6H9zM9 2v4m6-4v4M9 18v4m6-4v4M2 9h4m-4 6h4m12-6h4m-4 6h4"/>',
  back: '<path d="M20 12H4m7-7-7 7 7 7"/>'
};
function ommegaDetailIcon(name) {
  return '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' + (ommegaDetailGlyphs[name] || ommegaDetailGlyphs.shield) + '</svg>';
}
function ommegaDetailRow(title, description, input) {
  return '<label class="policy-row"><span class="toggle-copy"><strong>' + ommegaEsc(title) + '</strong><small>' + ommegaEsc(description) + '</small></span><span class="switch">' + input + '<span></span></span></label>';
}
function ommegaDetailCard(icon, title, description, body) {
  const note = description ? '<p>' + ommegaEsc(description) + '</p>' : '';
  return '<section class="panel policy-card"><div class="policy-heading"><span class="row-icon">' + ommegaDetailIcon(icon) + '</span><div><h2>' + ommegaEsc(title) + '</h2>' + note + '</div></div>' + body + '</section>';
}
function ommegaDetailRadio(group, value, title, description) {
  return ommegaDetailRow(title, description, '<input type="radio" name="' + group + '" value="' + value + '" disabled>');
}
function ommegaDetailMessage(id, value, error = false) {
  const element = document.getElementById(id);
  if (element) { element.textContent = value; element.classList.toggle("error", error); }
}
function ommegaDetailCurrent(session) { return ommegaDetail && ommegaDetail.session === session; }
function ommegaDetailControls(selector, disabled) {
  document.querySelectorAll("#om-app-detail " + selector).forEach(input => { input.disabled = disabled; });
}

function ommegaCertificateCount(value) {
  if (!value || typeof value !== "object" || !Number.isSafeInteger(value.count) || value.count <= 0 || typeof value.shared !== "boolean") return null;
  return {count:value.count, shared:value.shared};
}

function ommegaUpdateAppScope(card) {
  const box = card.querySelector("md-checkbox");
  if (!box || typeof box.checked !== "boolean") return;
  let hint = card.querySelector(":scope > .app-scope-hint");
  if (!hint) {
    hint = document.createElement("span");
    hint.className = "app-scope-hint";
    const checkbox = card.querySelector(":scope > md-checkbox");
    card.insertBefore(hint, checkbox);
  }
  const global = typeof ommegaGlobalScopeEnabled !== "undefined" && ommegaGlobalScopeEnabled;
  const enabled = Boolean(box.checked);
  const full = global
    ? ommegaDetailText("全局控制", "Global scope")
    : enabled ? ommegaDetailText("作用域已启用", "Scope enabled") : ommegaDetailText("作用域未启用", "Scope disabled");
  const label = global ? ommegaDetailText("全局", "Global") : enabled ? ommegaDetailText("已启用", "On") : ommegaDetailText("未启用", "Off");
  if (hint.textContent !== label) hint.textContent = label;
  hint.title = full;
  hint.classList.toggle("active", enabled && !global);
  if (!card.querySelector(":scope > .om-app-chevron")) {
    const chevron = document.createElement("span");
    chevron.className = "om-app-chevron";
    chevron.innerHTML = '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 5 7 7-7 7"/></svg>';
    card.append(chevron);
  }
}
function ommegaUpdateAppCount(card) {
  const name = card.querySelector(".app-name"), pkg = card.getAttribute("data-package");
  if (!name || !pkg) return;
  if (!card.dataset.omOriginalName) card.dataset.omOriginalName = name.textContent;
  const count = ommegaCertificateCount(ommegaCertificateCounts?.[pkg]);
  let badge = name.querySelector(".om-cert-count");
  if (!count) { badge?.remove(); }
  else {
    if (!badge) { badge = document.createElement("span"); badge.className = "om-cert-count"; name.appendChild(badge); }
    const formatted = new Intl.NumberFormat().format(count.count);
    const label = count.shared
      ? ommegaDetailText("证书 " + formatted + " 次 · 共享 UID", "Certificates " + formatted + " · shared UID")
      : ommegaDetailText("证书 " + formatted + " 次", "Certificates " + formatted);
    if (badge.textContent !== label) badge.textContent = label;
    badge.title = count.shared
      ? ommegaDetailText("包含共享 UID 应用的成功远程证明调用", "Includes successful remote attestations from apps sharing this UID")
      : ommegaDetailText("成功的远程证明请求", "Successful remote attestation requests");
  }
  const description = ommegaDetailText("打开应用设置", "Open app settings") + ": " + card.dataset.omOriginalName;
  card.setAttribute("aria-label", count ? description + ", " + badge.title + ": " + count.count : description);
  ommegaUpdateAppScope(card);
}

async function ommegaRefreshCertificateCounts() {
  if (ommegaCountsBusy || document.hidden || document.body.dataset.omView !== "apps") return;
  ommegaCountsBusy = true;
  try {
    const state = await ommegaCpuRead();
    ommegaCertificateCounts = state.attestation_counts && typeof state.attestation_counts === "object" && !Array.isArray(state.attestation_counts)
      ? state.attestation_counts : null;
  } catch (_) { ommegaCertificateCounts = null; }
  finally {
    ommegaCountsBusy = false;
    document.querySelectorAll("#apps-list .card[data-package]").forEach(ommegaUpdateAppCount);
  }
}

async function ommegaDetailLoad(session, refreshScope = false) {
  const state = ommegaDetail;
  if (!state || state.session !== session || state.pending || ommegaAppScopeSaving || ommegaGlobalScopePending) return false;
  const revision = state.revision || 0, generation = ommegaTargetSnapshotGeneration;
  const load = state.load = (state.load || 0) + 1;
  const reads = [ommegaReadAppFile(OMMEGA_TARGET_SECURITY), ommegaAuthRead(), ommegaCpuRead()];
  if (refreshScope) reads.push(ommegaReadAppFile(OMMEGA_APP_TARGET), ommegaReadOnly('base64 < "' + OMMEGA_SCOPE_CONFIG + '"'));
  const results = await Promise.allSettled(reads);
  // A same-session save can start and finish before these reads return. Its
  // revision, not just its pending flag, protects the confirmed new values.
  if (!ommegaDetailCurrent(session) || state.load !== load || state.pending || (state.revision || 0) !== revision ||
      ommegaAppScopeSaving || ommegaGlobalScopePending || generation !== ommegaTargetSnapshotGeneration) return false;
  let ok = true;
  const failed = (id, error, controls) => {
    ok = false;
    ommegaDetailMessage(id, String(error?.message || error), true);
    if (controls) ommegaDetailControls(controls, true);
  };
  const [security, auth, cpu, target, injector] = results;
  try {
    if (security.status !== "fulfilled") throw security.reason;
    const policy = ommegaEditSecurityPolicy(security.value, state.pkg, null);
    const mode = policy.modes.get(state.pkg) || "global_default";
    document.querySelector('#om-app-detail input[name="om-app-mode"][value="' + mode + '"]').checked = true;
    ommegaDetailControls('input[name="om-app-mode"]', false);
    ommegaDetailMessage("om-detail-mode-status", "");
  } catch (error) { failed("om-detail-mode-status", error, 'input[name="om-app-mode"]'); }
  try {
    if (auth.status !== "fulfilled") throw auth.reason;
    const mode = auth.value.config.apps[state.pkg] || auth.value.config.default;
    const input = document.querySelector('#om-app-detail input[name="om-app-auth"][value="' + mode + '"]');
    if (!input) throw new Error("Invalid app authentication policy");
    input.checked = true;
    ommegaDetailControls('input[name="om-app-auth"]', false);
    const inherited = !Object.hasOwn(auth.value.config.apps, state.pkg);
    ommegaDetailMessage("om-detail-auth-status", inherited
      ? ommegaDetailText("当前继承全局默认策略；选择后单独保存。", "Currently inheriting the global default; a choice saves an app override.")
      : ommegaDetailText("仅对之后新建的密钥生效。", "Applies to newly created keys."));
  } catch (error) { failed("om-detail-auth-status", error, 'input[name="om-app-auth"]'); }
  try {
    if (cpu.status !== "fulfilled") throw cpu.reason;
    const box = document.getElementById("om-detail-cpu");
    box.checked = cpu.value.config.enabled && cpu.value.config.packages.includes(state.pkg);
    box.disabled = false;
    const needsGlobalSettings = !cpu.value.config.enabled && cpu.value.config.packages.some(pkg => pkg !== state.pkg);
    document.getElementById("om-detail-cpu-settings").hidden = !needsGlobalSettings;
    ommegaDetailMessage("om-detail-cpu-status", cpu.value.config.enabled
      ? ommegaDetailText("需要启用 Zygisk，保存后重新打开应用。", "Requires Zygisk; reopen the app after saving.")
      : ommegaDetailText("属性功能当前关闭。", "The property feature is currently off."));
  } catch (error) { failed("om-detail-cpu-status", error); document.getElementById("om-detail-cpu").disabled = true; }
  if (refreshScope) {
    const input = document.getElementById("om-detail-scope");
    try {
      if (target.status !== "fulfilled") throw target.reason;
      if (injector.status !== "fulfilled") throw injector.reason;
      if (Number(injector.value.errno) !== 0) throw new Error(injector.value.stderr || "Could not read global scope");
      const bytes = Uint8Array.from(atob(String(injector.value.stdout).replace(/\s/g, "")), byte => byte.charCodeAt(0));
      const scope = ommegaScopeSetting(new TextDecoder("utf-8", {fatal:true, ignoreBOM:true}).decode(bytes));
      ommegaGlobalScopeEnabled = scope.global; ommegaGlobalScopeKnown = true; ommegaGlobalScopeMessage = ""; ommegaScopeSync();
      // Global scope does not consume target.txt; a target edit made while it
      // is enabled must not block the detail page or its other policies.
      const dirty = !scope.global && ommegaAppsHaveChanges();
      const changed = !scope.global && target.value !== ommegaTargetSnapshot;
      // A detail read never rebases the list's saved selection or CAS snapshot.
      // Display a retained draft, or the freshly read saved value, explicitly.
      if (!dirty) input.checked = scope.global || ommegaScopePackages(target.value || "").includes(state.pkg);
      input.disabled = scope.global || dirty || changed;
      if (dirty || changed) {
        ommegaDetailMessage("om-detail-scope-status", dirty
          ? ommegaDetailText("应用列表尚未稳定，请刷新后重试。", "The app list is not settled; refresh and retry.")
          : ommegaDetailText("已读取保存的作用域；列表已变更，请先返回应用列表刷新。", "Saved scope was read; the list changed. Return to the app list and refresh first."), changed);
        if (changed) ok = false;
      } else ommegaDetailMessage("om-detail-scope-status", scope.global
        ? ommegaDetailText("全局作用域已开启，单独设置暂不生效。", "Global scope is enabled; individual scope is paused.") : "");
    } catch (error) { input.disabled = true; failed("om-detail-scope-status", error); }
  }
  return ok;
}

async function ommegaDetailSaveMode(input) {
  const state = ommegaDetail;
  if (!state || !input.checked) return;
  state.revision = (state.revision || 0) + 1;
  state.pending = (state.pending || 0) + 1;
  const session = state.session, mode = input.value;
  let readable = true;
  ommegaDetailControls('input[name="om-app-mode"]', true);
  ommegaDetailMessage("om-detail-mode-status", ommegaDetailText("正在保存…", "Saving…"));
  try {
    const original = await ommegaReadAppFile(OMMEGA_TARGET_SECURITY);
    const edited = ommegaEditSecurityPolicy(original, state.pkg, mode);
    if (edited.changed) {
      const response = await _(ommegaAppFileWriteCommand(OMMEGA_TARGET_SECURITY, original, edited.replacement));
      if (Number(response.errno) !== 0) throw new Error(response.stderr || "Save failed");
      const saved = await ommegaReadAppFile(OMMEGA_TARGET_SECURITY);
      if (saved !== edited.replacement) throw new Error("Mode readback did not match");
    }
    ommegaTargetModes = edited.modes;
    if (ommegaAppsBaseline !== null) {
      const baseline = JSON.parse(ommegaAppsBaseline);
      for (const pair of baseline) if (pair[0] === state.pkg) pair[1] = mode;
      ommegaAppsBaseline = JSON.stringify(baseline);
    }
    if (ommegaDetailCurrent(session)) ommegaDetailMessage("om-detail-mode-status", ommegaDetailText("已保存。", "Saved."));
  } catch (error) {
    if (ommegaDetailCurrent(session)) {
      ommegaDetailMessage("om-detail-mode-status", ommegaDetailText("保存失败", "Save failed") + ": " + error.message, true);
      try {
        const original = await ommegaReadAppFile(OMMEGA_TARGET_SECURITY);
        const modeNow = ommegaEditSecurityPolicy(original, state.pkg, null).modes.get(state.pkg) || "global_default";
        if (ommegaDetailCurrent(session)) document.querySelector('#om-app-detail input[name="om-app-mode"][value="' + modeNow + '"]').checked = true;
      } catch (_) { readable = false; }
    }
  } finally {
    state.pending--;
    if (ommegaDetailCurrent(session)) ommegaDetailControls('input[name="om-app-mode"]', !readable);
  }
}

async function ommegaDetailSaveScope(input) {
  const state = ommegaDetail;
  if (!state || ommegaAppScopeSaving) return;
  state.revision = (state.revision || 0) + 1;
  state.pending = (state.pending || 0) + 1;
  const session = state.session, requested = input.checked;
  let readable = true;
  input.disabled = true; ommegaAppScopeSaving = true;
  ++ommegaTargetSnapshotGeneration;
  ommegaDetailMessage("om-detail-scope-status", ommegaDetailText("正在保存…", "Saving…"));
  try {
    await ommegaScopeRead();
    ommegaScopeSync();
    if (ommegaGlobalScopeEnabled) throw new Error(ommegaDetailText("全局作用域已开启", "Global scope is enabled"));
    if (ommegaTargetSnapshot === undefined) throw new Error(ommegaDetailText("请刷新应用列表后重试", "Refresh the app list and retry"));
    const original = await ommegaReadAppFile(OMMEGA_APP_TARGET);
    if (original !== ommegaTargetSnapshot) throw new Error(ommegaDetailText("作用域已变更，请刷新后重试", "Scope changed; refresh and retry"));
    const edited = ommegaEditAppScope(original, state.pkg, requested);
    if (edited.changed) {
      const response = await _(ommegaAppFileWriteCommand(OMMEGA_APP_TARGET, original, edited.replacement));
      if (Number(response.errno) !== 0) throw new Error(response.stderr || "Save failed");
      const saved = await ommegaReadAppFile(OMMEGA_APP_TARGET);
      if (saved !== edited.replacement) throw new Error("Scope readback did not match");
      ommegaTargetSnapshot = saved;
      Qe = ommegaScopePackages(saved || "");
      ommegaAppsBaseline = ommegaAppSettingsSnapshot(Qe);
    }
    state.box.checked = requested; state.card.classList.toggle("selected", requested); Di();
    ommegaUpdateAppScope(state.card);
    if (ommegaDetailCurrent(session)) ommegaDetailMessage("om-detail-scope-status", ommegaDetailText("已保存。", "Saved."));
  } catch (error) {
    if (ommegaDetailCurrent(session)) {
      ommegaDetailMessage("om-detail-scope-status", ommegaDetailText("保存失败", "Save failed") + ": " + error.message, true);
      try {
        const actual = await ommegaReadAppFile(OMMEGA_APP_TARGET);
        if (actual !== ommegaTargetSnapshot) ommegaTargetSnapshot = undefined;
        if (ommegaDetailCurrent(session)) {
          input.checked = ommegaGlobalScopeEnabled || ommegaScopePackages(actual || "").includes(state.pkg);
          state.box.checked = input.checked;
          state.card.classList.toggle("selected", input.checked);
          Di();
          ommegaUpdateAppScope(state.card);
        }
      } catch (_) { readable = false; }
    }
  } finally {
    state.pending--;
    ommegaAppScopeSaving = false;
    if (ommegaDetailCurrent(session)) input.disabled = !readable || ommegaGlobalScopeEnabled;
  }
}

async function ommegaDetailSaveAuth(input) {
  const state = ommegaDetail;
  if (!state || !input.checked) return;
  state.revision = (state.revision || 0) + 1;
  state.pending = (state.pending || 0) + 1;
  const session = state.session;
  let readable = true;
  ommegaDetailControls('input[name="om-app-auth"]', true);
  try {
    const read = await ommegaAuthRead();
    const apps = {...read.config.apps, [state.pkg]:input.value};
    await ommegaAuthCommit(read.contents, {...read.config, apps});
    if (ommegaDetailCurrent(session)) ommegaDetailMessage("om-detail-auth-status", ommegaDetailText("已保存，对新建密钥生效。", "Saved for newly created keys."));
  } catch (error) {
    if (ommegaDetailCurrent(session)) {
      ommegaDetailMessage("om-detail-auth-status", ommegaDetailText("保存失败", "Save failed") + ": " + error.message, true);
      try {
        const read = await ommegaAuthRead();
        const actual = read.config.apps[state.pkg] || read.config.default;
        if (ommegaDetailCurrent(session)) document.querySelector('#om-app-detail input[name="om-app-auth"][value="' + actual + '"]').checked = true;
      } catch (_) { readable = false; }
    }
  } finally {
    state.pending--;
    if (ommegaDetailCurrent(session)) ommegaDetailControls('input[name="om-app-auth"]', !readable);
  }
}

async function ommegaDetailSaveCpu(box) {
  const state = ommegaDetail;
  if (!state) return;
  state.revision = (state.revision || 0) + 1;
  state.pending = (state.pending || 0) + 1;
  const session = state.session, requested = box.checked;
  let readable = true;
  box.disabled = true;
  try {
    const read = await ommegaCpuRead();
    const packages = new Set(read.config.packages);
    if (requested) {
      const props = read.profile?.properties;
      const ready = props && ["ro.product.brand", "ro.product.device", "ro.product.name", "ro.product.manufacturer", "ro.product.model", "ro.soc.model"]
        .every(key => typeof props[key] === "string" && props[key]);
      if (!ready || read.standalone_active) throw new Error(ommegaDetailText("B 端属性不可用或独立模块仍在运行", "B properties unavailable or standalone module active"));
      if (!read.config.enabled && [...packages].some(pkg => pkg !== state.pkg)) throw new Error(ommegaDetailText("已有其他保存的应用，请先在 CPU 总设置中启用", "Other apps are saved; enable the CPU feature in its global settings first"));
      packages.add(state.pkg);
    } else packages.delete(state.pkg);
    const config = {enabled:requested || read.config.enabled, packages:[...packages]};
    const response = await _(ommegaCpuCommand("write", {expected:read.original, replacement:ommegaCpuText(config)}));
    if (Number(response.errno) !== 0) throw new Error(response.stderr || "Save failed");
    const saved = await ommegaCpuRead();
    if (ommegaCpuText(saved.config) !== ommegaCpuText(config)) throw new Error("CPU scope readback did not match");
    if (ommegaDetailCurrent(session)) {
      box.checked = saved.config.enabled && saved.config.packages.includes(state.pkg);
      ommegaDetailMessage("om-detail-cpu-status", ommegaDetailText("已保存，请重新打开应用。", "Saved. Reopen the app."));
    }
  } catch (error) {
    if (ommegaDetailCurrent(session)) {
      ommegaDetailMessage("om-detail-cpu-status", ommegaDetailText("保存失败", "Save failed") + ": " + error.message, true);
      try { const read = await ommegaCpuRead(); if (ommegaDetailCurrent(session)) box.checked = read.config.enabled && read.config.packages.includes(state.pkg); }
      catch (_) { readable = false; }
    }
  } finally {
    state.pending--;
    if (ommegaDetailCurrent(session)) box.disabled = !readable;
  }
}

// The system-app dialog only controls which system apps are visible in the app
// list. Scope stays an explicit per-app switch, so this writer never touches
// target.txt.
async function ommegaWriteSystemApp(pkg, include) {
  if (!ommegaAuthPackage(pkg) || typeof include !== "boolean") throw new Error("Invalid package");
  if (ommegaAppScopeSaving || ommegaGlobalScopePending) throw new Error("Another scope change is in progress");
  ommegaAppScopeSaving = true;
  ++ommegaTargetSnapshotGeneration;
  try {
    if (include) {
      const found = await _("pm list packages -s " + pkg);
      if (Number(found.errno) !== 0 || !String(found.stdout).split(/\r?\n/).some(line => line.trim() === "package:" + pkg))
        throw new Error(ommegaDetailText("未找到系统应用", "System app not found"));
    }
    const original = await ommegaReadAppFile(OMMEGA_SYSTEM_APPS);
    const edited = ommegaEditAppScope(original, pkg, include);
    if (!edited.changed) return;
    const result = await _(ommegaAppFileWriteCommand(OMMEGA_SYSTEM_APPS, original, edited.replacement));
    if (Number(result.errno) !== 0) throw new Error(result.stderr || "Save failed");
    if (await ommegaReadAppFile(OMMEGA_SYSTEM_APPS) !== edited.replacement) throw new Error("Readback did not match");
  } finally {
    ommegaAppScopeSaving = false;
  }
}

function ommegaInstallSystemAppWriters() {
  const dialog = document.getElementById("add-system-app-dialog");
  const add = document.getElementById("add-system-app-button");
  if (!dialog || !add || dialog.dataset.omSafeWriter) return;
  dialog.dataset.omSafeWriter = "true";
  const run = async (pkg, include, control) => {
    control.disabled = true;
    try {
      await ommegaWriteSystemApp(pkg, include);
      if (include) document.getElementById("system-app-input").value = "";
      dialog.close();
      await qt();
    } catch (error) {
      console.error("Could not update system app:", error);
      Me(ommegaDetailText("保存失败", "Save failed") + ": " + error.message);
    } finally { control.disabled = false; }
  };
  add.onclick = () => run(document.getElementById("system-app-input").value.trim(), true, add);
  dialog.addEventListener("click", event => {
    const remove = event.target.closest?.(".remove-system-app-button");
    if (!remove) return;
    event.preventDefault(); event.stopImmediatePropagation();
    const pkg = remove.closest(".system-app-item")?.querySelector("span")?.textContent.trim();
    run(pkg, false, remove);
  }, true);
}

if (typeof Ms === "function") {
  const ommegaBaseSystemApps = Ms;
  Ms = async function () {
    await ommegaBaseSystemApps();
    ommegaInstallSystemAppWriters();
  };
}

function ommegaCloseAppDetail() {
  if (!ommegaDetail) return;
  const previous = ommegaDetail;
  ommegaDetail = null; ++ommegaDetailSession;
  document.body.removeAttribute("data-om-app-detail");
  const detail = document.getElementById("om-app-detail");
  if (detail) { detail.hidden = true; detail.innerHTML = ""; }
  window.scrollTo(0, previous.scrollY);
  previous.card?.focus({preventScroll:true});
}

function ommegaOpenAppDetail(card) {
  const pkg = card.getAttribute("data-package"), box = card.querySelector("md-checkbox");
  if (!ommegaAuthPackage(pkg) || !box || !ommegaGlobalScopeKnown || ommegaGlobalScopePending) return;
  const session = ++ommegaDetailSession;
  const name = card.dataset.omOriginalName || card.querySelector(".app-name")?.textContent || pkg;
  ommegaDetail = {pkg, card, box, session, scrollY:window.scrollY};
  const detail = document.getElementById("om-app-detail");
  detail.innerHTML = '<button type="button" class="back-button" id="om-detail-back">' + ommegaDetailIcon('back') + '<span>' + ommegaDetailText("返回应用列表", "Back to apps") + '</span></button>' +
    '<div class="app-detail-heading"><span class="app-symbol app-symbol-large" id="om-detail-icon"></span><div><span class="section-kicker">APPLICATION</span><h1 tabindex="-1">' + ommegaEsc(name) + '</h1><p>' + ommegaEsc(pkg) + '</p></div></div>' +
    '<div class="app-policy-stack">' +
    ommegaDetailCard('apps', ommegaDetailText("作用域", "Scope"), '',
      ommegaDetailRow(ommegaDetailText("启用作用域", "Enable scope"), ommegaGlobalScopeEnabled
        ? ommegaDetailText("全局作用域已开启，单独设置暂不生效。", "Individual scope is paused while global scope is enabled.")
        : ommegaDetailText("控制此应用是否纳入作用域。", "Choose whether this app is in scope."),
        '<input id="om-detail-scope" type="checkbox" role="switch" ' + ((ommegaGlobalScopeEnabled || box.checked) ? 'checked ' : '') + (ommegaGlobalScopeEnabled ? 'disabled' : '') + '>') +
      '<p class="om-detail-status" id="om-detail-scope-status" role="status"></p>') +
    ommegaDetailCard('shield', ommegaDetailText("接管模式", "Takeover mode"), ommegaDetailText("选择一项", "Choose one"),
      (ommegaDetailRadio("om-app-mode", "global_default", ommegaDetailText("全局默认", "Global default"), ommegaDetailText("跟随全局接管模式。", "Follow the global mode.")) +
      ommegaDetailRadio("om-app-mode", "strongbox", "StrongBox", ommegaDetailText("为此应用选择 StrongBox。", "Use StrongBox for this app.")) +
      ommegaDetailRadio("om-app-mode", "tee", "TEE", ommegaDetailText("为此应用选择 TEE。", "Use TEE for this app."))) +
      '<p class="om-detail-status" id="om-detail-mode-status" role="status"></p>') +
    ommegaDetailCard('lock', ommegaDetailText("认证签名策略", "Authentication signing"), ommegaDetailText("选择一项", "Choose one"),
      (ommegaDetailRadio("om-app-auth", "auto", ommegaDetailText("自动", "Auto"), ommegaDetailText("按自动策略处理。", "Use the automatic policy.")) +
      ommegaDetailRadio("om-app-auth", "compatibility", ommegaDetailText("支付兼容", "Payment compatibility"), ommegaDetailText("在本机验证认证，由远端签名。", "Authenticate locally and sign remotely.")) +
      ommegaDetailRadio("om-app-auth", "preserve", ommegaDetailText("保持认证要求", "Preserve authentication"), ommegaDetailText("远端保留认证要求。", "Keep the remote authentication requirement."))) +
      '<p class="om-detail-status" id="om-detail-auth-status" role="status"></p>') +
    ommegaDetailCard('cpu', 'CPU ' + ommegaDetailText("与设备属性", "and device properties"), '',
      ommegaDetailRow(ommegaDetailText("启用", "Enable"), ommegaDetailText("为此应用启用 CPU 与设备属性策略。", "Enable the property view for this app."), '<input id="om-detail-cpu" type="checkbox" role="switch" disabled>') +
      '<p class="om-detail-status" id="om-detail-cpu-status" role="status"></p><div class="form-actions"><button type="button" id="om-detail-cpu-settings" class="secondary-button" hidden>' + ommegaDetailText("CPU 总设置", "CPU feature settings") + '</button></div>') + '</div>';
  const icon = card.querySelector(".app-icon")?.cloneNode(true);
  if (icon?.getAttribute("src")) { icon.removeAttribute("id"); icon.alt = ""; document.getElementById("om-detail-icon").appendChild(icon); }
  else document.getElementById("om-detail-icon").textContent = name.slice(0, 1).toUpperCase();
  detail.hidden = false; document.body.dataset.omAppDetail = "open";
  history.pushState({...history.state, omAppDetail:pkg}, "");
  window.scrollTo(0, 0); detail.querySelector("h1")?.focus({preventScroll:true});
  document.getElementById("om-detail-back").onclick = () => history.back();
  document.getElementById("om-detail-scope").onchange = event => ommegaDetailSaveScope(event.target);
  detail.querySelectorAll('input[name="om-app-mode"]').forEach(input => input.onchange = () => ommegaDetailSaveMode(input));
  detail.querySelectorAll('input[name="om-app-auth"]').forEach(input => input.onchange = () => ommegaDetailSaveAuth(input));
  document.getElementById("om-detail-cpu").onchange = event => ommegaDetailSaveCpu(event.target);
  document.getElementById("om-detail-cpu-settings").onclick = () => {
    for (const id of ["ommega-cpu-policy-dialog", "ommega-cpu-app-dialog"]) {
      const dialog = document.getElementById(id); if (dialog && dialog.parentNode !== document.body) document.body.appendChild(dialog);
    }
    document.getElementById("ommega-cpu-policy-dialog")?.addEventListener("close", () => { if (ommegaDetailCurrent(session)) ommegaDetailLoad(session); }, {once:true});
    ommegaCpuOpenPolicy();
  };
  ommegaDetailLoad(session);
}

// Scope is changed only by the per-app switch. The vendor bulk-selection
// actions edit hidden checkboxes and would write target.txt for several apps at
// once, so they are removed from the app menu instead of pretending to be a
// temporary selection.
function ommegaHideBulkScopeActions() {
  for (const id of ["select-all", "deselect-all", "deselect-unnecessary"]) {
    const control = document.getElementById(id);
    if (!control) continue;
    const row = control.closest?.(".om-app-menu-item");
    (row || control).hidden = true;
  }
}

function ommegaInstallAppDetail() {
  const main = document.getElementById("om-main"), list = document.getElementById("apps-list");
  if (!main || !list || document.getElementById("om-app-detail")) return false;
  const detail = document.createElement("section"); detail.id = "om-app-detail"; detail.hidden = true; main.appendChild(detail);
  const updateCards = () => list.querySelectorAll(".card[data-package]").forEach(card => {
    if (!card.dataset.omDetailReady) {
      card.dataset.omDetailReady = "true"; card.setAttribute("role", "button"); card.tabIndex = 0;
    }
    ommegaUpdateAppCount(card);
  });
  updateCards(); new MutationObserver(updateCards).observe(list, {childList:true, subtree:true});
  document.addEventListener("change", event => {
    if (event.target?.id === "ommega-global-scope-enabled") setTimeout(updateCards, 50);
  }, true);
  setTimeout(updateCards, 900);
  list.addEventListener("pointerdown", event => { if (event.target.closest?.(".card")) event.stopPropagation(); }, true);
  list.addEventListener("click", event => {
    const card = event.target.closest?.(".card[data-package]");
    if (!card) return;
    event.preventDefault(); event.stopImmediatePropagation(); ommegaOpenAppDetail(card);
  }, true);
  list.addEventListener("keydown", event => {
    const card = event.target.closest?.(".card[data-package]");
    if (!card || !["Enter", " "].includes(event.key)) return;
    event.preventDefault(); event.stopImmediatePropagation(); ommegaOpenAppDetail(card);
  }, true);
  // The shared app reader owns startup/manual loads and their CAS snapshots;
  // no independent legacy bootstrap may later replace a fresh list or draft.
  const saveFab = document.getElementById("save");
  if (saveFab) { saveFab.hidden = true; saveFab.onclick = null; }
  ommegaHideBulkScopeActions();

  window.addEventListener("popstate", () => { if (ommegaDetail && history.state?.omAppDetail !== ommegaDetail.pkg) ommegaCloseAppDetail(); });
  // The page body also carries data-om-view; only the navigation buttons may
  // close the detail page when the user leaves the Apps view.
  document.querySelectorAll(".om-nav button[data-om-view]").forEach(button => button.addEventListener("click", () => {
    if (!ommegaDetail) return;
    const state = {...history.state}; delete state.omAppDetail; history.replaceState(state, ""); ommegaCloseAppDetail();
  }, true));
  window.addEventListener("back", event => {
    if (!ommegaDetail || document.querySelector("dialog[open],md-dialog[open]")) return;
    event.preventDefault(); event.stopImmediatePropagation(); history.back();
  }, true);
  document.addEventListener("keydown", event => {
    if (event.key !== "Escape" || !ommegaDetail || document.querySelector("dialog[open],md-dialog[open]")) return;
    event.preventDefault(); history.back();
  });
  document.body.classList.add("om-app-detail-ready");
  window.addEventListener("hashchange", ommegaRefreshCertificateCounts);
  document.addEventListener("visibilitychange", ommegaRefreshCertificateCounts);
  const countTimer = setInterval(ommegaRefreshCertificateCounts, 10000);
  window.addEventListener("pagehide", () => clearInterval(countTimer), {once:true});
  ommegaRefreshCertificateCounts();
  return true;
}

let ommegaAppLabelCache = null;
async function ommegaAppFallbackLabels() {
  if (ommegaAppLabelCache) return ommegaAppLabelCache;
  if (typeof fetch !== "function") return {};
  const controller = typeof AbortController === "function" ? new AbortController() : null;
  let timer;
  try {
    const result = await Promise.race([
      fetch("applist.json", controller ? {signal:controller.signal} : {}).then(response => response.ok ? response.json() : null),
      new Promise(resolve => { timer = setTimeout(() => { controller?.abort(); resolve(null); }, 12000); })
    ]);
    if (!Array.isArray(result)) return {};
    const labels = Object.create(null);
    for (const item of result) if (ommegaAuthPackage(item?.package_name) && typeof item.app_name === "string") labels[item.package_name] = item.app_name;
    return ommegaAppLabelCache = labels;
  } catch (_) { return {}; }
  finally { clearTimeout(timer); }
}

// The manager injects `ksu` as a Java object. Reading a method off that object
// and calling it later fails in Android WebView with "Java bridge method can't
// be invoked on a non-injected object", so every query keeps the live receiver.
// Labels are resolved in small batches and then package-by-package. A single
// oversized/failed batch must not turn an entire page of real app names into
// raw package ids (Android WebView can reject a large getPackagesInfo reply).
async function ommegaAppNames(pkgs, applist) {
  const names = new Map(), labeled = new Set();
  const take = info => {
    if (!info || typeof info !== "object") return;
    const pkg = info.packageName || info.package;
    const label = info.appLabel || info.appName || info.label;
    if (pkg && label && String(label).trim()) { names.set(pkg, String(label).trim()); labeled.add(pkg); }
  };
  for (const pkg of pkgs) {
    const cached = applist?.[pkg];
    if (cached && String(cached).trim()) { names.set(pkg, String(cached).trim()); labeled.add(pkg); }
    else names.set(pkg, pkg);
  }
  const bridge = globalThis.ksu;
  if (typeof bridge?.getPackagesInfo === "function" && pkgs.length) {
    for (let i = 0; i < pkgs.length; i += 24) {
      try {
        const infos = JSON.parse(bridge.getPackagesInfo(JSON.stringify(pkgs.slice(i, i + 24))));
        for (const info of Array.isArray(infos) ? infos : []) take(info);
      } catch (_) { /* retried one by one below */ }
    }
    for (const pkg of pkgs.filter(name => !labeled.has(name))) {
      try {
        const infos = JSON.parse(bridge.getPackagesInfo(JSON.stringify([pkg])));
        for (const info of Array.isArray(infos) ? infos : [infos]) take(info);
      } catch (_) { /* package stays on its package-id fallback */ }
    }
  } else if (typeof $packageManager !== "undefined") {
    for (const pkg of pkgs) {
      try {
        const label = $packageManager.getApplicationInfo(pkg, 0, 0).getLabel();
        if (label && String(label).trim()) names.set(pkg, String(label).trim());
      } catch (_) { /* keep the package id */ }
    }
  }
  return pkgs.map(pkg => ({appName: names.get(pkg) || pkg, packageName: pkg, labeled: labeled.has(pkg)}));
}



let ommegaAppRefreshRunning = null, ommegaAppRefreshQueued = null;
async function ommegaRefreshApps() {
  if (ommegaAppRefreshRunning) {
    if (!ommegaAppRefreshQueued) ommegaAppRefreshQueued = ommegaAppRefreshRunning.catch(() => {}).then(() => {
      ommegaAppRefreshQueued = null;
      return ommegaRefreshApps();
    });
    return ommegaAppRefreshQueued;
  }
  const button = document.getElementById("refresh");
  if (button) { button.disabled = true; button.setAttribute("aria-busy", "true"); }
  ommegaAppRefreshRunning = (async () => {
    if (ommegaAppScopeSaving || ommegaGlobalScopePending) throw new Error(ommegaDetailText("正在保存，请稍后刷新。", "Saving; refresh again shortly."));
    const generation = ++ommegaTargetSnapshotGeneration;
    const [target, security, system, injector, packages, fallbackLabels] = await Promise.all([
      ommegaReadAppFile(OMMEGA_APP_TARGET), ommegaReadAppFile(OMMEGA_TARGET_SECURITY), ommegaReadAppFile(OMMEGA_SYSTEM_APPS),
      ommegaReadOnly('base64 < "' + OMMEGA_SCOPE_CONFIG + '"'), ommegaReadOnly('pm list packages -3'), ommegaAppFallbackLabels()
    ]);
    if (Number(packages.errno) !== 0) throw new Error(packages.stderr || "Could not read apps");
    let scope = null, scopeError = "";
    try {
      if (Number(injector.errno) !== 0) throw new Error(injector.stderr || "Could not read global scope");
      scope = ommegaScopeSetting(new TextDecoder("utf-8", {fatal:true,ignoreBOM:true}).decode(Uint8Array.from(atob(injector.stdout.replace(/\s/g,"")), c => c.charCodeAt(0))));
    } catch (error) { scopeError = error.message; }
    const policy = ommegaEditSecurityPolicy(security, "com.ommega.validation", null);
    if (ommegaAppScopeSaving || ommegaGlobalScopePending) throw new Error(ommegaDetailText("正在保存，请稍后刷新。", "Saving; refresh again shortly."));
    const user = packages.stdout.split(/\r?\n/).map(row => row.replace(/^package:/, "").trim()).filter(ommegaAuthPackage);
    const extra = String(system || "").split(/\r?\n/).map(row => row.trim()).filter(ommegaAuthPackage);
    const installed = new Set([...user, ...extra]);
    const targetPackages = ommegaScopePackages(target || "");
    const labels = Object.assign(Object.create(null), fallbackLabels);
    for (const card of se.querySelectorAll(".card[data-package]")) {
      const pkg = card.getAttribute?.("data-package");
      const label = card.dataset?.omOriginalName || card.querySelector?.(".app-name")?.textContent;
      if (pkg && label) labels[pkg] = label;
    }
    // The list is the system's installed-app set, not a projection of
    // target.txt. System apps explicitly added to system_app remain visible so
    // they can be switched into scope later.
    const apps = await ommegaAppNames([...installed], labels);
    // No async policy write is allowed to race the final snapshot replacement.
    if (generation !== ommegaTargetSnapshotGeneration) throw new Error(ommegaDetailText("设置已变更，请再次刷新。", "Settings changed; refresh again."));
    if (ommegaAppScopeSaving || ommegaGlobalScopePending) throw new Error(ommegaDetailText("正在保存，请稍后刷新。", "Saving; refresh again shortly."));
    // Keep the scope file exactly as read. Uninstalled entries are retained so
    // a refresh never rewrites target.txt.
    Qe = targetPackages;
    ommegaTargetSnapshot = target;
    ommegaTargetModes = policy.modes;
    ommegaAppsBaseline = ommegaAppSettingsSnapshot(Qe);
    if (scope) ommegaGlobalScopeEnabled = scope.global;
    ommegaGlobalScopeKnown = Boolean(scope); ommegaGlobalScopeMessage = scopeError; ommegaScopeSync();
    const scrollY = window.scrollY;
    $s(apps);
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    document.dispatchEvent(new Event("ommega-apps-rendered"));
    window.scrollTo(0, scrollY);
    await ommegaRefreshCertificateCounts();
    if (scopeError) throw new Error(scopeError);
    return "";
  })();
  try { return await ommegaAppRefreshRunning; }
  finally {
    ommegaAppRefreshRunning = null;
    if (button && !ommegaAppRefreshQueued) { button.disabled = false; button.setAttribute("aria-busy", "false"); }
  }
}

async function ommegaRefreshActiveView(view) {
  if (view === "apps") {
    if (ommegaDetail) return ommegaDetailLoad(ommegaDetail.session, true);
    return ommegaRefreshApps();
  }
  if (view === "settings") {
    const box = document.getElementById("ommega-global-compat");
    if (ommegaCompatSaving) throw new Error(ommegaDetailText("正在保存，请稍后刷新。", "Saving; refresh again shortly."));
    if (box) {
      const revision = ommegaCompatRevision;
      box.disabled = true;
      try {
        const {enabled} = await ommegaReadTargetCompat();
        if (revision !== ommegaCompatRevision) throw new Error(ommegaDetailText("设置已变更，请再次刷新。", "Settings changed; refresh again."));
        if (ommegaCompatSaving) throw new Error(ommegaDetailText("正在保存，请稍后刷新。", "Saving; refresh again shortly."));
        ommegaCompatEnabled = enabled; box.checked = enabled;
        const status = document.getElementById("ommega-global-compat-status");
        if (status) status.textContent = "";
      } finally { if (!ommegaCompatSaving) box.disabled = false; }
    }
    if (ommegaCapabilityBusy) throw new Error(ommegaDetailText("系统能力正在读取或保存，请稍后刷新。", "System capability settings are busy; refresh again shortly."));
    if (!document.getElementById("ommega-capability-dialog")?.open) await ommegaCapabilityRefresh();
    if (!ommegaCapabilityState) throw new Error(ommegaDetailText("系统能力状态无法读取。", "Could not read system capability status."));
  }
}
document.addEventListener("ommega-refresh", event => { event.detail.promise = ommegaRefreshActiveView(event.detail.view); });
function ommegaBindAppRefresh() {
  const button = document.getElementById("refresh");
  if (!button || button.dataset.omRefreshReady) return;
  button.dataset.omRefreshReady = "true";
  // The vendor button also has a legacy/delegated click listener. Capture the
  // action first so qt() cannot clear drafts or launch a second unawaited load.
  button.addEventListener("click", async event => {
    event.preventDefault(); event.stopImmediatePropagation();
    const output = document.getElementById("om-updated");
    if (output) output.textContent = ommegaDetailText("正在刷新应用…", "Refreshing apps…");
    try {
      const note = await ommegaRefreshApps();
      if (output) output.textContent = note || ommegaDetailText("应用列表已刷新。", "App list refreshed.");
    } catch (error) { if (output) output.textContent = ommegaDetailText("刷新失败", "Refresh failed") + ": " + error.message; }
  }, true);
}
async function ommegaLoadApps() {
  try { await ommegaRefreshApps(); return true; }
  catch (error) {
    ommegaGlobalScopeKnown = false;
    ommegaGlobalScopeMessage = ommegaDetailText("应用读取失败，请刷新重试。", "Could not read apps. Refresh to retry.") + " " + error.message;
    ommegaScopeSync();
    const output = document.getElementById("om-updated");
    if (output) output.textContent = ommegaGlobalScopeMessage;
    const loader = document.querySelector(".loading");
    if (loader) loader.style.display = "none";
    return false;
  }
}
function ommegaUseAppReader() {
  // Both startup and later legacy callers must use the same queued, read-only
  // reader. Calling the old qt() would clear the draft before Mi() even runs.
  Mi = ommegaLoadApps;
  if (typeof qt === "function") qt = ommegaLoadApps;
}
ommegaInstallAppDetail();
if (typeof window !== "undefined") {
  ommegaUseAppReader();
  window.addEventListener("ommega-settings-ready", () => { ommegaInstallAppDetail(); ommegaHideBulkScopeActions(); ommegaBindAppRefresh(); ommegaUseAppReader(); });
  ommegaBindAppRefresh();
}
