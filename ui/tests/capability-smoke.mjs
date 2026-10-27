// Host preview only: the bridge records commands and simulates config writes in memory.
import {createRequire} from 'node:module';
import {mkdir,readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import assert from 'node:assert/strict';

const require = createRequire(import.meta.url);
const {chromium} = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const output = new URL('../test-results/', import.meta.url);
const root = new URL('../../', import.meta.url);
const mock = 'window.__OM_FIXTURE__={raw:"",digest:""};\n' + await readFile(new URL('../preview-bridge.js',import.meta.url),'utf8');
await mkdir(output, {recursive:true});
const browser = await chromium.launch({channel:process.env.BROWSER_CHANNEL || 'msedge',headless:true});
try {
  for (const [locale,theme,width,height] of [
    ['zh-CN','light',320,568], ['zh-CN','dark',393,852],
    ['en-US','dark',768,480], ['en-US','light',1024,768],
  ]) {
    const context = await browser.newContext({viewport:{width,height},locale,colorScheme:theme,reducedMotion:'reduce'});
    // Fulfill the real packaged assets locally; no listening port or device is needed.
    await context.route('http://127.0.0.1:4179/**', async route => {
      const path = new URL(route.request().url()).pathname;
      if (path === '/preview-bridge.js') return route.fulfill({contentType:'text/javascript',body:mock});
      const match = /^\/a\/(.*)$/.exec(path);
      if (!match || match[1].includes('..')) return route.fulfill({status:404});
      const name = match[1] || 'index.html';
      try {
        let body = await readFile(new URL('a-side/source/template/webroot/' + name,root));
        if (name.endsWith('.html')) body = Buffer.from(body.toString().replace('<head>','<head><script src="/preview-bridge.js"></script>'));
        const type = {html:'text/html',js:'text/javascript',css:'text/css',json:'application/json',png:'image/png',woff2:'font/woff2'}[name.split('.').at(-1)] || 'application/octet-stream';
        await route.fulfill({contentType:type,body});
      } catch { await route.fulfill({status:404}); }
    });
    await context.route('https://**/*',route=>route.abort());
    const page = await context.newPage(), errors = [];
    page.on('pageerror',error=>errors.push(error.message));
    await page.goto('http://127.0.0.1:4179/a/#om-settings',{waitUntil:'domcontentloaded'});
    const open = page.locator('#ommega-capability-open');
    await open.click();
    const dialog = page.locator('#ommega-capability-dialog');
    await page.waitForFunction(()=>document.getElementById('ommega-capability-strongbox')?.disabled===false);
    const strongbox = page.locator('#ommega-capability-strongbox');
    const deviceId = page.locator('#ommega-capability-device-id');
    assert.equal(await strongbox.isChecked(),false);
    assert.equal(await deviceId.isChecked(),false);
    assert.equal(await dialog.getByRole('switch').count(),2);
    assert.ok(await strongbox.evaluate(element=>element.getBoundingClientRect().width>=44));
    assert.equal(await strongbox.evaluate(element=>getComputedStyle(element).appearance),'none');
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth+1));
    assert.ok(await dialog.evaluate(element=>element.scrollWidth <= element.clientWidth+1));
    assert.ok(await page.locator('.ommega-capability-row').first().evaluate(element=>element.getBoundingClientRect().height>=48));
    await strongbox.check();
    assert.equal(await deviceId.isChecked(),false);
    await page.locator('#ommega-capability-save').click();
    await page.locator('#ommega-capability-status').filter({hasText:/已保存，重启设备后生效|Saved\. Restart/}).waitFor();
    assert.equal(await dialog.isVisible(),true);
    await page.screenshot({path:fileURLToPath(new URL(`capability-${locale}-${theme}-${width}.png`,output))});
    await page.locator('#ommega-capability-back').click();
    await open.click();
    await page.waitForFunction(()=>document.getElementById('ommega-capability-strongbox')?.disabled===false);
    assert.equal(await strongbox.isChecked(),true);
    assert.equal(await deviceId.isChecked(),false);
    await strongbox.uncheck();
    await deviceId.check();
    await page.locator('#ommega-capability-save').click();
    await page.locator('#ommega-capability-status').filter({hasText:/已保存，重启设备后生效|Saved\. Restart/}).waitFor();
    const writes = await page.evaluate(()=>window.__previewCommands.filter(command=>command.includes('ommega-capability-policy')));
    assert.ok(writes.some(command=>command.endsWith('"$ommega_capability_writer" write')));
    assert.ok(writes.every(command=>! /\b(?:prepare|reboot|restart|kill|mount|setprop)\b|\badb\s/.test(command)));
    await page.goBack();
    await dialog.waitFor({state:'hidden'});
    assert.equal(await page.locator('#om-settings').isVisible(),true);
    await page.evaluate(()=>{window.__OM_CAPABILITY_MOUNT__=null;});
    await open.click();
    await page.locator('#ommega-capability-mount-note').filter({hasText:/尚未挂载|not mounted/}).waitFor();
    assert.equal(await strongbox.isChecked(),false);
    assert.equal(await deviceId.isChecked(),true);
    assert.deepEqual(errors,[]);
    console.log(`PASS capability dialog: ${locale} ${theme} ${width}x${height}, independent saves, back and absent mount`);
    await context.close();
  }
} finally { await browser.close(); }
