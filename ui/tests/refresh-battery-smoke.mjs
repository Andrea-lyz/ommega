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

async function checkASearchIme(page,context,button,selector) {
  const search=page.locator('#om-app-search'), wallet=page.locator('.card[data-package="com.example.wallet"]'), chat=page.locator('.card[data-package="com.example.chat"]');
  const idle=()=>page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
  await page.waitForFunction(()=>window.__bootstrapHeld.length===1);
  for(const view of ['apps','settings','apps']) {
    await page.locator(`.om-nav [data-om-view=${view}]`).tap();
    await page.waitForFunction(view=>document.body.dataset.omView===view,view);
    assert.equal(await search.evaluate(el=>el===document.activeElement),false,`${view==='apps'?'cold/return Apps':'Settings'} navigation must not focus search or request the IME`);
    assert.equal(await page.locator('#om-app-search-region').isVisible(),view==='apps');
  }
  const structure=await search.evaluate(input=>{
    const legacy=document.getElementById('search'), header=legacy.closest('.header'), style=getComputedStyle(input);
    return {tag:input.tagName,type:input.type,bodyRegion:input.closest('#om-app-search-region')?.parentElement===document.body,
      inHeader:Boolean(input.closest('.header')),autoFocus:input.autofocus,readOnly:input.readOnly,disabled:input.disabled,
      oldClasses:input.matches('.search-input,.search-field,.search-bg'),shadow:Boolean(input.shadowRoot),
      label:input.labels?.length,userSelect:style.userSelect,webkitUserSelect:style.webkitUserSelect,
      opacity:style.opacity,pointerEvents:style.pointerEvents,transition:style.transitionProperty,
      legacyVisible:Boolean(legacy.getClientRects().length),legacyDisabled:legacy.disabled,headerInert:header.inert,headerHidden:header.hidden};
  });
  assert.deepEqual({...structure,transition:undefined},{tag:'INPUT',type:'text',bodyRegion:true,inHeader:false,autoFocus:false,readOnly:false,disabled:false,
    oldClasses:false,shadow:false,label:1,userSelect:'text',webkitUserSelect:'text',opacity:'1',pointerEvents:'auto',transition:undefined,
    legacyVisible:false,legacyDisabled:true,headerInert:true,headerHidden:true});
  assert.ok(!structure.transition.split(',').map(value=>value.trim()).includes('all'),'new editor must not inherit vendor transition: all');
  await search.tap();
  assert.equal(await search.evaluate(el=>el===document.activeElement),true,'only explicit touch interaction focuses the native input');
  // Load once without releasing the original delayed bootstrap. Its eventual
  // completion must refresh the same native input, not reset an IME session.
  await button.click();await idle();await wallet.waitFor();
  await page.evaluate(()=>{
    const node=document.getElementById('om-app-search'), descriptor=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value');
    const probe=window.__searchProbe={node,writes:[],events:[],calls:[],renders:[],composing:false};
    document.addEventListener('ommega-apps-rendered',()=>probe.renders.push({cards:document.querySelectorAll('#apps-list .card').length,value:node.value}));
    window.__searchScrollTo=window.scrollTo;
    window.scrollTo=(...args)=>{probe.calls.push('scrollTo');return window.__searchScrollTo.apply(window,args);};
    Object.defineProperty(node,'value',{configurable:true,get(){return descriptor.get.call(this);},set(value){probe.writes.push(String(value));descriptor.set.call(this,value);}});
    for(const name of ['focus','blur','scrollIntoView']) {
      const original=node[name];
      Object.defineProperty(node,name,{configurable:true,value(...args){probe.calls.push(name);return original.apply(this,args);}});
    }
    for(const type of ['compositionstart','compositionupdate','compositionend']) node.addEventListener(type,event=>{
      probe.events.push({type:event.type,data:event.data});
      if(type==='compositionstart') probe.composing=true;
      if(type==='compositionend') probe.composing=false;
    });
  });
  const cdp=await context.newCDPSession(page);
  const compose=text=>cdp.send('Input.imeSetComposition',{text,selectionStart:text.length,selectionEnd:text.length});
  const snapshot=()=>page.evaluate(()=>{
    const probe=window.__searchProbe;
    return {value:probe.node.value,sameNode:document.getElementById('om-app-search')===probe.node,focused:document.activeElement===probe.node,composing:probe.composing,writes:[...probe.writes],ends:probe.events.filter(event=>event.type==='compositionend').length};
  });
  const stableComposition=async(value,where)=>{
    assert.deepEqual(await snapshot(),{value,sameNode:true,focused:true,composing:true,writes:[],ends:0},where);
    assert.equal(await wallet.isVisible(),true,`${where}: legacy filtering must not hide rows for uncommitted pinyin`);
    assert.equal(await chat.isVisible(),true,`${where}: Chinese row stays visible until commit`);
  };
  try {
    await search.tap();await compose('zhongwen');
    await stableComposition('zhongwen','before delayed cold bootstrap');
    const initialReads=await page.evaluate(()=>window.__previewCommands.filter(command=>command==='pm list packages -3').length);
    await page.evaluate(()=>{window.__bootstrapReleased=true;const {command,options,callback}=window.__bootstrapHeld.shift();window.__bootstrapExec(command,options,callback);});
    await page.waitForFunction(count=>window.__previewCommands.filter(command=>command==='pm list packages -3').length>count && document.getElementById('refresh').getAttribute('aria-busy')==='false',initialReads);
    await stableComposition('zhongwen','after delayed cold bootstrap');
    await cdp.send('Input.insertText',{text:'中文'});
    assert.deepEqual(await snapshot(),{value:'中文',sameNode:true,focused:true,composing:false,writes:[],ends:1},'native candidate commit survives cold bootstrap');
    assert.equal(await chat.isVisible(),true,'committed Chinese immediately matches the Chinese app name');
    assert.equal(await wallet.isVisible(),false,'committed Chinese immediately excludes unmatched apps: '+JSON.stringify(await page.evaluate(()=>({renders:window.__searchProbe.renders,rows:[...document.querySelectorAll('#apps-list .card')].map(card=>({pkg:card.getAttribute('data-package'),name:card.dataset.omOriginalName,rowHidden:(card.closest('.card-box')||card).hidden,rowDisplay:getComputedStyle(card.closest('.card-box')||card).display}))}))));

    await search.fill('');
    await page.evaluate(()=>{
      window.__searchProbe.events=[];window.__searchProbe.writes=[];
      window.__searchExec=window.ksu.exec;window.__searchHeld=[];
      window.ksu.exec=(command,options,callback)=>command==='pm list packages -3'?window.__searchHeld.push({command,options,callback}):window.__searchExec(command,options,callback);
    });
    await compose('zhongwen');
    await page.evaluate(()=>{void document.getElementById('om-refresh').onclick();});
    await page.waitForFunction(()=>window.__searchHeld.length===1);
    await compose('zhongwenhouxuan');
    await stableComposition('zhongwenhouxuan','during held manual app refresh');
    await page.evaluate(()=>{const {command,options,callback}=window.__searchHeld.shift();window.__searchExec(command,options,callback);});
    await idle();
    await stableComposition('zhongwenhouxuan','after manual app refresh');
    await cdp.send('Input.insertText',{text:'中文'});
    assert.deepEqual(await snapshot(),{value:'中文',sameNode:true,focused:true,composing:false,writes:[],ends:1},'native candidate commit survives manual refresh');
    assert.equal(await chat.isVisible(),true);
    assert.equal(await wallet.isVisible(),false);
    await page.evaluate(()=>{window.ksu.exec=window.__searchExec;});

    // Stop animation frames only after reads are released. Drain one frame,
    // type through Chromium between frames, then release the second frame:
    // a stale refresh snapshot must never overwrite this new committed input.
    await search.fill('wallet');
    await search.evaluate(el=>el.setSelectionRange(el.value.length,el.value.length));
    await page.evaluate(()=>{
      window.__searchProbe.events=[];window.__searchProbe.writes=[];
      window.__searchRaf=window.requestAnimationFrame;window.__searchCancelRaf=window.cancelAnimationFrame;
      window.__searchFrames=[];let frameId=0;
      window.requestAnimationFrame=callback=>{const id=++frameId;window.__searchFrames.push({id,callback});return id;};
      window.cancelAnimationFrame=id=>{window.__searchFrames=window.__searchFrames.filter(frame=>frame.id!==id);};
      void document.getElementById('om-refresh').onclick();
    });
    await page.waitForFunction(()=>window.__searchFrames.length>=2 && !document.querySelector('#apps-list .card'),null,{polling:10});
    await page.evaluate(()=>{for(const {callback} of window.__searchFrames.splice(0)) callback(performance.now());});
    await cdp.send('Input.insertText',{text:'.new'});
    assert.equal(await search.inputValue(),'wallet.new','new input arrives between render/refresh frames');
    await page.evaluate(()=>{
      for(const {callback} of window.__searchFrames.splice(0)) callback(performance.now());
      window.requestAnimationFrame=window.__searchRaf;window.cancelAnimationFrame=window.__searchCancelRaf;
    });
    await idle();
    assert.deepEqual(await snapshot(),{value:'wallet.new',sameNode:true,focused:true,composing:false,writes:[],ends:0},'refresh preserves between-frame edits without invoking the input value setter');

    // Clear is an explicit edit, not a new app read or an editing-focus replay.
    await search.fill('  ＷＡＬＬＥＴ  ');
    assert.equal(await wallet.isVisible(),true,'trimmed, case-folded NFKC queries match the original app/package');
    assert.equal(await chat.isVisible(),false);
    const beforeClear=await page.evaluate(()=>{window.__searchProbe.calls=[];window.__searchProbe.writes=[];return window.__previewCommands.length;});
    await page.locator('#om-app-search-clear').tap();
    assert.equal(await search.inputValue(),'');
    assert.equal(await wallet.isVisible(),true);
    assert.equal(await chat.isVisible(),true);
    assert.equal(await page.locator('#om-app-search-clear').isVisible(),false);
    assert.deepEqual(await page.evaluate(()=>({calls:window.__searchProbe.calls,writes:window.__searchProbe.writes,commands:window.__previewCommands.length})),
      {calls:[],writes:[''],commands:beforeClear},'Clear alone writes one empty value and never invokes focus/blur/scroll or bridge commands');

    // Closing the open Material menu must not steal a single real search tap.
    await search.fill('wallet');
    await page.locator('#menu-button').tap();
    await page.waitForFunction(()=>document.getElementById('menu-options').open);
    await page.evaluate(()=>{
      window.__searchMenuClosed=false;window.__searchProbe.calls=[];window.__searchProbe.writes=[];
      document.getElementById('menu-options').addEventListener('closed',()=>{window.__searchMenuClosed=true;},{once:true});
    });
    await search.tap();
    await page.waitForFunction(()=>window.__searchMenuClosed);
    assert.equal(await search.evaluate(node=>node===document.activeElement),true,'one tap keeps input focus after the menu has fully closed');
    assert.equal(await search.inputValue(),'wallet');
    assert.deepEqual(await page.evaluate(()=>({calls:window.__searchProbe.calls,writes:window.__searchProbe.writes})),{calls:[],writes:[]});
    assert.equal(await page.locator('#menu-options').evaluate(popup=>popup.skipRestoreFocus),false,'menu focus policy is restored for its next keyboard use');

    // Detail pages hide the search region without destroying or rewriting it.
    await wallet.tap();
    await page.locator('#om-app-detail').waitFor({state:'visible'});
    assert.equal(await page.locator('#om-app-search-region').isVisible(),false);
    assert.equal(await search.inputValue(),'wallet');
    await page.locator('#om-detail-back').tap();
    await page.locator('#om-app-detail').waitFor({state:'hidden'});
    assert.equal(await page.locator('#om-app-search-region').isVisible(),true);
    assert.equal(await search.inputValue(),'wallet');
    assert.equal(await search.evaluate(node=>node===window.__searchProbe.node),true);
    assert.equal(await search.evaluate(node=>node===document.activeElement),false,'Back returns to the app card, not the editor/IME');
    assert.equal(await wallet.isVisible(),true);
    assert.equal(await chat.isVisible(),false);
    assert.deepEqual(await page.evaluate(()=>window.__searchProbe.writes),[],'detail/back never replay the query');
    return {coldDockFocus:false,nativeCandidateCommit:true,bootstrapComposition:true,manualRefreshComposition:true,betweenFrameInput:true,
      nativeTouch:true,isolatedEditor:true,clearOnlyEditorWrite:true,menuSingleTap:true,detailBackQuery:true,refreshValueWrites:0};
  } finally {
    await page.evaluate(()=>{
      if(window.__searchExec) window.ksu.exec=window.__searchExec;
      if(window.__searchRaf) {window.requestAnimationFrame=window.__searchRaf;window.cancelAnimationFrame=window.__searchCancelRaf;}
      if(window.__searchScrollTo) window.scrollTo=window.__searchScrollTo;
      if(window.__searchProbe) for(const name of ['value','focus','blur','scrollIntoView']) delete window.__searchProbe.node[name];
    });
    await cdp.detach();
  }
}

