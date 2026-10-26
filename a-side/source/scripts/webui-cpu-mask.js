
// Independent application scope for the bundled exp6 property view.
let ommegaCpuState = null, ommegaCpuDraft = null, ommegaCpuAppsDraft = null;
let ommegaCpuBusy = false, ommegaCpuLoadingApps = false;
let ommegaCpuSession = 0, ommegaCpuAppSession = 0;
let ommegaCpuInventory = [], ommegaCpuLimit = 40, ommegaCpuSystemsLoaded = false;

function ommegaCpuValidate(config) {
  if (!config || typeof config.enabled !== "boolean" || !Array.isArray(config.packages) ||
      config.packages.length > 4096 || new Set(config.packages).size !== config.packages.length ||
      config.packages.some(pkg => !ommegaAuthPackage(pkg))) throw new Error("Invalid CPU scope");
  return config;
}

function ommegaCpuText(config) {
  ommegaCpuValidate(config);
  return "version=1\nenabled=" + config.enabled + "\n" +
    [...config.packages].sort().map(pkg => "package=" + pkg + "\n").join("");
}

function ommegaCpuCommand(command, payload = null) {
  if (!["status", "write"].includes(command)) throw new Error("Unsupported CPU policy command");
  const prefix = ['set -e',
    'ommega_cpu_writer="/data/adb/modules/ommega/libs/arm64-v8a/ommega-cpu-policy"',
    'if [ ! -x "$ommega_cpu_writer" ]; then ommega_cpu_writer="/data/adb/modules/ommega/libs/x86_64/ommega-cpu-policy"; fi',
    '[ -x "$ommega_cpu_writer" ] || { echo "CPU policy writer unavailable; install the complete A module" >&2; exit 1; }'];
  prefix.push(payload === null ? '"$ommega_cpu_writer" status' :
    "printf '%s' '" + ommegaSoterRemoteBase64(JSON.stringify(payload)) + "' | base64 -d | \"$ommega_cpu_writer\" write");
  return prefix.join("\n");
}

async function ommegaCpuRead() {
  const response = await _(ommegaCpuCommand("status"));
  if (Number(response.errno) !== 0) throw new Error(response.stderr || "Could not read CPU scope");
  const state = JSON.parse(response.stdout);
  ommegaCpuValidate(state.config);
  if (state.original !== null && typeof state.original !== "string") throw new Error("Invalid CPU policy response");
  if (state.engine !== "exp6" || typeof state.standalone_active !== "boolean") throw new Error("Incompatible CPU module");
  return state;
}

function ommegaCpuStatus(message, apps = false) {
  const element = document.getElementById(apps ? "ommega-cpu-app-status" : "ommega-cpu-status");
  if (element) element.textContent = message;
}

