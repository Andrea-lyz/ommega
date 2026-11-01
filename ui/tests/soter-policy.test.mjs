import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {webcrypto} from 'node:crypto';
const source=readFileSync(new URL('../../a-side/source/template/webroot/soter.js',import.meta.url),'utf8').replace(/^import .*\n/gm,'').replace(/^export /gm,'');
function harness(){
 const nodes=new Map(),commands=[],writes=[];
 const initial={version:1,default:'',identities:[{id:'a',name:'Identity A',type:'local'}],apps:{'org.example.one':'a'}};
 const state={store:structuredClone(initial),expected:'v1',status:{a:{ready:true,cpu_id:'a'.repeat(32)}},legacy_revision:null,enabled:true};
 let readError=false,writeError=false;
 const node=id=>{if(!nodes.has(id))nodes.set(id,{id,innerHTML:'',textContent:'',value:'',hidden:id!=='om-soter-identity',querySelectorAll:()=>[],dataset:{}});return nodes.get(id);};
 const bridge=async command=>{commands.push(command);
   if(command.includes('soter-read'))return readError?{errno:1,stderr:'mock read failed'}:{errno:0,stdout:JSON.stringify(state)};
   if(command.includes('soter-write')||command.includes('soter-import')){
     const encoded=command.match(/printf %s '([A-Za-z0-9+/=]+)'/);assert.ok(encoded);const payload=JSON.parse(Buffer.from(encoded[1],'base64').toString());writes.push({command,payload});
     if(writeError)return {errno:1,stderr:'mock write failed'};
     if(payload.expected!==state.expected)return {errno:1,stderr:'Config changed; reload before saving'};
     if(payload.store)state.store=payload.store;state.expected='v'+(Number(state.expected.slice(1))+1);return {errno:0,stdout:JSON.stringify(state)};
   }
   if(command.includes('status.json'))return {errno:0,stdout:JSON.stringify({enabled:state.enabled?1:0,running:state.enabled?1:0,pid:123,owner:state.enabled?'us':'hal'})};
   if(command==='rm -f /data/adb/ommega/soterta/enabled'){state.enabled=false;return {errno:0};}
   if(command.includes('enabled.tmp')){state.enabled=true;return {errno:0};}
   throw Error('Unexpected command '+command);
 };
 const context=vm.createContext({TextEncoder,TextDecoder,Uint8Array,atob,btoa,structuredClone,crypto:webcrypto,document:{getElementById:node,addEventListener(){}},window:{confirm:()=>true},location:{hash:''},history:{back(){}},text:(_,en)=>en,icon:()=>'',createBridge:()=>bridge,bridge,setTimeout:fn=>{fn();return 0;}});
 vm.runInContext(source,context);vm.runInContext('mountSoter(bridge,bridge)',context);
 return {context,state,node,commands,writes,set readError(v){readError=v},set writeError(v){writeError=v},run:code=>vm.runInContext(code,context)};
}
test('failed identity read displays retry without creating an empty writable store',async()=>{const h=harness();h.readError=true;await h.run('soterShow("list")');assert.match(h.node('om-soter-page').innerHTML,/mock read failed/);assert.equal(h.writes.length,0);assert.equal(h.state.store.identities.length,1);});
test('stale rename uses original CAS token and cannot overwrite a newer app assignment',async()=>{const h=harness();await h.run('soterShow("detail","a")');h.state.expected='v2';h.state.store.apps['org.example.two']='a';h.node('om-soter-name').value='Renamed';await h.node('om-soter-rename').onclick();assert.equal(h.writes[0].payload.expected,'v1');assert.equal(h.state.store.identities[0].name,'Identity A');assert.equal(h.state.store.apps['org.example.two'],'a');assert.match(h.node('om-soter-msg').textContent,/Config changed/);});
test('delete removes all app references and default assignment',async()=>{const h=harness();h.state.store.default='a';await h.run('soterShow("detail","a")');await h.node('om-soter-delete').onclick();assert.equal(h.state.store.default,'');assert.deepEqual(h.state.store.apps,{});assert.deepEqual(h.state.store.identities,[]);});
test('private-key import sends only device path and public CPU id; failed writes never report success',async()=>{const h=harness();h.state.store.identities[0].type='key';await h.run('soterShow("detail","a")');h.node('om-soter-key-path').value='/sdcard/Download/private.pem';h.node('om-soter-cpu-id').value='a'.repeat(32);h.writeError=true;await h.node('om-soter-key-import').onclick();const p=h.writes[0].payload;assert.equal(p.path,'/sdcard/Download/private.pem');assert.equal(p.cpu_id,'a'.repeat(32));assert.ok(!h.commands.some(c=>c.includes('cat "/sdcard')));assert.ok(!JSON.stringify(h.writes).includes('BEGIN PRIVATE KEY'));assert.match(h.node('om-soter-msg').textContent,/mock write failed/);assert.equal(h.state.store.identities[0].pem,undefined);});
test('mutation bridge does not gain a read timeout or automatic retry',async()=>{const shared=readFileSync(new URL('../shared/ui-kit.js',import.meta.url),'utf8');const part=shared.slice(shared.indexOf('export function createBridge('),shared.indexOf('export function pollWhenVisible(')).replace('export ','');let timers=0,calls=0,callback;const window={ksu:{exec(_c,_o,name){calls++;callback=name;}}};const c=vm.createContext({window,text:(_,en)=>en,setTimeout:()=>{timers++;return 1},clearTimeout(){}});vm.runInContext(part,c);const pending=vm.runInContext('createBridge("test",0)("synthetic mutation")',c);assert.equal(timers,0);assert.equal(calls,1);window[callback](0,'ok','');assert.equal((await pending).stdout,'ok');});
test('the detail page offers no way to restart the shared TA',async()=>{
  const h=harness();
  await h.run('soterShow("detail","a")');
  assert.ok(!h.node('om-soter-identity').innerHTML.includes('om-soter-apply'));
  assert.ok(!h.commands.some(command=>command.includes('soterta.sh')));
});

test('the master switch only writes the watchdog flag',async()=>{
  const h=harness();
  await h.run('soterShow("list")');
  const master=h.node('om-soter-enabled');
  master.checked=false;
  await master.onchange();
  assert.ok(h.commands.includes('rm -f /data/adb/ommega/soterta/enabled'));
  assert.equal(h.state.enabled,false);
  assert.ok(!h.commands.some(command=>command.includes('soterta.sh')));
});

test('enabling writes the flag atomically and keeps policy untouched',async()=>{
  const h=harness();
  h.state.enabled=false;
  const before=JSON.stringify(h.state.store);
  await h.run('soterShow("list")');
  const master=h.node('om-soter-enabled');
  assert.ok(!master.checked,'the switch shows the stored off state');
  master.checked=true;
  await master.onchange();
  const enable=h.commands.find(command=>command.includes('enabled.tmp'));
  assert.ok(enable,'the enable sequence must be written');
  assert.match(enable,/chmod 0600/);
  assert.match(enable,/mv /);
  assert.equal(h.state.enabled,true);
  assert.equal(JSON.stringify(h.state.store),before);
  assert.equal(h.writes.length,0);
});
