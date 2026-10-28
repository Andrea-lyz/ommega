// Run against `node ui/preview.mjs`. Uses mock root responses only.
// PLAYWRIGHT_MODULE can point to a bundled Playwright installation.
import {createRequire} from 'node:module';
import {mkdir,writeFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';
const require = createRequire(import.meta.url);
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const previewOrigin = process.env.OMMEGA_PREVIEW_ORIGIN || 'http://127.0.0.1:4179';
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
    await page.goto(`${previewOrigin}/${side}/`,{waitUntil:'domcontentloaded'});
    await page.locator(side==='a'?'#om-updated':'#runtime-time').filter({hasText:/状态读取于|Status read at/}).waitFor();
    assert.equal(await page.locator('html').getAttribute('lang'),locale.startsWith('zh')?'zh-CN':'en');
    const refresh = page.locator(side==='a'?'#om-refresh':'#runtime-refresh');
    await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',side==='a'?'#om-refresh':'#runtime-refresh');
    // Busy/queued refresh semantics are covered deterministically by
    // ui/tests/refresh.test.mjs; this browser run verifies a real click completes
    // with a clean button and no hidden read failure.
    await refresh.click();
    await page.waitForFunction(selector=>document.querySelector(selector).getAttribute('aria-busy')==='false',side==='a'?'#om-refresh':'#runtime-refresh');
    assert.equal(await refresh.isEnabled(),true);
    assert.equal(await refresh.getAttribute('aria-label'),locale.startsWith('zh')?'刷新状态':'Refresh status');
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
      const fileLogging = page.locator('#file-log-enabled');
      assert.equal(await fileLogging.getAttribute('role'),'switch');
      assert.equal(await fileLogging.evaluate(element=>getComputedStyle(element).appearance),'none');
      await fileLogging.uncheck();
      assert.equal(await fileLogging.isChecked(),false);
      await fileLogging.check();
      await page.locator('#token-toggle').click();
      assert.equal(await page.locator('#relay-token').getAttribute('type'),'text');
      await page.locator('[data-view=settings]').click();
      await page.locator('#spl-open').click();
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
      assert.equal(await page.locator('#om-b-relay-id').textContent(),'dedicated-b');
      await page.locator('[data-om-view=settings]').click();
      const compat = page.locator('#ommega-global-compat');
      await page.waitForFunction(()=>document.getElementById('ommega-global-compat')?.disabled===false);
      await compat.check();
      await page.locator('#ommega-global-compat-status').filter({hasText:/已保存|Saved/}).waitFor();
      assert.equal(await compat.isChecked(),true);
      assert.equal(await page.locator('#mode-compat').count(),0);
      await compat.uncheck();
      await page.locator('#ommega-global-compat-status').filter({hasText:/已保存|Saved/}).waitFor();
      assert.equal(await compat.isChecked(),false);
      assert.deepEqual(await page.locator('#om-language option').evaluateAll(options=>options.map(option=>option.value)),['auto','zh-CN','en']);
      assert.deepEqual(await page.evaluate(async()=>Object.keys(await(await fetch('./locales/languages.json')).json()).sort()),['en','zh-CN']);
      assert.equal(await page.locator('#om-legacy-settings').isVisible(),false);
      for (const action of ['help','about']) {
        await page.locator(`[data-om-action=${action}]`).click();
        await page.locator(`#${action}-dialog`).waitFor({state:'visible'});
        if (action === 'help') {
          // A dialog covers the page: the navigation has to dim with it and
          // stop looking actionable.
          assert.equal(await page.evaluate(()=>document.body.classList.contains('om-dialog-open')),true);
          const navOpacity = await page.locator('.om-nav').evaluate(element=>getComputedStyle(element).opacity);
          assert.ok(Number(navOpacity) < 0.5,`the navigation must dim with the dialog (opacity ${navOpacity})`);
          // The back gesture closes the dialog instead of switching the view.
          const view = await page.evaluate(()=>document.body.dataset.omView);
          await page.goBack().catch(()=>{});
          await page.locator('#help-dialog').waitFor({state:'hidden'});
          assert.equal(await page.evaluate(()=>document.body.classList.contains('om-dialog-open')),false);
          assert.equal(await page.evaluate(()=>document.body.dataset.omView),view,'the back gesture must not switch the view');
          await page.locator(`[data-om-action=${action}]`).click();
          await page.locator(`#${action}-dialog`).waitFor({state:'visible'});
        }
        await page.locator(`#close-${action}`).click();
        await page.locator(`#${action}-dialog`).waitFor({state:'hidden'});
      }
      // The child precedes the parent in DOM order. Closing it must keep the
      // parent open, both through its button and through browser/native back.
      await page.locator('[data-om-action=about]').click();
      await page.locator('#about-dialog').waitFor({state:'visible'});
      const parentToken = await page.evaluate(()=>history.state.omDialog);
      for (const close of ['button','history','native']) {
        await page.locator('#help-dialog').evaluate(dialog=>dialog.show());
        await page.locator('#help-dialog').waitFor({state:'visible'});
        if (close === 'button') await page.locator('#close-help').click();
        else if (close === 'history') await page.goBack().catch(()=>{});
        else await page.evaluate(()=>window.dispatchEvent(new Event('back',{cancelable:true})));
        await page.locator('#help-dialog').waitFor({state:'hidden'});
        await page.waitForFunction(token=>history.state?.omDialog===token,parentToken);
        assert.equal(await page.locator('#about-dialog').isVisible(),true,`${close} must close only the child`);
        assert.equal(await page.evaluate(()=>document.body.classList.contains('om-dialog-open')),true);
        assert.equal(await page.evaluate(()=>document.body.dataset.omView),'settings');
      }
      await page.locator('#close-about').click();
      await page.locator('#about-dialog').waitFor({state:'hidden'});
      await page.waitForFunction(()=>!history.state?.omDialog);
      await page.locator('[data-om-action=remote-config]').click();
      await page.locator('#remote-config-dialog').waitFor({state:'visible'});
      const remoteToken = page.locator('#remote-config-token');
      assert.equal(await remoteToken.getAttribute('type'),'password');
      await page.locator('.om-remote-token button').click();
      assert.equal(await remoteToken.getAttribute('type'),'text');
      await page.locator('.om-remote-advanced summary').click();
      assert.equal(await page.locator('#remote-config-disable-native-strongbox').isVisible(),true);
      await page.locator('#cancel-remote-config').click();
      await page.locator('[data-om-action=ommega-soterta]').click();
      await page.locator('#ommega-soterta-dialog').waitFor({state:'visible'});
      assert.equal(await page.locator('#ommega-soterta-refresh').textContent(),locale.startsWith('zh')?'刷新':'Refresh');
      await page.locator('#ommega-soterta-close').click();
      await page.locator('[data-om-view=apps]').click();
      await page.locator('#apps-list').waitFor({state:'visible'});
      // The title and the search button are not surfaces: they carry no
      // background of their own.
      for (const selector of ['#title','#search-button']) {
        const background = await page.locator(selector).evaluate(element=>getComputedStyle(element).backgroundColor);
        assert.ok(/rgba\(0, 0, 0, 0\)|transparent/.test(background),`${selector} must not carry a background (${background})`);
      }
      // The list is the installed-app inventory, not the saved scope. Apps
      // that were never added to target.txt must still be present.
      for (const pkg of ['com.example.wallet','com.example.checker','com.example.chat','com.lingqing.trustattestor','com.ommega.authrepro']) {
        assert.equal(await page.locator(`.card[data-package="${pkg}"]`).count(),1,`${pkg} must be listed`);
      }
      // Saving is automatic, so no save FAB may cover the list; the last card
      // still has to scroll clear of the fixed navigation dock.
      await page.evaluate(()=>window.scrollTo(0,document.body.scrollHeight));
      const layout = await page.evaluate(()=>{
        const save=document.getElementById('save');
        const dock=document.querySelector('.om-nav').getBoundingClientRect();
        const cards=[...document.querySelectorAll('#apps-list .card')];
        return {saveVisible:Boolean(save&&save.getClientRects().length),dockTop:dock.top,lastBottom:cards[cards.length-1].getBoundingClientRect().bottom};
      });
      assert.equal(layout.saveVisible,false,'the vendor save FAB must not be shown');
      assert.ok(layout.lastBottom <= layout.dockTop,`the last card must clear the dock (${layout.lastBottom} > ${layout.dockTop})`);
      await page.screenshot({path:fileURLToPath(new URL(`a-apps-${locale}.png`,output)),fullPage:true});
      await page.locator('#menu-button').click();
      await page.waitForFunction(()=>document.getElementById('menu-options').open);
      assert.deepEqual(await page.locator('#menu-options md-menu-item').evaluateAll(items=>items.filter(item=>!item.hidden).map(item=>item.id)),['add-system-app']);
      assert.equal(await page.locator('#select-all').isVisible(),false);
      assert.equal(await page.locator('#deselect-all').isVisible(),false);
      assert.equal(await page.locator('#deselect-unnecessary').isVisible(),false);
      await page.screenshot({path:fileURLToPath(new URL(`a-app-menu-${locale}.png`,output)),fullPage:true,animations:'disabled'});
      await page.keyboard.press('Escape');
      await page.waitForFunction(()=>!document.getElementById('menu-options').open);
      // Scope is written only by the individual switch. Chat starts outside
      // target.txt, is added by its switch, and can be removed again.
      await page.locator('#search').fill('chat');
      await page.locator('.card[data-package="com.example.chat"]').click();
      await page.locator('#om-app-detail').waitFor({state:'visible'});
      await page.waitForFunction(()=>!document.getElementById('om-detail-scope').disabled);
      assert.equal(await page.locator('#om-detail-scope').isChecked(),false);
      await page.locator('#om-detail-scope').check();
      await page.locator('#om-detail-scope-status').filter({hasText:/已保存|Saved/}).waitFor();
      assert.equal(await page.evaluate(()=>targetList.includes('com.example.chat')),true);
      await page.locator('#om-detail-scope').uncheck();
      await page.locator('#om-detail-scope-status').filter({hasText:/已保存|Saved/}).waitFor();
      assert.equal(await page.evaluate(()=>targetList.includes('com.example.chat')),false);
      await page.locator('#om-detail-back').click();
      await page.locator('#om-app-detail').waitFor({state:'hidden'});
      await page.locator('#search').fill('wallet');
      await page.locator('.card[data-package="com.example.wallet"]').waitFor({state:'visible'});
      assert.equal(await page.locator('.card[data-package="com.example.checker"]').isVisible(),false);
      await page.locator('.card[data-package="com.example.wallet"]').click();
      await page.locator('#om-app-detail').waitFor({state:'visible'});
      await page.waitForFunction(()=>!document.querySelector('input[name="om-app-mode"][value="tee"]')?.disabled);
      await page.locator('input[name="om-app-mode"][value="tee"]').check();
      await page.locator('#om-detail-mode-status').filter({hasText:/已保存|Saved/}).waitFor();
      await page.locator('input[name="om-app-auth"][value="preserve"]').check();
      await page.locator('#om-detail-auth-status').filter({hasText:/已保存|Saved/}).waitFor();
      await page.locator('#om-detail-cpu').check();
      await page.locator('#om-detail-cpu-status').filter({hasText:/已保存|Saved/}).waitFor();
      await page.locator('#om-detail-back').click();
      await page.locator('#om-app-detail').waitFor({state:'hidden'});
      await page.locator('#ommega-global-scope-enabled').check();
      await page.waitForFunction(()=>document.getElementById('ommega-global-scope-enabled').checked && !document.getElementById('ommega-global-scope-enabled').disabled);
      await page.locator('.card[data-package="com.example.wallet"]').click();
      await page.locator('#om-app-detail').waitFor({state:'visible'});
      assert.equal(await page.locator('#om-detail-scope').isDisabled(),true);
      assert.equal(await page.locator('#om-detail-scope').isChecked(),true,'global scope covers every installed app');
      await page.waitForFunction(()=>!document.querySelector('input[name="om-app-mode"][value="tee"]')?.disabled);
      assert.equal(await page.locator('input[name="om-app-mode"][value="tee"]').isChecked(),true);
      assert.equal(await page.locator('input[name="om-app-auth"][value="preserve"]').isChecked(),true);
      assert.equal(await page.locator('#om-detail-cpu').isChecked(),true);
      await page.locator('#om-detail-back').click();
      await page.locator('#om-app-detail').waitFor({state:'hidden'});
      await page.locator('[data-om-view=settings]').click();
      // The badge carries the outcome; the record time stays in the card
      // footer, and a recovered relay is not reported as a failure.
      await page.locator('[data-om-view=overview]').click();
      const badge = page.locator('#om-relay-state');
      assert.equal(await badge.getAttribute('data-tone'),'ok');
      assert.doesNotMatch(await badge.textContent(),/\d{1,2}:\d{2}/);
      assert.match(await page.locator('#om-health-time').textContent(),/\d{1,2}:\d{2}/);
      assert.equal(await page.locator('#om-health-message').textContent(),'');
      // An error verdict shows the reason the relay gave.
      await page.evaluate(()=>{
        const now=Math.floor(Date.now()/1000);
        window.__OM_HEALTH__=JSON.stringify({last_ok_unix:now-120,last_error_unix:now-120,last_status:500,last_kind:'error',last_message:JSON.stringify({error:'no key for alias ommega-remote-1'}),last_verdict:'error'});
      });
      await page.locator('#om-refresh').click();
      await page.waitForFunction(()=>document.getElementById('om-relay-state').dataset.tone==='error');
      assert.match(await page.locator('#om-health-message').textContent(),/no key for alias ommega-remote-1/);
      await page.locator('[data-om-view=settings]').click();
    }
    await page.locator(side==='a'?'#om-appearance-open':'#appearance-open').click();
    await page.locator('#om-theme').selectOption(theme==='dark'?'light':'dark');
    assert.equal(await page.locator('html').getAttribute('data-theme'),theme==='dark'?'light':'dark');
    assert.deepEqual(errors,[],`${side} runtime errors`);
    const state = await page.evaluate(()=>({commands:window.__previewCommands.length,storage:Object.keys(localStorage)}));
    assert.ok(state.storage.every(key=>!/token|password/i.test(key)));
    results.push({side,locale,theme,errors,commands:state.commands});
    await context.close();
  }
  if (process.env.SKIP_OPENUI !== '1') {
  const design = await browser.newPage({viewport:{width:768,height:1000}});
  const designErrors = [];
  design.on('pageerror',error=>designErrors.push(error.message));
  await design.goto(`${previewOrigin}/openui/`,{waitUntil:'networkidle'});
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
  }
  await writeFile(new URL('browser-results.json',output),JSON.stringify(results,null,2));
  console.log(JSON.stringify(results));
} finally { await browser.close(); }
