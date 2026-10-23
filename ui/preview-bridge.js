// Injected only by preview.mjs. Never copied into the module webroots.
const apps = [{packageName:'com.example.wallet',appLabel:'Wallet',isSystem:false,uid:10101},{packageName:'com.example.checker',appLabel:'Attestation Checker',isSystem:false,uid:10102},{packageName:'com.example.chat',appLabel:'Chat',isSystem:false,uid:10103},
  {packageName:'com.lingqing.trustattestor',appLabel:'TrustAttestor',isSystem:false,uid:10104},
  {packageName:'com.ommega.authrepro',appLabel:'Auth Repro',isSystem:false,uid:10105},
  {packageName:'com.android.settings',appLabel:'Android Settings',isSystem:true,uid:1000},
  {packageName:'com.google.android.gms',appLabel:'Google Play services',isSystem:true,uid:10106}];
let authPolicy = null;
let injectorConfig = 'version = 1\n[filter]\nenabled = true\n';
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
    if (command.includes('ommega-auth-policy') && command.includes('base64 -d')) {
      const match = command.match(/printf '%s' '([^']+)'/);
      const request = match && JSON.parse(utf8Decode(match[1]));
      if (!request || request.expected !== authPolicy) {errno=75; stdout='';}
      else {authPolicy=request.replacement;stdout='saved';}
    }
    else if (command.includes('auth-signing.toml')) stdout = authPolicy === null ? 'M:' : 'F:' + utf8Base64(authPolicy);
    else if (command.startsWith('base64 < ') && command.includes('injector.toml')) stdout = utf8Base64(injectorConfig);
    else if (command.includes('ommega_scope_file=')) {
      const before = command.match(/\[ "\$ommega_scope_current" = '([^']*)' \]/);
      const after = command.match(/printf '%s' '([^']+)'/);
      if (!before || !after || before[1] !== utf8Base64(injectorConfig)) errno=75;
      else injectorConfig=utf8Decode(after[1]);
    }
    else if (command.includes('[HEALTH]')) stdout = '\n[HEALTH]\n'+(window.__OM_HEALTH__ || JSON.stringify({last_ok_unix:now-120,last_error_unix:now-3600,last_status:500,last_kind:'error',last_message:JSON.stringify({error:'all fulfilment layers failed for device op11-b: no key for alias ommega-remote-c83964f4bf8534c1 (call attest first)'}),last_verdict:'ok'}))+'\n[SOTER]\n'+JSON.stringify({enabled:1,running:1,ledger:1,updated:now})+'\n[VERSION]\nversion=1.8.0-preview\n[DAEMON]\n1924\n[END]\n';
    else if (command.includes('PIDS=')) stdout = 'PIDS=14680\nSPL=2026-08-01\nOS=16\n';
    else if (command.includes('payload=$(base64')) stdout = window.__OM_FIXTURE__.digest+'\n'+window.__OM_FIXTURE__.raw+'\n';
    else if (command.includes('spl-control.sh') && command.includes('status')) stdout = 'SYSTEM_SPL=2026-08-01\nBOOT_SPL=2025-12-01\nVENDOR_SPL=2026-08-01\nOS_VERSION=16\nORIGIN_SYSTEM_SPL=2025-12-01\nORIGIN_VENDOR_SPL=2025-12-01\nORIGIN_OS_VERSION=16\nFLOOR_SYSTEM_SPL=2026-08-01\nFLOOR_VENDOR_SPL=2026-08-01\nFLOOR_OS_VERSION=16\nAPPLIED_SYSTEM_SPL=2026-08-01\nAPPLIED_BOOT_SPL=2025-12-01\nAPPLIED_VENDOR_SPL=2026-08-01\nAPPLIED_OS_VERSION=16\n';
    else if (/\b(mv|touch|rm|mkdir|chmod|stop|start)\b|\bsave\s| > /.test(command)) errno = 1;
    else if (command.includes('target.txt')) stdout = 'com.example.wallet\ncom.example.checker\n';
    else if (command.includes('module.prop')) stdout = 'name=Ommega\nversion=1.8.0-preview\nversionCode=28\n';
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
