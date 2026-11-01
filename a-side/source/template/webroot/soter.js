// Full-page SOTER identities. Private keys are read and validated only by Rust.
import {icon,text,createBridge} from './ui-kit.js?v=material-ui-29';
const TYPES=[['local','本地随机身份','Local random'],['key','私钥身份','Private key'],['relay','远程中继身份','Remote relay']];
let exec=null,writeExec=null,snapshot=null,adding=false,generation=0,busy=false;
function esc(value){return String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));}
function label(type){const row=TYPES.find(row=>row[0]===type);return row?text(row[1],row[2]):type;}
function injectStyle() {
  if (document.getElementById('om-soter-style')) return;
  const style = document.createElement('style');
  style.id = 'om-soter-style';
  style.textContent = [
    '.om-soter-toolbar{display:flex;gap:8px;margin:0 0 14px}a.action-row{text-decoration:none;color:inherit}.om-soter-list{display:flex;flex-direction:column;gap:10px}',
    '.om-soter-row{display:flex;align-items:center;gap:12px;width:100%;text-align:left;padding:16px 18px;border:1px solid var(--om-line);border-radius:16px;background:var(--om-raised);color:var(--om-ink)}',
    '.om-soter-row .name{font-weight:650;font-size:15px}',
    '.om-soter-row .meta{font-size:12px;color:var(--om-muted);margin-top:3px;overflow-wrap:anywhere}',
    '.om-soter-row .tag{margin-left:auto;flex:0 0 auto;font-size:11px;padding:4px 9px;border-radius:999px;background:var(--om-card);color:var(--om-muted)}',
    '.om-soter-types{display:grid;gap:10px;grid-template-columns:repeat(3,minmax(0,1fr));margin-top:12px}',
    '.om-soter-type{display:flex;flex-direction:column;gap:6px;padding:14px;border:1px solid var(--om-line);border-radius:16px;background:var(--om-raised);color:var(--om-ink);text-align:left}',
    '.om-soter-type b{font-size:14px}.om-soter-type small{font-size:11px;color:var(--om-muted);line-height:1.4}',
    '.om-soter-field{display:flex;flex-direction:column;gap:6px;margin:12px 0}',
    '.om-soter-field label{font-size:12px;color:var(--om-muted)}',
    '.om-soter-field input{min-height:44px;padding:10px 12px;border-radius:12px;border:1px solid var(--om-line);background:var(--om-raised);color:var(--om-ink);font-size:13px}',
    '.om-soter-kv{display:grid;grid-template-columns:110px minmax(0,1fr);gap:6px 12px;font-size:13px;margin:10px 0}',
    '.om-soter-kv .k{color:var(--om-muted)}.om-soter-kv .v{overflow-wrap:anywhere}',
    '.om-soter-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}',
    '.om-soter-msg{font-size:12px;color:var(--om-muted);line-height:1.5;margin:10px 0 0;overflow-wrap:anywhere}',
  ].join('');
  document.head.appendChild(style);
}

