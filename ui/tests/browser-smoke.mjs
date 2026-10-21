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
    } else {
      await page.locator('[data-om-view=settings]').click();
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
