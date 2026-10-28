// Injected only by preview.mjs. Never copied into the module webroots.
const apps = [{packageName:'com.example.wallet',appLabel:'Wallet',isSystem:false,uid:10101},{packageName:'com.example.checker',appLabel:'Attestation Checker',isSystem:false,uid:10102},{packageName:'com.example.chat',appLabel:'Chat',isSystem:false,uid:10103},
  {packageName:'com.lingqing.trustattestor',appLabel:'TrustAttestor',isSystem:false,uid:10104},
  {packageName:'com.ommega.authrepro',appLabel:'Auth Repro',isSystem:false,uid:10105},
  {packageName:'com.android.settings',appLabel:'Android Settings',isSystem:true,uid:1000},
  {packageName:'com.google.android.gms',appLabel:'Google Play services',isSystem:true,uid:10106}];
let authPolicy = null;
let capabilityPolicy = null;
let capabilityConfig = {strongbox:false, device_id_attestation:false};
let injectorConfig = 'version = 1\n[filter]\nenabled = true\n';
let targetCompat = null;
let targetList = 'com.example.wallet\ncom.example.checker\n';
let targetSecurity = null;
let systemApps = null;
let cpuOriginal = null;
let cpuConfig = {enabled:false,packages:[]};
const cpuProfile = {properties:{'ro.product.model':'Demo B','ro.product.brand':'Example','ro.product.manufacturer':'Example','ro.product.device':'demo_device','ro.product.name':'demo_product','ro.soc.model':'Demo SoC'}};
const utf8Base64 = value => btoa(String.fromCharCode(...new TextEncoder().encode(value)));
const utf8Decode = value => new TextDecoder().decode(Uint8Array.from(atob(value), c=>c.charCodeAt(0)));
// Native managers serve this scheme; the browser preview uses local sample icons.
const srcDescriptor = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, 'src');
Object.defineProperty(HTMLImageElement.prototype, 'src', {get:srcDescriptor.get,set(value) {
  if (String(value).startsWith('ksu://icon/')) {
    const label = (apps.find(app=>String(value).endsWith(app.packageName))?.appLabel || 'A')[0];
    value = 'data:image/svg+xml;base64,' + btoa('<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" rx="12" fill="#215e51"/><text x="24" y="32" text-anchor="middle" fill="white" font-family="sans-serif" font-size="25">'+label+'</text></svg>');
  }
  srcDescriptor.set.call(this,value);
}});
window.__previewCommands = [];
window.ksu = {
  exec(command,options,callback) {
    window.__previewCommands.push(command);
    const now = Math.floor(Date.now()/1000);
    let stdout = '', errno = 0;
    if (command.includes('ommega-capability-policy')) {
      if (command.includes('base64 -d')) {
        const match = command.match(/printf '%s' '([^']+)'/);
        const request = match && JSON.parse(utf8Decode(match[1]));
        if (!request || request.expected !== capabilityPolicy) {errno=75; stdout='';}
        else {
          capabilityPolicy = request.replacement;
          capabilityConfig = {
            strongbox:/^strongbox=true$/m.test(capabilityPolicy),
            device_id_attestation:/^device_id_attestation=true$/m.test(capabilityPolicy),
          };
          stdout='saved';
        }
      } else stdout = JSON.stringify({original:capabilityPolicy, config:capabilityConfig,
        mounted:window.__OM_CAPABILITY_MOUNT__ === undefined ? {strongbox:false,device_id_attestation:false} : window.__OM_CAPABILITY_MOUNT__, mount_error:null});
    }
    else if (command.includes('ommega_app_writer=') && command.includes('base64 -d')) {
      const match = command.match(/printf '%s' '([^']+)'/);
      const request = match && JSON.parse(utf8Decode(match[1]));
      const isTarget = command.endsWith(' write-target');
      const isSecurity = command.endsWith(' write-target-security');
      const isSystem = command.endsWith(' write-system-apps');
      const current = isTarget ? targetList : isSecurity ? targetSecurity : isSystem ? systemApps : undefined;
      if ((!isTarget && !isSecurity && !isSystem) || !request || request.expected !== current) {errno=75; stdout='';}
      else {if (isTarget) targetList=request.replacement; else if (isSecurity) targetSecurity=request.replacement; else systemApps=request.replacement; stdout='saved';}
    }
    else if (command.includes('ommega-auth-policy') && command.includes('base64 -d')) {
      const match = command.match(/printf '%s' '([^']+)'/);
      const request = match && JSON.parse(utf8Decode(match[1]));
      if (!request || request.expected !== authPolicy) {errno=75; stdout='';}
      else {authPolicy=request.replacement;stdout='saved';}
    }
    else if (command.includes('ommega-cpu-policy') && !command.includes('[HEALTH]')) {
      if (command.includes('base64 -d')) {
        const match = command.match(/printf '%s' '([^']+)'/);
        const request = match && JSON.parse(utf8Decode(match[1]));
        if (!request || request.expected !== cpuOriginal) {errno=75; stdout='';}
        else {
          cpuOriginal = request.replacement;
          cpuConfig = {enabled:/^enabled=true$/m.test(cpuOriginal), packages:[...cpuOriginal.matchAll(/^package=(.+)$/gm)].map(item=>item[1])};
          stdout='saved';
        }
      } else stdout=JSON.stringify({original:cpuOriginal,config:cpuConfig,profile:cpuProfile,engine:'exp6',standalone_active:false,
        attestation_counts:window.__OM_ATTESTATION_COUNTS__ || {'com.example.wallet':{count:12,shared:false},'com.example.checker':{count:3,shared:true}}});
    }
    else if (command.includes('system_app')) {
      stdout = command.includes('printf "M:"') ? systemApps === null ? 'M:' : 'F:' + utf8Base64(systemApps) : systemApps || '';
    }
    else if (command === 'pm list packages -3') stdout = apps.filter(app=>!app.isSystem).map(app=>'package:' + app.packageName).join('\n');
    else if (command.startsWith('pm list packages -s ')) {
      const pkg = command.slice('pm list packages -s '.length);
      stdout = apps.some(app => app.isSystem && app.packageName === pkg) ? 'package:' + pkg : '';
    }
    else if (command.includes('target-security.toml')) {
      stdout = command.includes('printf "M:"')
        ? targetSecurity === null ? 'M:' : 'F:' + utf8Base64(targetSecurity)
        : targetSecurity || '';
    }
    else if (command.includes('target-compat.toml')) {
      if (command.includes('ommega_compat_current=')) {
        const before = command.match(/\[ "\$ommega_compat_current" = '([^']+)' \]/);
        const after = command.match(/printf '%s' '([^']+)'/);
        const current = targetCompat === null ? 'M:' : 'F:' + utf8Base64(targetCompat);
        if (!before || !after || before[1] !== current) {errno=75; stdout='';}
        else {targetCompat = utf8Decode(after[1]); stdout='saved';}
      } else if (command.includes('printf "M:"')) stdout = targetCompat === null ? 'M:' : 'F:' + utf8Base64(targetCompat);
      else stdout = targetCompat || '';
    }
    else if (command.includes('auth-signing.toml')) stdout = authPolicy === null ? 'M:' : 'F:' + utf8Base64(authPolicy);
    else if (command.startsWith('base64 < ') && command.includes('injector.toml')) stdout = utf8Base64(injectorConfig);
    else if (command.includes('ommega_scope_file=')) {
      const before = command.match(/\[ "\$ommega_scope_current" = '([^']*)' \]/);
      const after = command.match(/printf '%s' '([^']+)'/);
      if (!before || !after || before[1] !== utf8Base64(injectorConfig)) errno=75;
      else injectorConfig=utf8Decode(after[1]);
    }
    else if (command.includes('ommega_status_config=')) stdout='url: https://relay.example.com:8443\ndevice_id: dedicated-b\nremote: true\ntls_insecure: false\n';
    else if (command.includes('ommega_status_url=')) stdout=JSON.stringify(window.__OM_B_STATUS__ || {status:'ok',online_b_devices:[{device_id:'dedicated-b',battery_percent:76,battery_charging:true,battery_updated_at_ms:Date.now()}]});
    else if (command.includes('[HEALTH]')) stdout = '\n[HEALTH]\n'+(window.__OM_HEALTH__ || JSON.stringify({last_ok_unix:now-120,last_error_unix:now-3600,last_status:500,last_kind:'error',last_message:JSON.stringify({error:'all fulfilment layers failed for device op11-b: no key for alias ommega-remote-c83964f4bf8534c1 (call attest first)'}),last_verdict:'ok'}))+'\n[SOTER]\n'+JSON.stringify({enabled:1,running:1,ledger:1,updated:now})+'\n[VERSION]\nversion=1.8.0-preview\n[DAEMON]\n1924\n[PROFILE]\n'+JSON.stringify({profile:cpuProfile,last_b_latency_ms:100})+'\n[REMOTE_DEVICE]\ndedicated-b\n[TARGET]\npresent\ncom.example.wallet\ncom.example.checker\n[END]\n';
    else if (command.includes('PIDS=')) stdout = 'PIDS=14680\nSPL=2026-08-01\nOS=16\nVERSION=2.0.0-preview\n';
    else if (command.includes('payload=$(base64')) stdout = window.__OM_FIXTURE__.digest+'\n'+window.__OM_FIXTURE__.raw+'\n';
    else if (command.includes('spl-control.sh') && command.includes('status')) stdout = 'SYSTEM_SPL=2026-08-01\nBOOT_SPL=2025-12-01\nVENDOR_SPL=2026-08-01\nOS_VERSION=16\nORIGIN_SYSTEM_SPL=2025-12-01\nORIGIN_VENDOR_SPL=2025-12-01\nORIGIN_OS_VERSION=16\nFLOOR_SYSTEM_SPL=2026-08-01\nFLOOR_VENDOR_SPL=2026-08-01\nFLOOR_OS_VERSION=16\nAPPLIED_SYSTEM_SPL=2026-08-01\nAPPLIED_BOOT_SPL=2025-12-01\nAPPLIED_VENDOR_SPL=2026-08-01\nAPPLIED_OS_VERSION=16\n';
    else if (/\b(mv|touch|rm|mkdir|chmod|stop|start)\b|\bsave\s| > /.test(command)) errno = 1;
    else if (command.includes('target.txt')) stdout = command.includes('printf "M:"')
      ? targetList === null ? 'M:' : 'F:' + utf8Base64(targetList)
      : targetList || '';
    else if (command.includes('module.prop')) stdout = command.includes("grep '^version='")
      ? '1.8.0-preview\n'
      : 'name=Ommega\nversion=1.8.0-preview\nversionCode=28\n';
    else if (command.includes('getprop')) stdout = '17\n';
    setTimeout(()=>window[callback]?.(errno,stdout,errno?'Preview is read only':''),20);
  },
  listPackages: type => JSON.stringify(apps.filter(app=>type==='user'?!app.isSystem:type==='system'?app.isSystem:true).map(app=>app.packageName)),
  getPackagesInfo: packages => JSON.stringify(apps.filter(app=>JSON.parse(packages).includes(app.packageName))),
  toast: () => {}, spawn: () => {},
};
document.addEventListener('DOMContentLoaded',() => {
  const note = document.createElement('div'); note.textContent = /^zh/.test(navigator.language)?'界面预览 · 所有状态均为示例数据':'UI preview · all status values are sample data';
  note.style.cssText = 'padding:10px;text-align:center;font:12px system-ui;background:#17473e;color:#b9f9e5';
  document.body.prepend(note);
});
