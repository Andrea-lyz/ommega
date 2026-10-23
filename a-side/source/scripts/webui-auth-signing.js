
// Runtime auth-signing policy. Kept outside the scope list, which is inert in
// global mode. App names are display-only; only validated package IDs persist.
const OMMEGA_AUTH_SIGNING = "/data/adb/ommega/ommegadata/data/auth-signing.toml";
const OMMEGA_AUTH_MODES = ["auto", "compatibility", "preserve"];
let ommegaAuthConfig = null;
let ommegaAuthOriginal = null;
let ommegaAuthBusy = false;
let ommegaAuthLoadingApps = false;
let ommegaAuthAppsLoaded = false;
let ommegaAuthDraft = "auto";
let ommegaAuthAppsDraft = null;
let ommegaAuthAppsOriginal = null;
let ommegaAuthInventory = [];
let ommegaAuthLimit = 40;

function ommegaAuthPackage(value) {
  return typeof value === "string" && value.length <= 255 &&
    /^[A-Za-z][A-Za-z0-9_]*(?:\.[A-Za-z][A-Za-z0-9_]*)+$/.test(value);
}

function ommegaAuthValidate(config) {
  if (!config || config.version !== 1 || !OMMEGA_AUTH_MODES.includes(config.default) ||
      !config.apps || Array.isArray(config.apps) || typeof config.apps !== "object" ||
      Object.keys(config.apps).length > 4096 ||
      Object.entries(config.apps).some(([pkg, mode]) => !ommegaAuthPackage(pkg) || !OMMEGA_AUTH_MODES.includes(mode))) {
    throw new Error("Invalid auth-signing configuration");
  }
  return config;
}

function ommegaAuthParse(contents) {
  if (contents === null) return {version: 1, default: "auto", apps: Object.create(null)};
  // An intentionally small schema. Reject unsupported/ambiguous TOML rather
  // than silently dropping settings written by another version or by hand.
  const result = {apps: Object.create(null)};
  let apps = false;
  for (const raw of String(contents).split(/\r?\n/)) {
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    if (line === "[apps]" && !apps) { apps = true; continue; }
    const match = apps
      ? line.match(/^"([A-Za-z0-9_.]+)"\s*=\s*"(auto|compatibility|preserve)"$/)
      : line.match(/^(version|default)\s*=\s*(1|"(?:auto|compatibility|preserve)")$/);
    if (!match) throw new Error("Unsupported auth-signing TOML; reload or edit the file directly");
    const target = apps ? result.apps : result;
    if (Object.hasOwn(target, match[1])) throw new Error("Duplicate auth-signing setting");
    target[match[1]] = apps ? match[2] : match[1] === "version" && match[2] === "1" ? 1 : match[2].replaceAll('"', "");
  }
  return ommegaAuthValidate(result);
}

function ommegaAuthToml(config) {
  ommegaAuthValidate(config);
  return ['# Ommega authentication-signing policy. New keys only.', 'version = 1',
    'default = "' + config.default + '"', '', '[apps]',
    ...Object.keys(config.apps).sort().map(pkg => '"' + pkg + '" = "' + config.apps[pkg] + '"'), ''].join("\n");
}

function ommegaAuthReadCommand() {
  return 'if [ ! -e "' + OMMEGA_AUTH_SIGNING + '" ]; then printf "M:"; ' +
    'elif [ -f "' + OMMEGA_AUTH_SIGNING + '" ]; then printf "F:"; base64 < "' + OMMEGA_AUTH_SIGNING + '"; else exit 1; fi';
}

async function ommegaAuthRead() {
  const response = await _(ommegaAuthReadCommand());
  if (Number(response.errno) !== 0) throw new Error(response.stderr || "Could not read auth-signing policy");
  const encoded = String(response.stdout).replace(/\s/g, "");
  if (encoded === "M:") return {contents: null, config: ommegaAuthParse(null)};
  if (!encoded.startsWith("F:")) throw new Error("Invalid policy read response");
  const bytes = Uint8Array.from(atob(encoded.slice(2)), c => c.charCodeAt(0));
  const contents = new TextDecoder("utf-8", {fatal: true, ignoreBOM: true}).decode(bytes);
  return {contents, config: ommegaAuthParse(contents)};
}

