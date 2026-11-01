// Actual packaged A WebUI, local asset routes and an in-memory bridge only.
import {createRequire} from 'node:module';
import {readFile,mkdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=new URL('../../',import.meta.url),output=new URL('../test-results/',import.meta.url);
const bridge=await readFile(new URL('../preview-bridge.js',import.meta.url),'utf8');
const mock=bridge+String.raw`
(()=>{
 const original=window.ksu.exec.bind(window.ksu);
 const test=window.__SOTER_TEST={store:{version:1,default:'',identities:[],apps:{}},expected:null,failRead:false,failWrite:false,failImport:false,calls:[],shared:{}};
 const response=()=>({store:test.store,expected:test.expected,legacy_revision:null,status:Object.fromEntries(test.store.identities.map(i=>[i.id,{ready:i.type==='local'||i.type==='key'&&!!i.pem||i.type==='relay'&&!!i.url&&!!i.token&&!!i.device_id,cpu_id:i.cpu_id||null,fingerprint:i.pem?'a'.repeat(64):null}])),warnings:[],shared:test.shared});
 window.ksu.exec=(command,options,callback)=>{
  const read=command.match(/soter-read(?:\s+(\S+))?$/);
  if(read||command.endsWith('soter-write')||command.endsWith('soter-import')){
   test.calls.push(command);let errno=0,stdout='',stderr='';
   if(read){test.lastReadPackage=(read[1]||"").replace(/"/g,"")||null;if(test.failRead){errno=1;stderr='mock read failed';}else stdout=JSON.stringify(response());}
   else{
    const encoded=command.match(/printf %s '([A-Za-z0-9+/=]+)'/);const payload=JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(encoded[1]),c=>c.charCodeAt(0))));
    if(payload.expected!==test.expected){errno=1;stderr='Config changed; reload before saving';}
    else if(test.failWrite||test.failImport&&command.endsWith('soter-import')){errno=1;stderr='mock persistence failed';}
    else{if(command.endsWith('soter-write'))test.store=payload.store;else{const record=test.store.identities.find(i=>i.id===payload.id);record.cpu_id=payload.cpu_id;record.pem='/managed/synthetic-only.pem';}test.expected=JSON.stringify(test.store);stdout=JSON.stringify(response());}
   }
   setTimeout(()=>window[callback]?.(errno,stdout,stderr),5);return;
  }
  if(command==='cat /data/adb/ommega/soterta/status.json 2>/dev/null'){setTimeout(()=>window[callback]?.(0,JSON.stringify({enabled:1,running:1,pid:123,owner:'us'}),''),0);return;}
  return original(command,options,callback);
 };
})();
`;
await mkdir(output,{recursive:true});
const browser=await chromium.launch({channel:process.env.BROWSER_CHANNEL||'msedge',headless:true});
try{
 for(const [locale,theme,width,height]of [['zh-CN','light',320,640],['zh-CN','dark',393,852],['en-US','light',1024,768]]){
  const context=await browser.newContext({viewport:{width,height},locale,colorScheme:theme,reducedMotion:'reduce'});
  await context.route('**/*',async route=>{
   const url=new URL(route.request().url());if(url.origin!=='http://127.0.0.1:4179')return route.abort();
   if(url.pathname==='/soter-preview-bridge.js')return route.fulfill({contentType:'text/javascript',body:mock});
   const match=/^\/a\/(.*)$/.exec(url.pathname);if(!match||match[1].includes('..'))return route.fulfill({status:404});
   const name=match[1]||'index.html';try{let body=await readFile(new URL('a-side/source/template/webroot/'+name,root));if(name.endsWith('.html'))body=Buffer.from(body.toString().replace('<head>','<head><script src="/soter-preview-bridge.js"></script>'));
    await route.fulfill({contentType:{html:'text/html',js:'text/javascript',css:'text/css',json:'application/json',png:'image/png',woff2:'font/woff2'}[name.split('.').at(-1)]||'application/octet-stream',body});
   }catch{return route.fulfill({status:404});}
  });
  const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.message));page.on('dialog',dialog=>dialog.accept());
  await page.goto('http://127.0.0.1:4179/a/#om-settings');await page.locator('[data-om-action="ommega-soterta"]').click();
  await page.locator('#om-soter-add').click();await page.locator('[data-add="local"]').click();await page.locator('#om-soter-name').waitFor();
  await page.locator('#om-soter-name').fill('Review identity');await page.locator('#om-soter-rename').click();await page.locator('#om-soter-msg').filter({hasText:/Saved|已保存/}).waitFor();
  const local=await page.evaluate(()=>window.__SOTER_TEST.store.identities[0].id);
  await page.locator('#om-soter-detail-back').click();await page.locator('#om-soter-add').click();await page.locator('[data-add="key"]').click();await page.locator('#om-soter-key-path').fill('/sdcard/Download/synthetic.pem');await page.locator('#om-soter-cpu-id').fill('a'.repeat(32));
  await page.evaluate(()=>window.__SOTER_TEST.failImport=true);await page.locator('#om-soter-key-import').click();await page.locator('#om-soter-msg').filter({hasText:'mock persistence failed'}).waitFor();
  assert.equal(await page.evaluate(()=>!!window.__SOTER_TEST.store.identities.find(i=>i.type==='key').pem),false);
  await page.evaluate(()=>window.__SOTER_TEST.failImport=false);await page.locator('#om-soter-key-import').click();await page.locator('#om-soter-msg').filter({hasText:/Key validated|私钥已在设备端/}).waitFor();
  await page.locator('#om-soter-default').click();await page.locator('#om-soter-msg').filter({hasText:/Saved|已保存/}).waitFor();
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
  await page.screenshot({path:fileURLToPath(new URL('soter-'+locale+'-'+theme+'-'+width+'.png',output))});
  await page.evaluate(()=>window.__SOTER_TEST.shared={'com.example.wallet':['com.example.wallet','com.example.checker'],'com.example.checker':['com.example.wallet','com.example.checker']});
  await page.locator('[data-om-view="apps"]').click();await page.locator('.card[data-package="com.example.wallet"]').click();
  await page.waitForFunction(()=>document.querySelectorAll('input[name="om-app-soter"]').length===3);
  await page.locator('input[name="om-app-soter"][value="'+local+'"]').check({force:true});await page.locator('#om-detail-soter-status').filter({hasText:/Saved|已保存/}).waitFor();
  assert.equal(await page.evaluate(()=>window.__SOTER_TEST.store.apps['com.example.wallet']),local);
  assert.equal(await page.evaluate(()=>window.__SOTER_TEST.lastReadPackage),'com.example.wallet');
  assert.deepEqual(await page.evaluate(()=>Object.keys(window.__SOTER_TEST.shared)),['com.example.wallet','com.example.checker']);
  assert.equal(await page.evaluate(()=>window.__SOTER_TEST.store.apps['com.example.checker']),local);
  await page.evaluate(()=>window.__SOTER_TEST.expected='changed elsewhere');const key=await page.evaluate(()=>window.__SOTER_TEST.store.identities.find(i=>i.type==='key').id);
  await page.locator('input[name="om-app-soter"][value="'+key+'"]').check({force:true});await page.locator('#om-detail-soter-status').filter({hasText:/Config changed/}).waitFor();assert.ok(await page.locator('input[name="om-app-soter"][value="'+local+'"]').isChecked());
  await page.evaluate(()=>{window.__SOTER_TEST.store={version:1,default:'',identities:[],apps:{}};window.__SOTER_TEST.expected=null;});await page.locator('#om-detail-back').click();await page.locator('.card[data-package="com.example.wallet"]').click();const def=page.locator('input[name="om-app-soter"][value=""]');await page.waitForFunction(()=>document.querySelectorAll('input[name="om-app-soter"]').length===1);assert.ok(await def.isChecked());assert.ok(await def.isDisabled());
  assert.deepEqual(errors,[]);console.log('PASS SOTER '+locale+' '+theme+' '+width+': list/detail, rename, file-only import failure/success, defaults, app CAS, disabled empty default');await context.close();
 }
}finally{await browser.close();}
