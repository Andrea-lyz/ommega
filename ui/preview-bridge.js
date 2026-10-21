// Injected only by preview.mjs. Never copied into the module webroots.
const apps = [{packageName:'com.example.wallet',appLabel:'Wallet',isSystem:false,uid:10101},{packageName:'com.example.checker',appLabel:'Attestation Checker',isSystem:false,uid:10102},{packageName:'com.example.chat',appLabel:'Chat',isSystem:false,uid:10103}];
window.__previewCommands = [];
window.ksu = {
  exec(command,options,callback) {
    window.__previewCommands.push(command);
    const now = Math.floor(Date.now()/1000);
    let stdout = '', errno = 0;
    if (command.includes('[HEALTH]')) stdout = '\n[HEALTH]\n'+JSON.stringify({last_ok_unix:now-24})+'\n[SOTER]\n'+JSON.stringify({enabled:1,running:1,ledger:1,updated:now})+'\n[VERSION]\nversion=1.8.0-preview\n[DAEMON]\n1924\n[END]\n';
    else if (command.includes('PIDS=')) stdout = 'PIDS=14680\nSPL=2026-08-01\nOS=16\n';
    else if (command.includes('payload=$(base64')) stdout = window.__OM_FIXTURE__.digest+'\n'+window.__OM_FIXTURE__.raw+'\n';
    else if (command.includes('spl-control.sh') && command.includes('status')) stdout = 'SYSTEM_SPL=2026-08-01\nBOOT_SPL=2025-12-01\nVENDOR_SPL=2026-08-01\nOS_VERSION=16\nORIGIN_SYSTEM_SPL=2025-12-01\nORIGIN_VENDOR_SPL=2025-12-01\nORIGIN_OS_VERSION=16\n';
    else if (/\b(mv|touch|rm|mkdir|chmod|stop|start)\b|\bsave\s| > /.test(command)) errno = 1;
    else if (command.includes('target.txt')) stdout = 'com.example.wallet\ncom.example.checker\n';
    else if (command.includes('module.prop')) stdout = 'name=Ommega\nversion=1.8.0-preview\nversionCode=28\n';
    else if (command.includes('getprop')) stdout = '17\n';
    setTimeout(()=>window[callback]?.(errno,stdout,errno?'Preview is read only':''),20);
  },
  listPackages: () => JSON.stringify(apps.map(app=>app.packageName)),
  getPackagesInfo: () => JSON.stringify(apps),
  toast: () => {}, spawn: () => {},
};
document.addEventListener('DOMContentLoaded',() => {
  const note = document.createElement('div'); note.textContent = /^zh/.test(navigator.language)?'界面预览 · 所有状态均为示例数据':'UI preview · all status values are sample data';
  note.style.cssText = 'padding:10px;text-align:center;font:12px system-ui;background:#17473e;color:#b9f9e5';
  document.body.prepend(note);
});