function encode(value){return btoa(Array.from(new TextEncoder().encode(JSON.stringify(value)),b=>String.fromCharCode(b)).join(''));}
export async function policyCall(operation,payload=null,packageName=null){
  if(!['soter-read','soter-write','soter-import'].includes(operation))throw new Error('Unsupported SOTER operation');
  if(packageName!==null&&(operation!=='soter-read'||!/^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$/.test(packageName)))throw new Error('Invalid SOTER package');
  const command=['set -e','w=/data/adb/modules/ommega/libs/arm64-v8a/ommega-auth-policy','[ -x "$w" ] || w=/data/adb/modules/ommega/libs/x86_64/ommega-auth-policy','[ -x "$w" ] || { echo "Install the complete updated A module" >&2; exit 1; }',
    payload===null?'"$w" '+operation+(packageName===null?'':' "'+packageName+'"'):'printf %s \''+encode(payload)+'\' | base64 -d | "$w" '+operation].join('\n');
  if(command.length>120000)throw new Error(text('配置过大，无法通过模块管理器安全保存，请使用设备端管理工具。','Policy is too large for the manager bridge; use the on-device management tool.'));
  const result=await (operation==='soter-read'?exec:writeExec)(command);
  if(!result||Number(result.errno)!==0)throw new Error(result?.stderr||text('SOTER 操作失败','SOTER operation failed'));
  let response;try{response=JSON.parse(result.stdout);}catch{throw new Error(text('管理接口返回格式错误','Invalid management response'));}
  if(!response?.store||!Array.isArray(response.store.identities)||!response.status)throw new Error('Invalid SOTER policy response');
  if(operation!=="soter-read"&&response.warnings?.length)window.alert?.(response.warnings.join("\n"));
  return response;
}
function request(base,store){return {expected:base.expected??null,legacy_revision:base.legacy_revision??null,store};}
function currentHost(){return document.getElementById('om-soter-identity')?.hidden===false?document.getElementById('om-soter-identity'):document.getElementById('om-soter-page');}
function message(value){const node=currentHost()?.querySelector?.('[data-soter-message]')||document.getElementById('om-soter-msg');if(node)node.textContent=value;}
async function action(run){
  if(busy)return;busy=true;
  const host=currentHost();host?.querySelectorAll('button,input').forEach(n=>{n.dataset.soterDisabled=n.disabled?'1':'0';n.disabled=true;});
  try{await run();}catch(error){message(String(error.message||error));}finally{busy=false;host?.querySelectorAll('[data-soter-disabled]').forEach(n=>{n.disabled=n.dataset.soterDisabled==='1';delete n.dataset.soterDisabled;});}
}
function header(title,description,back,extra=''){return '<div class="page-heading"><div><span class="section-kicker">SOTER</span><h1>'+esc(title)+'</h1><p>'+esc(description)+'</p></div></div><div class="om-soter-toolbar"><button type="button" class="om-button secondary" id="'+back+'">'+text('返回','Back')+'</button>'+extra+'</div>';}
function panel(title,body){return '<article class="panel settings-group"><div class="group-header"><h2 class="group-heading">'+esc(title)+'</h2></div>'+body+'</article>';}
function note(message){return '<p class="note" style="padding:0 20px 18px">'+esc(message)+'</p>';}
function row(item){const status=snapshot.status[item.id];const meta=label(item.type)+(status?.ready?'':text(' · 尚未配置完整',' · incomplete'));
  return '<a class="action-row" href="#om-soter/'+encodeURIComponent(item.id)+'"><span class="row-icon">'+icon(item.type==='key'?'key':item.type==='relay'?'server':'shield')+'</span><span class="row-text"><strong>'+esc(item.name)+(snapshot.store.default===item.id?text(' · 默认',' · default'):'')+'</strong><small>'+esc(meta)+'</small></span>'+icon('chevron')+'</a>';
}
function errorPage(host,error,back,target,retry){host.innerHTML=header('SOTER',String(error.message||error),back)+'<button type="button" class="om-button secondary" id="om-soter-retry">'+text('重新读取','Retry')+'</button>';document.getElementById(back).onclick=()=>{location.hash=target;};document.getElementById('om-soter-retry').onclick=retry;}
/// Ask the watchdog for the software TA: the flag is the only request it reads,
/// and it applies the change on its own poll.
function flagCommand(on){
  return on
    ? ['set -e','mkdir -p /data/adb/ommega/soterta','date +%s > /data/adb/ommega/soterta/enabled.tmp','chmod 0600 /data/adb/ommega/soterta/enabled.tmp','mv /data/adb/ommega/soterta/enabled.tmp /data/adb/ommega/soterta/enabled'].join('\n')
    : 'rm -f /data/adb/ommega/soterta/enabled';
}

function switchHtml(enabled,ta){
  const state = enabled
    ? (ta&&ta.running?text('软件 TA 运行中','Software TA running'):text('已启用，等待看门狗接管…','Enabled; waiting for the watchdog'))
    : text('已关闭：所有应用回落到原厂 HAL，SOTER 检查会失败','Off: every app falls back to the stock HAL and SOTER checks fail');
  const id = ta&&ta.device_id&&ta.device_id!=='-'?' · '+esc(ta.device_id):'';
  return panel(text('SOTER 总控','SOTER master switch'),
    '<label class="policy-row"><span class="toggle-copy"><strong>'+text('启用 SOTER','Enable SOTER')+'</strong><small>'+
    text('关闭后所有应用都不再使用软件 TA；应用里的身份分配会保留，但不会生效。','When off no app uses the software TA; per-app assignments are kept but inactive.')+
    '</small></span><span class="switch"><input type="checkbox" id="om-soter-enabled" role="switch" aria-label="'+esc(text('启用 SOTER','Enable SOTER'))+'"'+(enabled?' checked':'')+'><span></span></span></label>'+
    '<p class="note" id="om-soter-switch-state" style="padding:0 20px 16px">'+state+id+'</p>');
}

/// The flag is the whole request; the watchdog publishes the outcome a moment later,
/// so the page samples the status twice instead of pretending the change is instant.
function applySwitch(on){
  return action(async()=>{
    const result=await writeExec(flagCommand(on));
    if(!result||Number(result.errno)!==0)throw new Error(result?.stderr||text('写入开关失败','Failed to write the switch'));
    message(text('开关已写入，等待看门狗应用…','Switch written; waiting for the watchdog…'));
    await new Promise(resolve=>setTimeout(resolve,3000));
    await renderList();
    await new Promise(resolve=>setTimeout(resolve,6000));
    await renderList();
  });
}