function ommegaCpuSync() {
  const get = id => document.getElementById(id);
  const policy = get("ommega-cpu-policy-dialog");
  if (!policy) return;
  for (const control of policy.querySelectorAll("button,input"))
    control.disabled = control.id !== "ommega-cpu-policy-back" && (ommegaCpuBusy || !ommegaCpuDraft && control.id !== "ommega-cpu-reload");
  if (ommegaCpuDraft) get("ommega-cpu-enabled").checked = ommegaCpuDraft.enabled;
  const profile = ommegaCpuState?.profile?.properties;
  const profileReady = profile && ["ro.product.brand", "ro.product.device", "ro.product.name", "ro.product.manufacturer", "ro.product.model", "ro.soc.model"]
    .every(key => typeof profile[key] === "string" && profile[key]);
  get("ommega-cpu-save").disabled = ommegaCpuBusy || !ommegaCpuDraft ||
    ommegaCpuDraft.enabled && (!profileReady || ommegaCpuState?.standalone_active);
  get("ommega-cpu-scope-count").textContent = ommegaUiLocalize("已选应用 / Selected apps") + ": " + (ommegaCpuDraft?.packages.length || 0);
  for (const [id, key] of [["model", "ro.product.model"], ["soc", "ro.soc.model"], ["brand", "ro.product.brand"],
    ["device", "ro.product.device"], ["product", "ro.product.name"], ["manufacturer", "ro.product.manufacturer"]])
    get("ommega-cpu-b-" + id).textContent = profile?.[key] || "—";
  get("ommega-cpu-profile-note").textContent = ommegaUiLocalize(ommegaCpuState?.standalone_active
    ? "独立 MaskCPU 仍已启用。请先停用独立模块，并在 A 重启后使用此入口。 / Standalone MaskCPU is active. Disable it and restart A before using this entry."
    : profileReady ? "属性由 B 端自动提供。 / Properties are supplied automatically by B."
      : "尚未收到 B 端属性。请安装配套 B 模块并连接远程链路。 / No B properties received. Install the matching B module and connect the relay.");
  const summary = get("ommega-cpu-summary");
  if (ommegaCpuState) summary.textContent = ommegaUiLocalize(ommegaCpuState.config.enabled ? "已启用 / Enabled" : "已关闭 / Off") +
    " · " + ommegaCpuState.config.packages.length + " " + ommegaUiLocalize("个应用 / apps");
  for (const control of get("ommega-cpu-app-dialog").querySelectorAll("button,input"))
    control.disabled = control.id !== "ommega-cpu-app-back" && (ommegaCpuLoadingApps || ommegaCpuBusy);
}

async function ommegaCpuRefresh() {
  if (ommegaCpuBusy) return;
  const session = ++ommegaCpuSession;
  ommegaCpuBusy = true; ommegaCpuSync();
  ommegaCpuStatus(ommegaUiLocalize("正在读取… / Loading…"));
  try {
    const state = await ommegaCpuRead();
    if (session !== ommegaCpuSession) return;
    ommegaCpuState = state;
    ommegaCpuDraft = {enabled:state.config.enabled, packages:[...state.config.packages]};
    ommegaCpuStatus("");
  } catch (error) {
    if (session === ommegaCpuSession) {
      ommegaCpuState = null; ommegaCpuDraft = null;
      ommegaCpuStatus(ommegaUiLocalize("读取失败 / Could not load settings") + ": " + error.message);
      document.getElementById("ommega-cpu-summary").textContent = ommegaUiLocalize("状态不可用 / Status unavailable");
    }
  } finally { ommegaCpuBusy = false; ommegaCpuSync(); }
}

async function ommegaCpuOpenPolicy() {
  const dialog = document.getElementById("ommega-cpu-policy-dialog");
  if (dialog.open) return;
  dialog.showModal();
  await ommegaCpuRefresh();
}

async function ommegaCpuSave() {
  if (ommegaCpuBusy || !ommegaCpuDraft || !ommegaCpuState) return;
  const config = {enabled:ommegaCpuDraft.enabled, packages:[...ommegaCpuDraft.packages]};
  if (config.enabled && (!ommegaCpuState.profile || ommegaCpuState.standalone_active)) return;
  ommegaCpuBusy = true; ommegaCpuSync();
  ommegaCpuStatus(ommegaUiLocalize("正在保存… / Saving…"));
  try {
    const response = await _(ommegaCpuCommand("write", {expected:ommegaCpuState.original, replacement:ommegaCpuText(config)}));
    if (Number(response.errno) !== 0) throw new Error(response.stderr || "Could not save CPU scope");
    const state = await ommegaCpuRead();
    if (ommegaCpuText(state.config) !== ommegaCpuText(config)) throw new Error("CPU scope readback did not match");
    ommegaCpuState = state; ommegaCpuDraft = {...state.config, packages:[...state.config.packages]};
    document.getElementById("ommega-cpu-policy-dialog").close();
  } catch (error) { ommegaCpuStatus(ommegaUiLocalize("保存失败 / Could not save settings") + ": " + error.message); }
  finally { ommegaCpuBusy = false; ommegaCpuSync(); }
}

