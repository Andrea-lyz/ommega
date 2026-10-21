// Run against `node ui/preview.mjs`. Uses mock root responses only.
// PLAYWRIGHT_MODULE can point to a bundled Playwright installation.
import {createRequire} from 'node:module';
import {mkdir,writeFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
const require = createRequire(import.meta.url);
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const output = new URL('../test-results/',import.meta.url);
await mkdir(output,{recursive:true});
const browser = await chromium.launch({channel:process.env.BROWSER_CHANNEL || 'msedge',headless:true});
const results = [];
try {
  for (const [side,locale,theme] of [['a','zh-CN','dark'],['a','en-US','light'],['b','zh-CN','dark'],['b','en-US','light']]) {
    const context = await browser.newContext({viewport:{width:393,height:852},locale,colorScheme:theme,reducedMotion:'reduce'});
    await context.route('https://**/*',route=>route.abort()); // Prove the module UI can render offline.
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror',error=>errors.push(error.message));
    await page.goto(`http://127.0.0.1:4179/${side}/`,{waitUntil:'domcontentloaded'});
    await page.locator(side==='a'?'#om-updated':'#runtime-time').filter({hasText:/状态读取于|Status read at/}).waitFor();
    assert.equal(await page.locator('html').getAttribute('lang'),locale.startsWith('zh')?'zh-CN':'en');
    const refresh = page.locator(side==='a'?'#om-refresh':'#runtime-refresh');
    await page.evaluate(prefix => {
      window.__originalExec = window.ksu.exec;
      window.__heldRefreshes = [];
      window.ksu.exec = (command,options,callback) => {
        if (callback.startsWith(prefix)) window.__heldRefreshes.push({command,options,callback});
        else window.__originalExec(command,options,callback);
      };
    },side==='a'?'ommega_a_dashboard_':'ommega_b_status_');
    for (const failed of [true,false]) {
      await refresh.click();
      assert.equal(await refresh.isDisabled(),true);
      assert.equal(await refresh.getAttribute('aria-busy'),'true');
      assert.equal(await refresh.getAttribute('aria-label'),locale.startsWith('zh')?'正在刷新':'Refreshing');
      assert.equal(await refresh.locator('svg').evaluate(svg=>getComputedStyle(svg).animationName),'none');
      await page.emulateMedia({reducedMotion:'no-preference'});
      assert.equal(await refresh.locator('svg').evaluate(svg=>getComputedStyle(svg).animationName),'om-refresh-spin');
      await page.emulateMedia({reducedMotion:'reduce'});
      await refresh.evaluate(button=>button.click());
      assert.equal(await page.evaluate(()=>window.__heldRefreshes.length),1,'Busy refresh must not send another read');
      await page.evaluate(failed => {
        const {command,options,callback} = window.__heldRefreshes.shift();
        if (failed) window[callback](1,'','Preview read failed');
        else window.__originalExec(command,options,callback);
      },failed);
      await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',side==='a'?'#om-refresh':'#runtime-refresh');
      assert.equal(await refresh.isEnabled(),true);
      assert.equal(await refresh.getAttribute('aria-label'),locale.startsWith('zh')?'刷新状态':'Refresh status');
      if (failed) assert.equal(await page.locator(side==='a'?'#om-updated':'#runtime-time').textContent(),'Preview read failed');
    }
    await page.evaluate(()=>{window.ksu.exec=window.__originalExec;});
    await refresh.screenshot({path:fileURLToPath(new URL(`${side}-refresh-${locale}.png`,output))});
    await page.screenshot({path:fileURLToPath(new URL(`${side}-${locale}-${theme}.png`,output)),fullPage:true});
    for (const width of [320,393,768,1024]) {
      await page.setViewportSize({width,height:852});
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth+1),`${side} overflow at ${width}`);
    }
    await page.setViewportSize({width:393,height:852});
    if (side === 'b') {
      await page.locator('[data-view=connection]').click();
      await page.locator('#relay-server').waitFor({state:'visible'});
      assert.equal(await page.locator('#relay-device').inputValue(),'dedicated-b');
      await page.locator('#token-toggle').click();
      assert.equal(await page.locator('#relay-token').getAttribute('type'),'text');
      await page.locator('[data-view=settings]').click();
      await page.locator('#spl-section > summary').click();
      assert.match(await page.locator('#origin').textContent(),/System 2025-12-01/);
      assert.match(await page.locator('#spl-floor').textContent(),/System 2026-08-01/);
      assert.match(await page.locator('#spl-applied').textContent(),/Boot 2025-12-01/);
      assert.equal(await page.locator('#osversion').getAttribute('placeholder'),locale.startsWith('zh')?'如 17，留空保持当前生效值':'e.g. 17; blank keeps the current value');
      await page.getByRole('button',{name:locale.startsWith('zh')?'留空保持当前值':'Keep current values',exact:true}).click();
      assert.equal(await page.locator('#system').inputValue(),'');
      assert.equal(await page.locator('#osversion').inputValue(),'');
      await page.screenshot({path:fileURLToPath(new URL(`b-spl-${locale}.png`,output)),fullPage:true});
      await page.reload({waitUntil:'domcontentloaded'}); // Discard the unsaved fixture-only draft.
      await page.waitForFunction(()=>document.getElementById('system').value==='2026-08-01');
    } else {
      await page.locator('[data-om-view=settings]').click();
      assert.deepEqual(await page.locator('#om-language option').evaluateAll(options=>options.map(option=>option.value)),['auto','zh-CN','en']);
      assert.deepEqual(await page.evaluate(async()=>Object.keys(await(await fetch('./locales/languages.json')).json()).sort()),['en','zh-CN']);
      assert.equal(await page.locator('#om-legacy-settings').isVisible(),false);
      for (const action of ['help','about']) {
        await page.locator(`[data-om-action=${action}]`).click();
        await page.locator(`#${action}-dialog`).waitFor({state:'visible'});
        await page.locator(`#close-${action}`).click();
      }
      await page.locator('[data-om-action=remote-config]').click();
      await page.locator('#remote-config-dialog').waitFor({state:'visible'});
      await page.locator('#cancel-remote-config').click();
      await page.locator('[data-om-action=ommega-soterta]').click();
      await page.locator('#ommega-soterta-dialog').waitFor({state:'visible'});
      assert.equal(await page.locator('#ommega-soterta-refresh').textContent(),locale.startsWith('zh')?'刷新':'Refresh');
      await page.locator('#ommega-soterta-close').click();
      await page.locator('[data-om-view=apps]').click();
      await page.locator('#apps-list').waitFor({state:'visible'});
      await page.screenshot({path:fileURLToPath(new URL(`a-apps-${locale}.png`,output)),fullPage:true});
      await page.locator('#menu-button').click();
      await page.locator('#select-denylist').waitFor({state:'visible'});
      assert.deepEqual(await page.locator('#menu-options md-menu-item').evaluateAll(items=>items.map(item=>item.id)),['select-denylist','deselect-unnecessary','add-system-app']);
      await page.screenshot({path:fileURLToPath(new URL(`a-app-menu-${locale}.png`,output)),fullPage:true,animations:'disabled'});
      await page.keyboard.press('Escape');
      await page.locator('[data-om-view=settings]').click();
    }
    await page.locator('#om-theme').selectOption(theme==='dark'?'light':'dark');
    assert.equal(await page.locator('html').getAttribute('data-theme'),theme==='dark'?'light':'dark');
    assert.deepEqual(errors,[],`${side} runtime errors`);
    const state = await page.evaluate(()=>({commands:window.__previewCommands.length,storage:Object.keys(localStorage)}));
    assert.ok(state.storage.every(key=>!/token|password/i.test(key)));
    results.push({side,locale,theme,errors,commands:state.commands});
    await context.close();
  }
  const design = await browser.newPage({viewport:{width:768,height:1000}});
  const designErrors = [];
  design.on('pageerror',error=>designErrors.push(error.message));
  await design.goto('http://127.0.0.1:4179/openui/',{waitUntil:'networkidle'});
  await design.getByRole('button',{name:'A / B',exact:true}).click();
  await design.getByRole('heading',{name:'硬件证明，随时就绪。'}).waitFor();
  assert.deepEqual(designErrors,[],'OpenUI design renderer errors');
  assert.equal(await design.locator('html').getAttribute('data-theme'),'dark');
  await design.getByRole('button',{name:'切换外观',exact:true}).click();
  assert.equal(await design.locator('html').getAttribute('data-theme'),'light');
  await design.getByRole('button',{name:'切换外观',exact:true}).click();
  await design.screenshot({path:fileURLToPath(new URL('openui-design.png',output)),fullPage:true});
  results.push({surface:'OpenUI design preview',errors:designErrors});
  await design.close();
  await writeFile(new URL('browser-results.json',output),JSON.stringify(results,null,2));
  console.log(JSON.stringify(results));
} finally { await browser.close(); }