function ommegaAuthWriteCommand(original, config) {
  const payload = ommegaSoterRemoteBase64(JSON.stringify({expected: original, replacement: ommegaAuthToml(config)}));
  // This separate Rust executable only writes this policy. It cannot start a
  // daemon or issue HAL calls, even when launched from the manager root shell.
  return ["set -e",
    'ommega_auth_writer="/data/adb/modules/ommega/libs/arm64-v8a/ommega-auth-policy"',
    'if [ ! -x "$ommega_auth_writer" ]; then ommega_auth_writer="/data/adb/modules/ommega/libs/x86_64/ommega-auth-policy"; fi',
    '[ -x "$ommega_auth_writer" ] || { echo "Policy writer unavailable; install the complete A module" >&2; exit 1; }',
    "printf '%s' '" + payload + "' | base64 -d | \"$ommega_auth_writer\" write"].join("\n");
}

function ommegaAuthModeName(mode) {
  return ommegaUiLocalize({auto: "自动 / Auto", compatibility: "支付兼容 / Payment compatibility", preserve: "保持认证要求 / Preserve authentication"}[mode]);
}

function ommegaAuthStatus(message, dialog = false) {
  const element = document.getElementById(dialog ? "ommega-auth-app-status" : "ommega-auth-status");
  if (element) element.textContent = message;
}

function ommegaAuthSync() {
  const panel = document.getElementById("ommega-auth-signing");
  if (!panel || !panel.querySelectorAll) return;
  for (const control of panel.querySelectorAll("input,button")) {
    control.disabled = ommegaAuthBusy || (!ommegaAuthConfig && control.id !== "ommega-auth-reload");
  }
  for (const radio of panel.querySelectorAll('input[name="ommega-auth-mode"]')) radio.checked = radio.value === ommegaAuthDraft;
  const button = document.getElementById("ommega-auth-apps");
  if (button) button.textContent = ommegaUiLocalize("单应用例外 / App exceptions") + " (" + Object.keys(ommegaAuthConfig?.apps || {}).length + ")";
  const dialog = document.getElementById("ommega-auth-dialog");
  if (dialog?.querySelectorAll) for (const control of dialog.querySelectorAll("button,input,select")) {
    control.disabled = ommegaAuthBusy && !(ommegaAuthLoadingApps && control.id === "ommega-auth-app-cancel");
    if (control.id === "ommega-auth-app-save" && !ommegaAuthAppsLoaded) control.disabled = true;
  }
}

async function ommegaAuthRefresh() {
  if (ommegaAuthBusy) return;
  ommegaAuthBusy = true; ommegaAuthSync();
  ommegaAuthStatus(ommegaUiLocalize("正在读取… / Loading…"));
  try {
    const read = await ommegaAuthRead();
    ommegaAuthConfig = read.config; ommegaAuthOriginal = read.contents;
    ommegaAuthDraft = read.config.default;
    ommegaAuthStatus(ommegaUiLocalize("当前策略 / Current policy") + ": " + ommegaAuthModeName(read.config.default));
  } catch (error) {
    ommegaAuthConfig = null;
    ommegaAuthStatus(ommegaUiLocalize("读取失败 / Could not read policy") + ": " + error.message);
  } finally { ommegaAuthBusy = false; ommegaAuthSync(); }
}

async function ommegaAuthCommit(original, config) {
  const response = await _(ommegaAuthWriteCommand(original, config));
  if (Number(response.errno) !== 0) throw new Error(response.stderr || "Could not write policy");
  const read = await ommegaAuthRead();
  if (ommegaAuthToml(read.config) !== ommegaAuthToml(config)) throw new Error("Policy readback did not match");
  ommegaAuthConfig = read.config; ommegaAuthOriginal = read.contents;
}

async function ommegaAuthSaveDefault() {
  if (ommegaAuthBusy || !ommegaAuthConfig) return;
  ommegaAuthBusy = true; ommegaAuthSync();
  ommegaAuthStatus(ommegaUiLocalize("正在保存… / Saving…"));
  try {
    await ommegaAuthCommit(ommegaAuthOriginal, {...ommegaAuthConfig, default: ommegaAuthDraft});
    ommegaAuthStatus(ommegaUiLocalize("策略已保存，将用于之后新建的密钥。 / Policy saved for newly created keys."));
  } catch (error) {
    ommegaAuthStatus(ommegaUiLocalize("保存失败 / Could not save policy") + ": " + error.message);
  } finally { ommegaAuthBusy = false; ommegaAuthSync(); }
}

