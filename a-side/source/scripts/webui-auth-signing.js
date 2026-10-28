
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
let ommegaAuthShowSystem = false;
let ommegaAuthSystemsLoaded = false;
let ommegaAuthAppSession = 0;

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
  const response = await ommegaReadOnly(ommegaAuthReadCommand());
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

function ommegaAuthStatus(message, apps = false) {
  const element = document.getElementById(apps ? "ommega-auth-app-status" : "ommega-auth-status");
  if (element) element.textContent = message;
  if (!apps && !ommegaAuthConfig) {
    const summary = document.getElementById("ommega-auth-summary");
    if (summary) summary.textContent = message;
  }
}

function ommegaAuthSync() {
  const policy = document.getElementById("ommega-auth-policy-dialog");
  if (policy?.querySelectorAll) {
    for (const control of policy.querySelectorAll("input,button")) {
      control.disabled = ommegaAuthBusy || (!ommegaAuthConfig && control.id !== "ommega-auth-reload");
      if (control.id === "ommega-auth-policy-back") control.disabled = false;
    }
    for (const radio of policy.querySelectorAll('input[name="ommega-auth-mode"]')) radio.checked = radio.value === ommegaAuthDraft;
  }
  const count = Object.keys(ommegaAuthConfig?.apps || {}).length;
  const button = document.getElementById("ommega-auth-apps");
  if (button) {
    button.disabled = ommegaAuthBusy || !ommegaAuthConfig;
    button.textContent = ommegaUiLocalize("单应用例外 / App exceptions") + " (" + count + ")";
  }
  const launcher = document.getElementById("ommega-auth-open");
  if (launcher) launcher.disabled = ommegaAuthBusy;
  const summary = document.getElementById("ommega-auth-summary");
  if (summary && ommegaAuthConfig) summary.textContent = ommegaAuthModeName(ommegaAuthConfig.default) + " · " +
    ommegaUiLocalize("单应用例外 / App exceptions") + " " + count;
  const dialog = document.getElementById("ommega-auth-dialog");
  if (dialog?.querySelectorAll) for (const control of dialog.querySelectorAll("button,input,select")) {
    control.disabled = ommegaAuthBusy;
    if (["ommega-auth-app-save", "ommega-auth-search", "ommega-auth-system"].includes(control.id)) control.disabled ||= !ommegaAuthAppsLoaded;
    if (control.id === "ommega-auth-app-back") control.disabled = false;
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

async function ommegaAuthOpenPolicy() {
  const dialog = document.getElementById("ommega-auth-policy-dialog");
  if (dialog.open || ommegaAuthBusy) return;
  dialog.showModal();
  await ommegaAuthRefresh();
  if (dialog.open) (dialog.querySelector("input:checked") || document.getElementById("ommega-auth-reload")).focus();
}

async function ommegaAuthSaveDefault() {
  if (ommegaAuthBusy || !ommegaAuthConfig) return;
  ommegaAuthBusy = true; ommegaAuthSync();
  ommegaAuthStatus(ommegaUiLocalize("正在保存… / Saving…"));
  try {
    await ommegaAuthCommit(ommegaAuthOriginal, {...ommegaAuthConfig, default: ommegaAuthDraft});
    ommegaAuthStatus(ommegaUiLocalize("策略已保存，将用于之后新建的密钥。 / Policy saved for newly created keys."));
    document.getElementById("ommega-auth-policy-dialog")?.close();
  } catch (error) {
    ommegaAuthStatus(ommegaUiLocalize("保存失败 / Could not save policy") + ": " + error.message);
  } finally {
    ommegaAuthBusy = false; ommegaAuthSync();
    const policy = document.getElementById("ommega-auth-policy-dialog");
    if (policy && !policy.open) document.getElementById("ommega-auth-open")?.focus();
  }
}

function ommegaAuthFilteredApps(query) {
  const search = String(query).trim().toLocaleLowerCase();
  return ommegaAuthInventory.filter(app => (ommegaAuthShowSystem || !app.system) &&
    (app.appName + " " + app.packageName).toLocaleLowerCase().includes(search));
}

function ommegaAuthLoadIcon(image, packageName) {
  // Both settings lists share the actual app icon pipeline. A failed manager
  // scheme may still have a PackageManager drawable available.
  let schemeTried = false, drawableTried = false;
  const source = value => {
    image.hidden = false; image.style.opacity = "0";
    if (image.previousElementSibling) image.previousElementSibling.hidden = false;
    image.src = value;
  };
  const next = () => {
    if (!schemeTried && typeof globalThis.ksu?.exec === "function") {
      schemeTried = true; source("ksu://icon/" + packageName); return;
    }
    if (!drawableTried && typeof $packageManager !== "undefined" && typeof ks === "function") {
      drawableTried = true;
      Promise.resolve().then(() => ks($packageManager.getApplicationIcon(packageName, 0, 0)))
        .then(response => response.arrayBuffer()).then(buffer => {
          const value = "data:image/png;base64," + Ts(buffer);
          if (typeof dr !== "undefined") dr.set(packageName, value);
          source(value);
        }).catch(next);
      return;
    }
    image.hidden = true;
    if (image.previousElementSibling) image.previousElementSibling.hidden = false;
  };
  image.addEventListener("load", () => {
    image.hidden = false; image.style.opacity = "1";
    if (image.previousElementSibling) image.previousElementSibling.hidden = true;
  });
  image.addEventListener("error", next);
  if (typeof dr !== "undefined" && dr.has(packageName)) source(dr.get(packageName));
  else next();
}

function ommegaAuthRenderApps() {
  const list = document.getElementById("ommega-auth-app-list");
  const matches = ommegaAuthFilteredApps(document.getElementById("ommega-auth-search").value);
  list.replaceChildren();
  for (const app of matches.slice(0, ommegaAuthLimit)) {
    const row = document.createElement("div"); row.className = "ommega-auth-app";
    const picture = document.createElement("span"); picture.className = "ommega-auth-icon"; picture.setAttribute("aria-hidden", "true");
    const fallback = document.createElement("span"); fallback.textContent = [...app.appName][0] || "A";
    const image = document.createElement("img"); image.alt = ""; image.width = 40; image.height = 40;
    image.loading = "lazy"; image.decoding = "async";
    picture.append(fallback, image); ommegaAuthLoadIcon(image, app.packageName);
    const name = document.createElement("span"); name.className = "ommega-auth-app-name";
    const title = document.createElement("strong"); title.textContent = app.appName;
    const pkg = document.createElement("small"); pkg.textContent = app.packageName; name.append(title, pkg);
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
    row.append(picture, name, select); list.appendChild(row);
  }
  const more = document.getElementById("ommega-auth-more");
  more.hidden = matches.length <= ommegaAuthLimit;
  ommegaAuthStatus(ommegaUiLocalize("匹配应用 / Matching apps") + ": " + matches.length, true);
}

async function ommegaAuthReadApps(type) {
  const packages = [...new Set(await Zo(type))].filter(ommegaAuthPackage);
  return (await ommegaNames(packages, {})).map(app => ({...app, system: type === "system"}));
}

async function ommegaAuthOpenApps() {
  if (ommegaAuthBusy || !ommegaAuthConfig) return;
  const session = ++ommegaAuthAppSession;
  ommegaAuthAppsDraft = {...ommegaAuthConfig.apps}; ommegaAuthAppsOriginal = ommegaAuthOriginal;
  ommegaAuthInventory = []; ommegaAuthShowSystem = false; ommegaAuthSystemsLoaded = false;
  ommegaAuthLoadingApps = true; ommegaAuthAppsLoaded = false;
  const dialog = document.getElementById("ommega-auth-dialog");
  document.getElementById("ommega-auth-system").checked = false;
  document.getElementById("ommega-auth-search").value = "";
  document.getElementById("ommega-auth-app-list").replaceChildren();
  document.getElementById("ommega-auth-more").hidden = true;
  dialog.showModal(); ommegaAuthSync();
  ommegaAuthStatus(ommegaUiLocalize("正在读取用户应用… / Loading user apps…"), true);
  try {
    const apps = await ommegaAuthReadApps("user");
    if (session !== ommegaAuthAppSession || !dialog.open) return;
    ommegaAuthInventory = apps;
    ommegaAuthInventory.sort((a, b) => a.appName.localeCompare(b.appName));
    ommegaAuthAppsLoaded = true; ommegaAuthLimit = 40; ommegaAuthRenderApps();
  } catch (error) {
    if (session === ommegaAuthAppSession && dialog.open) ommegaAuthStatus(ommegaUiLocalize("读取应用失败，请返回后重试 / Could not load apps; go back and retry") + ": " + error.message, true);
  } finally {
    if (session === ommegaAuthAppSession) { ommegaAuthLoadingApps = false; ommegaAuthSync(); }
  }
}

async function ommegaAuthToggleSystem() {
  ommegaAuthShowSystem = document.getElementById("ommega-auth-system").checked;
  ommegaAuthLimit = 40;
  if (!ommegaAuthShowSystem || ommegaAuthSystemsLoaded) { ommegaAuthRenderApps(); return; }
  if (ommegaAuthLoadingApps) return;
  const session = ommegaAuthAppSession;
  ommegaAuthLoadingApps = true;
  ommegaAuthStatus(ommegaUiLocalize("正在读取系统应用… / Loading system apps…"), true);
  try {
    const apps = await ommegaAuthReadApps("system");
    if (session !== ommegaAuthAppSession) return;
    const byPackage = new Map(ommegaAuthInventory.map(app => [app.packageName, app]));
    for (const app of apps) if (!byPackage.has(app.packageName)) byPackage.set(app.packageName, app);
    ommegaAuthInventory = [...byPackage.values()].sort((a, b) => a.appName.localeCompare(b.appName));
    ommegaAuthSystemsLoaded = true; ommegaAuthRenderApps();
  } catch (error) {
    if (session === ommegaAuthAppSession) {
      ommegaAuthShowSystem = false; document.getElementById("ommega-auth-system").checked = false;
      ommegaAuthStatus(ommegaUiLocalize("读取系统应用失败 / Could not load system apps") + ": " + error.message, true);
    }
  } finally {
    if (session === ommegaAuthAppSession) { ommegaAuthLoadingApps = false; ommegaAuthSync(); }
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
    if (document.getElementById("ommega-auth-policy-dialog").open && !document.getElementById("ommega-auth-dialog").open) document.getElementById("ommega-auth-apps").focus();
  }
}

function ommegaInstallAuthSigning() {
  const slot = document.getElementById("om-auth-signing-slot");
  if (!slot || typeof slot.appendChild !== "function" || document.getElementById("ommega-auth-signing")) return false;
  const panel = document.createElement("section"); panel.id = "ommega-auth-signing";
  const back = '<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="m14 5-7 7 7 7"/></svg>';
  const forward = '<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="m9 5 7 7-7 7"/></svg>';
  panel.innerHTML = ommegaUiLocalizeHtml(
    '<button type="button" id="ommega-auth-open" class="om-action"><span>认证签名策略 / Authentication signing<small id="ommega-auth-summary">正在读取… / Loading…</small></span>' + forward + '</button>' +
    '<dialog id="ommega-auth-policy-dialog" class="ommega-auth-modal" aria-labelledby="ommega-auth-heading" data-om-return-focus="ommega-auth-open">' +
    '<header class="ommega-auth-toolbar"><button type="button" id="ommega-auth-policy-back" class="om-button secondary">' + back + '<span>返回 / Back</span></button><h2 id="ommega-auth-heading">认证签名策略 / Authentication signing</h2></header>' +
    '<div class="ommega-auth-policy-content"><fieldset aria-labelledby="ommega-auth-heading">' +
    '<label><input type="radio" name="ommega-auth-mode" value="auto"><span><strong>自动（推荐） / Auto (recommended)</strong><small>识别已知认证检测探针，其余请求优先支付兼容。 / Preserve known authentication probes; use payment compatibility for other eligible requests.</small></span></label>' +
    '<label><input type="radio" name="ommega-auth-mode" value="compatibility"><span><strong>支付兼容 / Payment compatibility</strong><small>在本机验证认证，由远端签名。 / Authenticate on this device and sign remotely.</small></span></label>' +
    '<label><input type="radio" name="ommega-auth-mode" value="preserve"><span><strong>保持认证要求 / Preserve authentication</strong><small>远端保留认证要求，部分应用的指纹签名可能失败。 / Keep authentication requirements on the remote key; fingerprint signing may fail in some apps.</small></span></label>' +
    '</fieldset><p class="ommega-auth-note">仅影响之后新建的密钥，已有密钥不会自动转换。 / Only affects newly created keys. Existing keys are not converted.</p>' +
    '<button type="button" id="ommega-auth-apps" class="om-action">单应用例外 / App exceptions</button>' +
    '<div id="ommega-auth-status" role="status" aria-live="polite"></div>' +
    '<div class="ommega-auth-actions"><button type="button" id="ommega-auth-reload" class="om-button secondary">重新读取 / Reload</button><button type="button" id="ommega-auth-save" class="om-button">保存策略 / Save policy</button></div></div></dialog>' +
    '<dialog id="ommega-auth-dialog" class="ommega-auth-modal" aria-labelledby="ommega-auth-app-heading" data-om-return-focus="ommega-auth-apps">' +
    '<header class="ommega-auth-toolbar"><button type="button" id="ommega-auth-app-back" class="om-button secondary">' + back + '<span>返回 / Back</span></button><h2 id="ommega-auth-app-heading">单应用例外 / App exceptions</h2><button type="button" id="ommega-auth-app-save" class="om-button">保存 / Save</button></header>' +
    '<div class="ommega-auth-app-filters"><label for="ommega-auth-search">搜索应用名称或包名 / Search app name or package</label><input id="ommega-auth-search" type="search" autocomplete="off">' +
    '<label class="ommega-auth-system-option"><input id="ommega-auth-system" type="checkbox" role="switch"><span>显示系统应用 / Show system apps</span></label>' +
    '<small>例外仅对作用域内应用生效；全局作用域下仍然有效。 / Exceptions apply only to apps in scope and remain active in global scope.</small>' +
    '<div id="ommega-auth-app-status" role="status" aria-live="polite"></div></div>' +
    '<div id="ommega-auth-app-list"></div><button type="button" id="ommega-auth-more" class="om-button secondary" hidden>显示更多 / Show more</button></dialog>');
  slot.appendChild(panel);
  const byId = id => document.getElementById(id);
  byId("ommega-auth-open").addEventListener("click", ommegaAuthOpenPolicy);
  byId("ommega-auth-save").addEventListener("click", ommegaAuthSaveDefault);
  byId("ommega-auth-reload").addEventListener("click", ommegaAuthRefresh);
  byId("ommega-auth-apps").addEventListener("click", ommegaAuthOpenApps);
  byId("ommega-auth-app-save").addEventListener("click", ommegaAuthSaveApps);
  byId("ommega-auth-policy-back").addEventListener("click", () => byId("ommega-auth-policy-dialog").close());
  byId("ommega-auth-app-back").addEventListener("click", () => byId("ommega-auth-dialog").close());
  byId("ommega-auth-policy-dialog").addEventListener("close", () => byId("ommega-auth-open").focus());
  byId("ommega-auth-dialog").addEventListener("close", () => {
    ++ommegaAuthAppSession; ommegaAuthLoadingApps = false;
    if (byId("ommega-auth-policy-dialog").open) byId("ommega-auth-apps").focus();
  });
  byId("ommega-auth-search").addEventListener("input", () => { ommegaAuthLimit = 40; ommegaAuthRenderApps(); });
  byId("ommega-auth-system").addEventListener("change", ommegaAuthToggleSystem);
  byId("ommega-auth-more").addEventListener("click", () => { ommegaAuthLimit += 40; ommegaAuthRenderApps(); });
  for (const radio of panel.querySelectorAll('input[name="ommega-auth-mode"]')) radio.addEventListener("change", () => {
    ommegaAuthDraft = radio.value;
    ommegaAuthStatus(ommegaUiLocalize("策略尚未保存 / Policy not saved yet"));
  });
  ommegaAuthRefresh(); return true;
}

ommegaInstallAuthSigning();
if (typeof window !== "undefined") window.addEventListener("ommega-settings-ready", ommegaInstallAuthSigning);
