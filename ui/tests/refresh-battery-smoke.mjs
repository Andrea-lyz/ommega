// Real packaged pages, in-memory root bridge, no server, device or HAL needed.
import {createRequire} from 'node:module';
import {readFile, mkdir, writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
const require = createRequire(import.meta.url);
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const root = new URL('../../',import.meta.url), output = new URL('../test-results/',import.meta.url);
const fixture = device => {
  const raw = `OMMEGA_RELAY_SERVER=https://relay.example.com:8443\nOMMEGA_RELAY_DEVICE_ID=${device}\nOMMEGA_RELAY_TOKEN=preview-placeholder\n`;
  return {raw:Buffer.from(raw).toString('base64'),digest:createHash('sha256').update(raw).digest('hex')};
};
const mock = `window.__OM_FIXTURE__=${JSON.stringify(fixture('dedicated-b'))};\n` + await readFile(new URL('../preview-bridge.js',import.meta.url),'utf8');
await mkdir(output,{recursive:true});
const browser = await chromium.launch({channel:process.env.BROWSER_CHANNEL || 'msedge',headless:true});
const results=[];
try {
  for (const [side,locale,theme] of [['a','zh-CN','dark'],['a','en-US','light'],['b','zh-CN','dark'],['b','en-US','light']]) {
    const context = await browser.newContext({viewport:{width:393,height:844},locale,colorScheme:theme,reducedMotion:'reduce'});
    await context.route('http://127.0.0.1:4179/**',async route=>{
      const path = new URL(route.request().url()).pathname;
      if(path==='/preview-bridge.js') return route.fulfill({contentType:'text/javascript',body:mock});
      const match=/^\/(a|b)\/(.*)$/.exec(path);
      if(!match || match[2].includes('..')) return route.fulfill({status:404});
      const name=match[2] || 'index.html';
      try {
        let body=await readFile(new URL(`${match[1]}-side/source/template/webroot/${name}`,root));
        if(name.endsWith('.html')) body=Buffer.from(body.toString().replace('<head>','<head><script src="/preview-bridge.js"></script>'));
        const type={html:'text/html',js:'text/javascript',css:'text/css',json:'application/json',xml:'application/xml',png:'image/png',woff2:'font/woff2'}[name.split('.').at(-1)] || 'application/octet-stream';
        await route.fulfill({contentType:type,body});
      }catch{await route.fulfill({status:404});}
    });
    await context.route('https://**/*',route=>route.abort());
    if (side==='a') await context.addInitScript(()=>{
      let bridge;
      window.__bootstrapHeld=[];window.__bootstrapReleased=false;
      Object.defineProperty(window,'ksu',{configurable:true,get:()=>bridge,set:value=>{
        bridge=value;const original=value.exec;window.__bootstrapExec=original;
        value.exec=(command,options,callback)=>command==='[ -d /data/adb/modules/.ommega ]' && !window.__bootstrapReleased
          ? window.__bootstrapHeld.push({command,options,callback}) : original(command,options,callback);
      }});
    });
    const page=await context.newPage(), errors=[];
    page.on('pageerror',error=>errors.push(error.message));
    await page.goto(`http://127.0.0.1:4179/${side}/`,{waitUntil:'domcontentloaded'});
    const selector=side==='a'?'#om-refresh':'#runtime-refresh';
    const button=page.locator(selector);
    await page.waitForFunction(selector=>document.querySelector(selector)?.getAttribute('aria-busy')==='false',selector);
    if(side==='a') {
      // A delayed original bootstrap must join the safe reader, never clear a
      // list/draft/search already loaded by an explicit refresh.
      await page.waitForFunction(()=>window.__bootstrapHeld.length===1);
      await page.locator('[data-om-view=apps]').click();
      await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
      await page.locator('.card[data-package="com.example.wallet"]').waitFor();
      await page.locator('#search').fill('wallet');
      const initialReads=await page.evaluate(()=>window.__previewCommands.filter(command=>command==='pm list packages -3').length);
      await page.evaluate(()=>{window.__bootstrapReleased=true;const {command,options,callback}=window.__bootstrapHeld.shift();window.__bootstrapExec(command,options,callback);});
      await page.waitForFunction(count=>window.__previewCommands.filter(command=>command==='pm list packages -3').length>count && document.getElementById('refresh').getAttribute('aria-busy')==='false',initialReads);
      assert.equal(await page.locator('[id="checkbox-com.example.wallet"]').evaluate(el=>el.checked),true);
      assert.equal(await page.locator('#search').evaluate(el=>el.value),'wallet');
      await page.locator('#search').fill('');
      await page.locator('[data-om-view=overview]').click();
      assert.match(await page.locator('#om-b-battery').textContent(),/^76%/);
      for(const [percent,charging] of [[0,false],[100,true]]) {
        await page.evaluate(({percent,charging})=>{window.__OM_B_STATUS__={status:'ok',online_b_devices:[{device_id:'dedicated-b',battery_percent:percent,battery_charging:charging,battery_updated_at_ms:Date.now()}]};},{percent,charging});
        await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
        assert.match(await page.locator('#om-b-battery').textContent(),new RegExp(`^${percent}%`));
        assert.match(await page.locator('#om-b-battery').textContent(),charging?/充电中|Charging/:/未充电|Not charging/);
      }
      for(const status of [
        {status:'ok',online_b_devices:[{device_id:'other',battery_percent:55,battery_charging:true,battery_updated_at_ms:Date.now()}]},
        {status:'ok',online_b_devices:[{device_id:'dedicated-b',battery_percent:55,battery_charging:true,battery_updated_at_ms:Date.now()-120000}]},
        {status:'ok',online_b_devices:[{device_id:'dedicated-b',battery_percent:null,battery_charging:null,battery_updated_at_ms:null}]},
      ]) {
        await page.evaluate(status=>{window.__OM_B_STATUS__=status;},status);
        await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
        assert.doesNotMatch(await page.locator('#om-b-battery').textContent(),/\d+%/);
      }
      await page.evaluate(()=>{delete window.__OM_B_STATUS__;});
      // A top refresh on Apps must re-enumerate packages and preserve draft and search.
      await page.locator('[data-om-view=apps]').click();
      await page.locator('.card[data-package="com.example.wallet"]').waitFor();
      await page.waitForFunction(()=>document.getElementById('ommega-global-scope-enabled')?.disabled===false);
      // The list comes from installed packages, not target.txt. Wallet and
      // checker are saved; chat and the other user apps were never in scope and
      // must still be visible. There is no save action or show-all filter.
      await page.locator('.card[data-package="com.example.chat"]').waitFor();
      assert.equal(await page.locator('.om-apps-save').count(),0);
      assert.equal(await page.locator('#save').isVisible(),false);
      assert.equal(await page.locator('#om-show-all-apps').count(),0);
      await page.locator('#menu-button').click();
      await page.waitForFunction(()=>document.getElementById('menu-options').open);
      assert.equal(await page.locator('#select-all').isVisible(),false);
      assert.equal(await page.locator('#deselect-all').isVisible(),false);
      assert.equal(await page.locator('#deselect-unnecessary').isVisible(),false);
      assert.equal(await page.locator('#add-system-app').isVisible(),true);
      await page.keyboard.press('Escape');
      await page.waitForFunction(()=>!document.getElementById('menu-options').open);
      const searchField=await page.evaluate(()=>{
        const field=document.getElementById('search');
        return {tag:field.tagName,type:field.type,readOnly:field.readOnly,disabled:field.disabled,inputMode:field.inputMode,insideShadow:Boolean(field.shadowRoot)};
      });
      assert.equal(searchField.tag,'INPUT');
      assert.equal(searchField.type,'text');
      assert.equal(searchField.readOnly,false);
      assert.equal(searchField.disabled,false);
      assert.notEqual(searchField.inputMode,'none');
      assert.equal(searchField.insideShadow,false);
      await page.locator('#search').fill('中文测试');
      assert.equal(await page.locator('#search').evaluate(el=>el.value),'中文测试');
      await page.locator('#search').fill('wallet');
      await page.evaluate(()=>{document.getElementById('checkbox-com.example.wallet').checked=false;});
      const before=await page.evaluate(()=>window.__previewCommands.length);
      await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
      assert.equal(await page.locator('#search').evaluate(el=>el.value),'wallet');
      assert.equal(await page.locator('[id="checkbox-com.example.wallet"]').evaluate(el=>el.checked),true);
      assert.equal(await page.locator('[id="checkbox-com.example.checker"]').evaluate(el=>el.checked),true);
      const reads=await page.evaluate(before=>window.__previewCommands.slice(before),before);
      assert.ok(reads.includes('pm list packages -3'));
      assert.ok(reads.every(command=>! /"\$ommega_app_writer" write-|spl-control\.sh" save|ctl\.restart|svc power/.test(command)));
      // The Apps menu's own refresh follows the same real reader.
      await page.locator('#menu-button').click();
      await page.locator('#refresh').click();
      await page.waitForFunction(()=>document.getElementById('refresh').getAttribute('aria-busy')==='false');
      assert.equal(await page.locator('[id="checkbox-com.example.wallet"]').evaluate(el=>el.checked),true);
      // Distinct menu/top actions must queue a new app read, not merely await
      // the already-running one and claim that the later request was refreshed.
      await page.evaluate(()=>{
        window.__appOriginal=window.ksu.exec;window.__appHeld=[];
        window.ksu.exec=(command,options,callback)=>command==='pm list packages -3'?window.__appHeld.push({command,options,callback}):window.__appOriginal(command,options,callback);
      });
      await page.locator('#refresh').click();
      await page.waitForFunction(()=>window.__appHeld.length===1);
      await page.evaluate(()=>{void document.getElementById('om-refresh').onclick();});
      assert.equal(await button.getAttribute('aria-busy'),'true');
      assert.equal(await page.evaluate(()=>window.__appHeld.length),1);
      await page.evaluate(()=>{const {command,options,callback}=window.__appHeld.shift();window.__appOriginal(command,options,callback);});
      await page.waitForFunction(()=>window.__appHeld.length===1);
      assert.equal(await button.getAttribute('aria-busy'),'true');
      await page.evaluate(()=>{const {command,options,callback}=window.__appHeld.shift();window.__appOriginal(command,options,callback);});
      await page.waitForFunction(()=>document.getElementById('om-refresh').getAttribute('aria-busy')==='false');
      await page.evaluate(()=>{window.ksu.exec=window.__appOriginal;});
      assert.equal(await page.locator('[id="checkbox-com.example.wallet"]').evaluate(el=>el.checked),true);
      await page.keyboard.press('Escape');
      await page.locator('[data-om-view=settings]').click();
      await page.waitForFunction(()=>document.getElementById('ommega-global-compat')?.disabled===false);
      await page.evaluate(()=>{targetCompat='version = 1\n[positive_key_id]\nenabled = true\n';document.getElementById('ommega-global-compat').disabled=true;});
      await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
      assert.equal(await page.locator('#ommega-global-compat').isChecked(),true);
      assert.equal(await page.locator('#ommega-global-compat').isEnabled(),true);
      assert.match(await page.locator('#om-updated').textContent(),/状态读取于|Status read at/);
    } else {
      assert.match(await page.locator('#server-state').textContent(),/服务器已连接|Server connected/);
      await page.evaluate(()=>{window.__OM_B_STATUS__={status:'ok',online_b_devices:[]};});
      await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
      assert.match(await page.locator('#server-state').textContent(),/B 端未连接|B device (?:disconnected|offline)/);
      await page.locator('[data-view=connection]').click();
      await page.locator('#relay-device').fill('unsaved-draft');
      await page.evaluate(fixture=>{window.__OM_FIXTURE__=fixture;window.__OM_B_STATUS__={status:'ok',online_b_devices:[{device_id:'new-b'}]};},fixture('new-b'));
      const before=await page.evaluate(()=>window.__previewCommands.length);
      await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
      assert.equal(await page.locator('#summary-device').textContent(),'new-b');
      assert.equal(await page.locator('#relay-device').inputValue(),'unsaved-draft');
      assert.match(await page.locator('#server-state').textContent(),/服务器已连接|Server connected/);
      const reads=await page.evaluate(before=>window.__previewCommands.slice(before),before);
      assert.ok(reads.some(command=>command.includes('payload=$(base64')));
      assert.ok(reads.some(command=>command.includes('PIDS=')));
      assert.ok(reads.some(command=>command.includes('spl-control.sh" status --readonly')));
      assert.ok(reads.some(command=>command.includes('ommega_status_url=')));
      assert.ok(reads.every(command=>! /spl-control\.sh" save|SAVED|ctl\.restart|svc power/.test(command)));
      await page.locator('[data-view=settings]').click();
      await page.locator('#spl-open').click();
      await page.waitForFunction(()=>!document.getElementById('save').disabled);
      await page.locator('#system').fill('2026-12-01');
      await page.evaluate(()=>document.getElementById('spl-dialog').close());
      await page.waitForFunction(()=>!document.getElementById('spl-dialog').open);
      await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
      assert.equal(await page.locator('#system').inputValue(),'2026-12-01');
    }
    // Busy lasts until the server response, not merely the local command.
    await page.evaluate(()=>{
      window.__testExec=window.ksu.exec;window.__held=[];
      window.ksu.exec=(command,options,callback)=>command.includes('ommega_status_url=')?window.__held.push({command,options,callback}):window.__testExec(command,options,callback);
    });
    await button.click();await page.waitForFunction(()=>window.__held.length===1);
    assert.equal(await button.getAttribute('aria-busy'),'true');
    await page.evaluate(()=>{const {command,options,callback}=window.__held.shift();window.__testExec(command,options,callback);});
    await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
    // Missing manager callback must release the read action without killing a process.
    await page.evaluate(()=>{
      const nativeTimeout=window.setTimeout;window.__testTimeout=nativeTimeout;
      window.setTimeout=(fn,ms,...args)=>nativeTimeout(fn,ms===12000?80:ms,...args);
      window.ksu.exec=(command,options,callback)=>{if(!command.includes('ommega_status_url='))window.__testExec(command,options,callback);};
    });
    await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
    assert.equal(await button.isEnabled(),true);
    assert.match(await page.locator(side==='a'?'#om-updated':'#refresh-message').textContent(),/刷新不完整|Refresh incomplete|失败|failed|超时|timed out|未完成|incomplete/i);
    await page.evaluate(()=>{window.ksu.exec=window.__testExec;window.setTimeout=window.__testTimeout;});
    await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
    assert.deepEqual(errors,[],`${side} ${locale} JavaScript errors`);
    for(const width of [320,393,844,1024]) {
      await page.setViewportSize({width,height:844});
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`${side} overflow at ${width}`);
    }
    await page.setViewportSize({width:393,height:844});
    await page.screenshot({path:fileURLToPath(new URL(`${side}-refresh-battery-${locale}.png`,output)),fullPage:true});
    results.push({side,locale,theme,errors,commands:await page.evaluate(()=>window.__previewCommands.length)});
    await context.close();
    console.log(`PASS ${side} ${locale}: actual refresh wiring, retained search, status/error completion${side==='a'?', battery freshness, installed-app list and isolation':''}`);
  }
  await writeFile(new URL('refresh-battery-results.json',output),JSON.stringify(results,null,2));
}finally{await browser.close();}