function ommegaAuthFilteredApps(query) {
  const search = String(query).trim().toLocaleLowerCase();
  return ommegaAuthInventory.filter(app => (app.appName + " " + app.packageName).toLocaleLowerCase().includes(search));
}

function ommegaAuthRenderApps() {
  const list = document.getElementById("ommega-auth-app-list");
  const search = document.getElementById("ommega-auth-search");
  const matches = ommegaAuthFilteredApps(search.value);
  list.replaceChildren();
  for (const app of matches.slice(0, ommegaAuthLimit)) {
    const row = document.createElement("label"); row.className = "ommega-auth-app";
    const name = document.createElement("span");
    const title = document.createElement("strong"); title.textContent = app.appName;
    const pkg = document.createElement("small"); pkg.textContent = app.packageName;
    name.append(title, pkg);
    const select = document.createElement("select");
    select.setAttribute("aria-label", app.appName + " — " + app.packageName);
    for (const mode of ["inherit", ...OMMEGA_AUTH_MODES]) {
      const option = document.createElement("option"); option.value = mode;
      option.textContent = mode === "inherit"
        ? ommegaUiLocalize("使用默认 / Use default") + " (" + ommegaAuthModeName(ommegaAuthConfig.default) + ")"
        : ommegaAuthModeName(mode);
      select.appendChild(option);
    }
    select.value = ommegaAuthAppsDraft[app.packageName] || "inherit";
    select.addEventListener("change", () => {
      if (select.value === "inherit") delete ommegaAuthAppsDraft[app.packageName];
      else ommegaAuthAppsDraft[app.packageName] = select.value;
      ommegaAuthStatus(ommegaUiLocalize("例外尚未保存 / Exceptions not saved yet"), true);
    });
    row.append(name, select); list.appendChild(row);
  }
  document.getElementById("ommega-auth-more").hidden = matches.length <= ommegaAuthLimit;
  ommegaAuthStatus(ommegaUiLocalize("匹配应用 / Matching apps") + ": " + matches.length, true);
}

async function ommegaAuthOpenApps() {
  if (ommegaAuthBusy || !ommegaAuthConfig) return;
  ommegaAuthAppsDraft = {...ommegaAuthConfig.apps}; ommegaAuthAppsOriginal = ommegaAuthOriginal;
  const dialog = document.getElementById("ommega-auth-dialog");
  dialog.showModal();
  document.getElementById("ommega-auth-search").value = "";
  document.getElementById("ommega-auth-app-list").replaceChildren();
  ommegaAuthBusy = true; ommegaAuthLoadingApps = true; ommegaAuthAppsLoaded = false; ommegaAuthSync();
  ommegaAuthStatus(ommegaUiLocalize("正在读取应用… / Loading apps…"), true);
  try {
    const packages = await Zo("all");
    const all = [...new Set([...packages, ...Object.keys(ommegaAuthAppsDraft)])].filter(ommegaAuthPackage);
    ommegaAuthInventory = await ommegaNames(all, {});
    ommegaAuthInventory.sort((a, b) => Number(Object.hasOwn(ommegaAuthAppsDraft, b.packageName)) -
      Number(Object.hasOwn(ommegaAuthAppsDraft, a.packageName)) || a.appName.localeCompare(b.appName));
    ommegaAuthAppsLoaded = true; ommegaAuthLimit = 40; ommegaAuthRenderApps();
  } catch (error) {
    ommegaAuthStatus(ommegaUiLocalize("读取应用失败，请关闭后重试 / Could not load apps; close and retry") + ": " + error.message, true);
  } finally {
    ommegaAuthBusy = false; ommegaAuthLoadingApps = false; ommegaAuthSync();
    if (dialog.open) document.getElementById("ommega-auth-search").focus();
    else document.getElementById("ommega-auth-apps").focus();
  }
}