await mkdir(output,{recursive:true});
const browser = await chromium.launch({channel:process.env.BROWSER_CHANNEL || 'msedge',headless:true});
const results=[];
try {
  for (const [side,locale,theme] of [['a','zh-CN','dark'],['a','en-US','light'],['b','zh-CN','dark'],['b','en-US','light']]) {
    const context = await browser.newContext({viewport:{width:393,height:844},locale,colorScheme:theme,reducedMotion:'reduce',...(side==='a'?{isMobile:true,hasTouch:true}:{})});
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
        const packages=value.getPackagesInfo.bind(value);
        value.getPackagesInfo=raw=>JSON.stringify(JSON.parse(packages(raw)).map(app=>app.packageName==='com.example.chat'?{...app,appLabel:'中文聊天'}:app));
        value.exec=(command,options,callback)=>command==='[ -d /data/adb/modules/.ommega ]' && !window.__bootstrapReleased
          ? window.__bootstrapHeld.push({command,options,callback}) : original(command,options,callback);
      }});
    });
    const page=await context.newPage(), errors=[];let searchIme;
    page.on('pageerror',error=>errors.push(error.message));
    await page.goto(`http://127.0.0.1:4179/${side}/`,{waitUntil:'domcontentloaded'});
    const selector=side==='a'?'#om-refresh':'#runtime-refresh';
    const button=page.locator(selector);
    await page.waitForFunction(selector=>document.querySelector(selector)?.getAttribute('aria-busy')==='false',selector);
    if(side==='a') {
      searchIme=await checkASearchIme(page,context,button,selector);
      assert.equal(await page.locator('[id="checkbox-com.example.wallet"]').evaluate(el=>el.checked),true);
      await page.locator('#om-app-search').fill('');
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
        const field=document.getElementById('om-app-search');
        return {tag:field.tagName,type:field.type,readOnly:field.readOnly,disabled:field.disabled,inputMode:field.inputMode,insideShadow:Boolean(field.shadowRoot)};
      });
      assert.equal(searchField.tag,'INPUT');
      assert.equal(searchField.type,'text');
      assert.equal(searchField.readOnly,false);
      assert.equal(searchField.disabled,false);
      assert.notEqual(searchField.inputMode,'none');
      assert.equal(searchField.insideShadow,false);
      await page.locator('#om-app-search').fill('中文测试');
      assert.equal(await page.locator('#om-app-search').evaluate(el=>el.value),'中文测试');
      await page.locator('#om-app-search').fill('wallet');
      await page.evaluate(()=>{document.getElementById('checkbox-com.example.wallet').checked=false;});
      const before=await page.evaluate(()=>window.__previewCommands.length);
      await button.click();await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',selector);
      assert.equal(await page.locator('#om-app-search').evaluate(el=>el.value),'wallet');
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
    results.push({side,locale,theme,errors,commands:await page.evaluate(()=>window.__previewCommands.length),...(searchIme?{searchIme}:{})});
    await context.close();
    console.log(`PASS ${side} ${locale}: actual refresh wiring, retained search, status/error completion${side==='a'?', battery freshness, installed-app list and isolation':''}`);
  }
  await writeFile(new URL('refresh-battery-results.json',output),JSON.stringify(results,null,2));
}finally{await browser.close();}