function ommegaCpuRenderApps() {
  const get = id => document.getElementById(id);
  if (!ommegaCpuAppsDraft) return;
  const query = get("ommega-cpu-search").value.trim().toLocaleLowerCase();
  const system = get("ommega-cpu-system").checked;
  const matches = ommegaCpuInventory.filter(app => (system || !app.system) &&
    (app.appName + " " + app.packageName).toLocaleLowerCase().includes(query));
  const list = get("ommega-cpu-app-list"); list.replaceChildren();
  for (const app of matches.slice(0, ommegaCpuLimit)) {
    const row = document.createElement("label"); row.className = "ommega-auth-app";
    const picture = document.createElement("span"); picture.className = "ommega-auth-icon"; picture.setAttribute("aria-hidden", "true");
    const fallback = document.createElement("span"); fallback.textContent = [...app.appName][0] || "A";
    const image = document.createElement("img"); image.alt = ""; image.width = 40; image.height = 40; image.loading = "lazy"; image.decoding = "async";
    picture.append(fallback, image); ommegaAuthLoadIcon(image, app.packageName);
    const name = document.createElement("span"); name.className = "ommega-auth-app-name";
    const title = document.createElement("strong"); title.textContent = app.appName;
    const pkg = document.createElement("small"); pkg.textContent = app.packageName; name.append(title, pkg);
    const check = document.createElement("input"); check.type = "checkbox"; check.checked = ommegaCpuAppsDraft.has(app.packageName);
    check.setAttribute("aria-label", app.appName + " — " + app.packageName);
    check.addEventListener("change", () => {
      if (check.checked) ommegaCpuAppsDraft.add(app.packageName); else ommegaCpuAppsDraft.delete(app.packageName);
      ommegaCpuStatus(ommegaUiLocalize("已选应用 / Selected apps") + ": " + ommegaCpuAppsDraft.size, true);
    });
    row.append(picture, name, check); list.appendChild(row);
  }
  get("ommega-cpu-more").hidden = matches.length <= ommegaCpuLimit;
  ommegaCpuStatus(ommegaUiLocalize("匹配应用 / Matching apps") + ": " + matches.length + " · " +
    ommegaUiLocalize("已选 / Selected") + ": " + ommegaCpuAppsDraft.size, true);
}

async function ommegaCpuOpenApps() {
  if (ommegaCpuBusy || !ommegaCpuDraft) return;
  const session = ++ommegaCpuAppSession;
  ommegaCpuAppsDraft = new Set(ommegaCpuDraft.packages);
  ommegaCpuInventory = []; ommegaCpuLimit = 40; ommegaCpuSystemsLoaded = false;
  ommegaCpuLoadingApps = true;
  const get = id => document.getElementById(id), dialog = get("ommega-cpu-app-dialog");
  get("ommega-cpu-search").value = ""; get("ommega-cpu-system").checked = false;
  get("ommega-cpu-app-list").replaceChildren(); get("ommega-cpu-more").hidden = true;
  dialog.showModal(); ommegaCpuSync();
  ommegaCpuStatus(ommegaUiLocalize("正在读取用户应用… / Loading user apps…"), true);
  try {
    const apps = await ommegaAuthReadApps("user");
    if (session !== ommegaCpuAppSession || !dialog.open) return;
    const byPackage = new Map(apps.map(app => [app.packageName, app]));
    for (const pkg of ommegaCpuAppsDraft) if (!byPackage.has(pkg)) byPackage.set(pkg, {packageName:pkg, appName:pkg, system:false});
    ommegaCpuInventory = [...byPackage.values()].sort((a,b) => a.appName.localeCompare(b.appName));
    ommegaCpuRenderApps();
  } catch (error) {
    if (session === ommegaCpuAppSession) ommegaCpuStatus(ommegaUiLocalize("读取应用失败，请返回重试 / Could not load apps; go back and retry") + ": " + error.message, true);
  } finally {
    if (session === ommegaCpuAppSession) { ommegaCpuLoadingApps = false; ommegaCpuSync(); }
  }
}