async function ommegaAuthSaveApps() {
  if (ommegaAuthBusy || !ommegaAuthAppsDraft || !ommegaAuthAppsLoaded) return;
  ommegaAuthBusy = true; ommegaAuthSync();
  ommegaAuthStatus(ommegaUiLocalize("正在保存… / Saving…"), true);
  try {
    await ommegaAuthCommit(ommegaAuthAppsOriginal, {...ommegaAuthConfig, apps: ommegaAuthAppsDraft});
    document.getElementById("ommega-auth-dialog").close();
    ommegaAuthStatus(ommegaUiLocalize("单应用例外已保存，将用于之后新建的密钥。 / App exceptions saved for newly created keys."));
  } catch (error) {
    ommegaAuthStatus(ommegaUiLocalize("保存失败 / Could not save policy") + ": " + error.message, true);
  } finally {
    ommegaAuthBusy = false; ommegaAuthSync();
    if (!document.getElementById("ommega-auth-dialog").open) document.getElementById("ommega-auth-apps").focus();
  }
}

function ommegaInstallAuthSigning() {
  const anchor = document.getElementById("ommega-global-scope") || document.getElementById("apps-list");
  if (!anchor?.parentElement || document.getElementById("ommega-auth-signing")) return false;
  const panel = document.createElement("section"); panel.id = "ommega-auth-signing";
  panel.innerHTML = ommegaUiLocalizeHtml('<style>' +
    '#ommega-auth-signing{margin:12px 16px;padding:16px;border-radius:16px;background:var(--md-sys-color-surface-container,#eee);color:var(--md-sys-color-on-surface,#222)}' +
    '#ommega-auth-signing h3,#ommega-auth-dialog h3{margin:0 0 12px;font-size:18px}' +
    '#ommega-auth-signing fieldset{border:0;margin:0;padding:0}#ommega-auth-signing label{display:flex;align-items:center;gap:12px;padding:8px 0;min-height:48px;cursor:pointer}' +
    '#ommega-auth-signing small,#ommega-auth-dialog small{display:block;font-size:13px;line-height:1.5;overflow-wrap:anywhere}' +
    '#ommega-auth-signing input[type="radio"]{width:20px;height:20px;flex:none;accent-color:var(--md-sys-color-primary,#465b91)}' +
    '#ommega-auth-signing button,#ommega-auth-dialog button,#ommega-auth-dialog select,#ommega-auth-search{font:inherit;min-height:44px;border:1px solid var(--md-sys-color-outline,#777);border-radius:12px;padding:8px 12px;background:var(--md-sys-color-surface,#fff);color:var(--md-sys-color-on-surface,#222)}' +
    '#ommega-auth-signing button,#ommega-auth-dialog button{cursor:pointer}#ommega-auth-signing button:disabled,#ommega-auth-dialog :disabled{opacity:.55;cursor:wait}' +
    '#ommega-auth-signing :focus-visible,#ommega-auth-dialog :focus-visible{outline:3px solid var(--md-sys-color-primary,#465b91);outline-offset:3px}' +
    '.ommega-auth-actions{display:flex;gap:8px;flex-wrap:wrap;margin-top:12px}' +
    '#ommega-auth-status,#ommega-auth-app-status{font-size:13px;line-height:1.5;margin-top:12px;overflow-wrap:anywhere}' +
    '#ommega-auth-dialog{box-sizing:border-box;width:calc(100% - 24px);max-width:640px;max-height:85vh;max-height:85dvh;padding:20px;border:0;border-radius:20px;background:var(--md-sys-color-surface,#fff);color:var(--md-sys-color-on-surface,#222)}' +
    '#ommega-auth-dialog[open]{display:flex;flex-direction:column;overflow:hidden}#ommega-auth-dialog>*{flex-shrink:0}' +
    '#ommega-auth-app-list{overflow-y:auto;min-height:0;max-height:42vh;flex:1 1 auto}' +
    '#ommega-auth-dialog::backdrop{background:#0008}#ommega-auth-search{box-sizing:border-box;width:100%;margin:8px 0}' +
    '#ommega-auth-app-list .ommega-auth-app{display:flex;align-items:center;flex-wrap:wrap;gap:8px;justify-content:space-between;padding:12px 0;border-bottom:1px solid var(--md-sys-color-outline-variant,#bbb)}' +
    '.ommega-auth-app>span{flex:1;min-width:160px;overflow-wrap:anywhere}.ommega-auth-app select{max-width:100%}' +
    '</style><h3 id="ommega-auth-heading">认证签名策略 / Authentication signing</h3>' +
    '<fieldset aria-labelledby="ommega-auth-heading">' +
    '<label><input type="radio" name="ommega-auth-mode" value="auto"><span><strong>自动（推荐） / Auto (recommended)</strong><small>识别已知认证检测探针，其余请求优先支付兼容。 / Preserve known authentication probes; use payment compatibility for other eligible requests.</small></span></label>' +
    '<label><input type="radio" name="ommega-auth-mode" value="compatibility"><span><strong>支付兼容 / Payment compatibility</strong><small>在本机验证认证，由远端签名。 / Authenticate on this device and sign remotely.</small></span></label>' +
    '<label><input type="radio" name="ommega-auth-mode" value="preserve"><span><strong>保持认证要求 / Preserve authentication</strong><small>远端保留认证要求，部分应用的指纹签名可能失败。 / Keep authentication requirements on the remote key; fingerprint signing may fail in some apps.</small></span></label>' +
    '</fieldset><p style="font-size:13px;line-height:1.5">仅影响之后新建的密钥，已有密钥不会自动转换。 / Only affects newly created keys. Existing keys are not converted.</p>' +
    '<div class="ommega-auth-actions"><button id="ommega-auth-save">保存策略 / Save policy</button><button id="ommega-auth-apps">单应用例外 / App exceptions</button><button id="ommega-auth-reload">重新读取 / Reload</button></div>' +
    '<div id="ommega-auth-status" role="status" aria-live="polite"></div>' +
    '<dialog id="ommega-auth-dialog" aria-labelledby="ommega-auth-app-heading"><h3 id="ommega-auth-app-heading">单应用例外 / App exceptions</h3>' +
    '<small>例外仅对作用域内应用生效；开启全局作用域后仍然有效。共享 UID 的应用可能共用策略，保持认证要求优先。 / Exceptions apply only to apps in scope and remain active in global scope. Apps sharing a UID may share a policy; preserve authentication takes priority.</small>' +
    '<label for="ommega-auth-search">搜索应用名称或包名 / Search app name or package</label><input id="ommega-auth-search" type="search" autocomplete="off">' +
    '<div id="ommega-auth-app-list"></div><button id="ommega-auth-more" hidden>显示更多 / Show more</button>' +
    '<div id="ommega-auth-app-status" role="status" aria-live="polite"></div>' +
    '<div class="ommega-auth-actions"><button id="ommega-auth-app-save">保存例外 / Save exceptions</button><button id="ommega-auth-app-cancel">取消 / Cancel</button></div></dialog>');
  anchor.parentElement.insertBefore(panel, anchor);
  const byId = id => document.getElementById(id);
  byId("ommega-auth-save").addEventListener("click", ommegaAuthSaveDefault);
  byId("ommega-auth-reload").addEventListener("click", ommegaAuthRefresh);
  byId("ommega-auth-apps").addEventListener("click", ommegaAuthOpenApps);
  byId("ommega-auth-app-save").addEventListener("click", ommegaAuthSaveApps);
  byId("ommega-auth-app-cancel").addEventListener("click", () => byId("ommega-auth-dialog").close());
  byId("ommega-auth-dialog").addEventListener("cancel", event => { if (ommegaAuthBusy && !ommegaAuthLoadingApps) event.preventDefault(); });
  byId("ommega-auth-dialog").addEventListener("close", () => byId("ommega-auth-apps").focus());
  byId("ommega-auth-search").addEventListener("input", () => { ommegaAuthLimit = 40; ommegaAuthRenderApps(); });
  byId("ommega-auth-more").addEventListener("click", () => { ommegaAuthLimit += 40; ommegaAuthRenderApps(); });
  for (const radio of panel.querySelectorAll('input[name="ommega-auth-mode"]')) radio.addEventListener("change", () => {
    ommegaAuthDraft = radio.value;
    ommegaAuthStatus(ommegaUiLocalize("策略尚未保存 / Policy not saved yet"));
  });
  ommegaAuthRefresh(); return true;
}

ommegaInstallAuthSigning();