async function renderList(){
  const host=document.getElementById('om-soter-page');if(!host)return;const turn=++generation;
  try{const fresh=await policyCall('soter-read');const ta=await taStatus();if(turn!==generation)return;snapshot=fresh;
    const types=TYPES.map(t=>'<button type="button" class="action-row" data-add="'+t[0]+'"><span class="row-icon">'+icon(t[0]==='key'?'key':t[0]==='relay'?'server':'shield')+'</span><span class="row-text"><strong>'+text(t[1],t[2])+'</strong></span>'+icon('chevron')+'</button>').join('');
    host.innerHTML=header(text('SOTER 身份','SOTER identities'),text('在应用策略中选择身份；错误配置不会自动换用其他身份。','Choose an identity per app; configuration errors never select another identity.'),'om-soter-back','<button class="om-button" id="om-soter-add" type="button">+ '+text('添加身份','Add identity')+'</button>')+
      '<div class="stack">'+switchHtml(fresh.enabled!==false,ta)+(adding?panel(text('选择身份类型','Choose a type'),types):'')+panel(text('已添加的身份','Identities'),fresh.store.identities.length?fresh.store.identities.map(row).join(''):note(text('还没有身份，点击「添加身份」创建。','No identities yet. Choose Add identity.')))+'</div><p id="om-soter-list-msg" data-soter-message class="om-soter-msg" role="status"></p>';
    document.getElementById('om-soter-back').onclick=()=>history.back();
    document.getElementById('om-soter-add').onclick=()=>{if(!busy){adding=!adding;renderList();}};
    const master=document.getElementById('om-soter-enabled');
    if(master)master.onchange=()=>{
      const on=master.checked;
      if(!on&&!window.confirm(text('关闭后所有应用的 SOTER 都会失效，直到再次启用。继续？','Turning SOTER off disables it for every app until you enable it again. Continue?'))){master.checked=true;return;}
      applySwitch(on);
    };
    host.onclick=event=>{if(busy){event.preventDefault();return;}const button=event.target.closest?.('[data-add]');if(!button)return;
      action(async()=>{const base=snapshot;const store=JSON.parse(JSON.stringify(base.store));const id=button.dataset.add+'-'+crypto.randomUUID().replaceAll('-','');store.identities.push({id,name:text('身份 ','Identity ')+(store.identities.length+1),type:button.dataset.add});
        snapshot=await policyCall('soter-write',request(base,store));adding=false;location.hash='om-soter/'+id;});};
  }catch(error){if(turn===generation)errorPage(host,error,'om-soter-back','om-settings',renderList);}
}
function field(id,title,value='',type='text',extra=''){return '<div class="om-soter-field"><label for="'+id+'">'+esc(title)+'</label><input id="'+id+'" type="'+type+'" value="'+esc(value)+'" '+extra+'></div>';}
async function taStatus(){const result=await exec('cat /data/adb/ommega/soterta/status.json 2>/dev/null');if(Number(result?.errno)!==0)return null;try{return JSON.parse(result.stdout);}catch{return null;}}
async function renderDetail(id){
  const host=document.getElementById('om-soter-identity');if(!host)return;const turn=++generation;
  try{const fresh=await policyCall('soter-read');const ta=await taStatus();if(turn!==generation)return;snapshot=fresh;
    const item=fresh.store.identities.find(i=>i.id===id);if(!item)throw new Error(text('身份不存在，请返回列表。','Identity not found; return to the list.'));
    const status=fresh.status[id];const values=[[text('类型','Type'),label(item.type)],[text('配置','Configuration'),status?.ready?text('完整','Ready'):status?.note||text('未知','Unknown')]];
    if(ta)values.push([text('软件 TA','Software TA'),ta.running?text('运行中','Running'):text('未运行','Stopped')]);
    if(status?.cpu_id)values.push(['cpu_id',status.cpu_id]);
    if(status?.fingerprint)values.push([text('公钥摘要','Public key digest'),'SHA-256 '+status.fingerprint]);
    if(item.type==='local')values.push([text('材料','Material'),status?.cpu_id?text('独立身份已生成','Independent material generated'):text('首次调用时生成独立密钥','Independent material is generated on first use')]);
    if(item.type==='relay')values.push([text('连接状态','Connection'),text('配置已保存不代表已连通','Saved configuration is not proof of connectivity')]);
    const kv='<div class="om-soter-kv">'+values.map(([k,v])=>'<div class="k">'+esc(k)+'</div><div class="v">'+esc(v)+'</div>').join('')+'</div>';
    let editor='';
    if(item.type==='key')editor=field('om-soter-key-path',text('设备上的私钥文件路径（PEM 或 JSON）','Key file on this device (PEM or JSON)'),'','text','placeholder="/sdcard/Download/identity.pem"')+field('om-soter-cpu-id',text('设备 ID（PEM 必填，JSON 可自带）','Device ID (required for PEM, optional for JSON)'),item.cpu_id||'','text','maxlength="32" autocomplete="off"')+'<button class="om-button secondary" type="button" id="om-soter-key-import">'+text('导入私钥','Import key')+'</button>';
    if(item.type==='relay')editor=field('om-soter-relay-url',text('服务器 URL','Server URL'),item.url||'')+field('om-soter-relay-device',text('设备 ID','Device ID'),item.device_id||'')+field('om-soter-relay-token','Token',item.token||'','password','autocomplete="off"')+'<button class="om-button secondary" type="button" id="om-soter-relay-save">'+text('保存','Save')+'</button>';
    const actions=field('om-soter-name',text('名称','Name'),item.name,'text','maxlength="80"')+'<div class="om-soter-actions"><button class="om-button" type="button" id="om-soter-rename">'+text('保存名称','Save name')+'</button><button class="om-button secondary" type="button" id="om-soter-default" '+(status?.ready?'':'disabled')+'>'+text('设为默认','Set as default')+'</button><button class="om-button secondary" type="button" id="om-soter-delete">'+text('删除','Delete')+'</button></div>';
    host.innerHTML=header(item.name,label(item.type),'om-soter-detail-back')+'<div class="stack">'+panel(text('身份信息','Identity'),'<div style="padding:2px 20px 18px">'+kv+editor+'</div>')+panel(text('操作','Actions'),'<div style="padding:2px 20px 18px">'+actions+'<p id="om-soter-msg" data-soter-message class="om-soter-msg" role="status"></p></div>')+'</div>';
    document.getElementById('om-soter-detail-back').onclick=()=>history.back();
    const save=async(change)=>{const base=snapshot;const store=JSON.parse(JSON.stringify(base.store));change(store,store.identities.find(i=>i.id===id));snapshot=await policyCall('soter-write',request(base,store));await renderDetail(id);message(text('已保存；该身份的材料变更会使旧签名会话失效。','Saved; material changes invalidate old signing sessions.'));};
    document.getElementById('om-soter-rename').onclick=()=>action(()=>save((_,record)=>{record.name=document.getElementById('om-soter-name').value.trim();if(!record.name)throw new Error(text('名称不能为空','Name cannot be empty'));}));
    document.getElementById('om-soter-default').onclick=()=>action(()=>save(store=>{store.default=id;}));
    document.getElementById('om-soter-delete').onclick=()=>action(async()=>{const count=Object.values(snapshot.store.apps).filter(v=>v===id).length;if(!window.confirm(text('删除身份并移除 '+count+' 个应用分配？受影响应用需重新注册。','Delete this identity and '+count+' app assignments? Affected apps must register again.')))return;
      const base=snapshot;const store=JSON.parse(JSON.stringify(base.store));store.identities=store.identities.filter(i=>i.id!==id);if(store.default===id)store.default='';for(const [pkg,value]of Object.entries(store.apps))if(value===id)delete store.apps[pkg];snapshot=await policyCall('soter-write',request(base,store));history.back();});
    const imp=document.getElementById('om-soter-key-import');if(imp)imp.onclick=()=>action(async()=>{const path=document.getElementById('om-soter-key-path').value.trim();const cpu_id=document.getElementById('om-soter-cpu-id').value.trim();if(!path.startsWith('/')||path.includes('\0'))throw new Error(text('请输入设备上的绝对文件路径','Enter an absolute device path'));
      if(item.pem&&!window.confirm(text('更换私钥会作废该身份下的旧注册与签名会话。继续？','Replacing this key invalidates old registrations and sign sessions. Continue?')))return;
      snapshot=await policyCall('soter-import',{...request(snapshot,null),id,path,cpu_id});await renderDetail(id);message(text('私钥已在设备端校验并保存，未经过页面。','Key validated and saved on-device; private material never entered this page.'));});
    const relay=document.getElementById('om-soter-relay-save');if(relay)relay.onclick=()=>action(()=>save((_,record)=>{record.url=document.getElementById('om-soter-relay-url').value.trim();record.device_id=document.getElementById('om-soter-relay-device').value.trim();record.token=document.getElementById('om-soter-relay-token').value.trim();}));
  }catch(error){if(turn===generation)errorPage(host,error,'om-soter-detail-back','om-soter',()=>renderDetail(id));}
}
export function mountSoter(bridge,mutations=createBridge('soter_write',0)){exec=bridge;writeExec=mutations;injectStyle();document.addEventListener('ommega-soter-policy',event=>{event.detail.promise=policyCall(event.detail.operation,event.detail.payload??null,event.detail.package??null);});}
export function soterShow(page,id){return page==='detail'?renderDetail(id):renderList();}