async function ommegaCpuToggleSystem() {
  const get = id => document.getElementById(id);
  ommegaCpuLimit = 40;
  if (!get("ommega-cpu-system").checked || ommegaCpuSystemsLoaded) { ommegaCpuRenderApps(); return; }
  if (ommegaCpuLoadingApps) return;
  const session = ommegaCpuAppSession;
  ommegaCpuLoadingApps = true; ommegaCpuSync();
  try {
    const apps = await ommegaAuthReadApps("system");
    if (session !== ommegaCpuAppSession) return;
    const byPackage = new Map(ommegaCpuInventory.map(app => [app.packageName, app]));
    for (const app of apps) byPackage.set(app.packageName, app);
    ommegaCpuInventory = [...byPackage.values()].sort((a,b) => a.appName.localeCompare(b.appName));
    ommegaCpuSystemsLoaded = true; ommegaCpuRenderApps();
  } catch (error) {
    if (session === ommegaCpuAppSession) {
      get("ommega-cpu-system").checked = false;
      ommegaCpuStatus(ommegaUiLocalize("读取系统应用失败 / Could not load system apps") + ": " + error.message, true);
    }
  } finally {
    if (session === ommegaCpuAppSession) { ommegaCpuLoadingApps = false; ommegaCpuSync(); }
  }
}

function ommegaInstallCpuMask() {
  const slot = document.getElementById("om-cpu-mask-slot");
  if (!slot || document.getElementById("ommega-cpu-mask")) return false;
  const panel = document.createElement("section"); panel.id = "ommega-cpu-mask";
  const back = '<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="m14 5-7 7 7 7"/></svg>';
  const forward = '<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="m9 5 7 7-7 7"/></svg>';
  panel.innerHTML = ommegaUiLocalizeHtml(
    '<button type="button" id="ommega-cpu-open" class="om-action"><span>CPU 与设备属性 / CPU and device properties<small id="ommega-cpu-summary">正在读取… / Loading…</small></span>' + forward + '</button>' +
    '<dialog id="ommega-cpu-policy-dialog" class="ommega-auth-modal" aria-labelledby="ommega-cpu-heading" data-om-return-focus="ommega-cpu-open">' +
    '<header class="ommega-auth-toolbar"><button type="button" id="ommega-cpu-policy-back" class="om-button secondary">' + back + '<span>返回 / Back</span></button><h2 id="ommega-cpu-heading">CPU 与设备属性 / CPU and device properties</h2></header>' +
    '<div class="ommega-auth-policy-content"><label class="ommega-cpu-enable"><input type="checkbox" role="switch" id="ommega-cpu-enabled"><span><strong>启用应用属性视图 / Enable app property view</strong><small>仅对下方选定的应用生效。 / Applies only to the apps selected below.</small></span></label>' +
    '<button type="button" id="ommega-cpu-apps" class="om-action"><span>选择作用域 / Choose scope<small id="ommega-cpu-scope-count"></small></span>' + forward + '</button>' +
    '<h3>B 端设备 / B device</h3><dl class="ommega-cpu-properties">' +
    '<dt>CPU</dt><dd id="ommega-cpu-b-soc">—</dd><dt>型号 / Model</dt><dd id="ommega-cpu-b-model">—</dd>' +
    '<dt>品牌 / Brand</dt><dd id="ommega-cpu-b-brand">—</dd><dt>设备 / Device</dt><dd id="ommega-cpu-b-device">—</dd>' +
    '<dt>产品 / Product</dt><dd id="ommega-cpu-b-product">—</dd><dt>制造商 / Manufacturer</dt><dd id="ommega-cpu-b-manufacturer">—</dd></dl>' +
    '<p class="ommega-auth-note" id="ommega-cpu-profile-note"></p><p class="ommega-auth-note">使用 exp6。需要启用 Zygisk；保存后重新打开目标应用生效。此作用域独立于 KeyMint 作用域和全局开关。 / Uses exp6. Requires Zygisk; reopen target apps after saving. This scope is independent of the KeyMint scope and its global switch.</p>' +
    '<div id="ommega-cpu-status" role="status" aria-live="polite"></div><div class="ommega-auth-actions"><button type="button" id="ommega-cpu-reload" class="om-button secondary">重新读取 / Reload</button><button type="button" id="ommega-cpu-save" class="om-button">保存设置 / Save settings</button></div></div></dialog>' +
    '<dialog id="ommega-cpu-app-dialog" class="ommega-auth-modal" aria-labelledby="ommega-cpu-app-heading" data-om-return-focus="ommega-cpu-apps">' +
    '<header class="ommega-auth-toolbar"><button type="button" id="ommega-cpu-app-back" class="om-button secondary">' + back + '<span>返回 / Back</span></button><h2 id="ommega-cpu-app-heading">选择作用域 / Choose scope</h2><button type="button" id="ommega-cpu-app-confirm" class="om-button">确认 / Confirm</button></header>' +
    '<div class="ommega-auth-app-filters"><label for="ommega-cpu-search">搜索应用名称或包名 / Search app name or package</label><input id="ommega-cpu-search" type="search" autocomplete="off">' +
    '<label class="ommega-auth-system-option"><input id="ommega-cpu-system" type="checkbox" role="switch"><span>显示系统应用 / Show system apps</span></label>' +
    '<small>确认范围后，返回上一级保存设置。 / Confirm the scope, then save settings in the previous dialog.</small><div id="ommega-cpu-app-status" role="status" aria-live="polite"></div></div>' +
    '<div id="ommega-cpu-app-list"></div><button type="button" id="ommega-cpu-more" class="om-button secondary" hidden>显示更多 / Show more</button></dialog>');
  slot.appendChild(panel);
  const get = id => document.getElementById(id);
  get("ommega-cpu-open").addEventListener("click", ommegaCpuOpenPolicy);
  get("ommega-cpu-reload").addEventListener("click", ommegaCpuRefresh);
  get("ommega-cpu-save").addEventListener("click", ommegaCpuSave);
  get("ommega-cpu-apps").addEventListener("click", ommegaCpuOpenApps);
  get("ommega-cpu-policy-back").addEventListener("click", () => get("ommega-cpu-policy-dialog").close());
  get("ommega-cpu-app-back").addEventListener("click", () => get("ommega-cpu-app-dialog").close());
  get("ommega-cpu-app-dialog").addEventListener("close", () => {
    ++ommegaCpuAppSession; ommegaCpuLoadingApps = false; ommegaCpuAppsDraft = null; ommegaCpuSync();
  });
  get("ommega-cpu-enabled").addEventListener("change", () => {
    if (ommegaCpuDraft) ommegaCpuDraft.enabled = get("ommega-cpu-enabled").checked;
    ommegaCpuStatus(ommegaUiLocalize("设置尚未保存 / Settings not saved yet")); ommegaCpuSync();
  });
  get("ommega-cpu-app-confirm").addEventListener("click", () => {
    if (!ommegaCpuAppsDraft || ommegaCpuLoadingApps) return;
    ommegaCpuDraft.packages = [...ommegaCpuAppsDraft].sort();
    get("ommega-cpu-app-dialog").close();
    ommegaCpuStatus(ommegaUiLocalize("范围已确认，请保存设置 / Scope confirmed; save settings")); ommegaCpuSync();
  });
  get("ommega-cpu-search").addEventListener("input", () => { ommegaCpuLimit = 40; ommegaCpuRenderApps(); });
  get("ommega-cpu-system").addEventListener("change", ommegaCpuToggleSystem);
  get("ommega-cpu-more").addEventListener("click", () => { ommegaCpuLimit += 40; ommegaCpuRenderApps(); });
  ommegaCpuRefresh(); return true;
}

ommegaInstallCpuMask();
if (typeof window !== "undefined") window.addEventListener("ommega-settings-ready", ommegaInstallCpuMask);
