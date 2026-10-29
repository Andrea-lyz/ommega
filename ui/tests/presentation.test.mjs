import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {resolveLanguage, parseProperties, freshness, escapeHtml, healthMessage} from '../shared/ui-kit.js';
const aSource = readFileSync(new URL('../../a-side/source/template/webroot/modern.js',import.meta.url),'utf8')
  .replace(/from '\.\/ui-kit\.js(?:\?[^']*)?'/,`from '${new URL('../shared/ui-kit.js',import.meta.url).href}'`)
  .replace(/from '\.\/app-search\.js(?:\?[^']*)?'/,`from '${new URL('../../a-side/source/template/webroot/app-search.js',import.meta.url).href}'`);
const {relayVerdict,requestIcon,deviceSnapshot,formatRelayLatency,parseBStatusConfig,bOnlineState,mergeAttestationHealth,bBatterySnapshot} = await import(`data:text/javascript;base64,${Buffer.from(aSource).toString('base64')}`);

test('Chinese locales map to Chinese; unsupported system languages fall back to English', () => {
  for (const locale of ['zh-CN','zh-TW','zh-HK','zh']) assert.equal(resolveLanguage('auto',[locale]),'zh-CN');
  for (const locale of ['en-US','de-DE','fr']) assert.equal(resolveLanguage('auto',[locale]),'en');
  assert.equal(resolveLanguage('en',['zh-CN']),'en');
  assert.equal(resolveLanguage('zh-CN',['en-US']),'zh-CN');
});
test('missing and stale SOTER snapshots never indicate a recent state', () => {
  assert.equal(freshness(undefined,1000000),'unknown');
  assert.equal(freshness(1100,1000000),'unknown');
  assert.equal(freshness(800,1000000),'stale');
  assert.equal(freshness(990,1000000),'recent');
});
test('relay history uses the newest outcome and handles absent files', () => {
  const now = 300 * 1000;
  assert.equal(relayVerdict(null,now).tone,'unknown');
  assert.equal(relayVerdict({last_ok_unix:100,last_error_unix:200},now).tone,'error');
  assert.equal(relayVerdict({last_ok_unix:200,last_error_unix:200},now).tone,'error');
  assert.equal(relayVerdict({last_ok_unix:300,last_error_unix:200},now).tone,'ok');
});
test('the published verdict decides a result the clocks cannot order', () => {
  const now = 1000 * 1000;
  // A failure and a success inside the same second tie the two timestamps, so
  // only the verdict the daemon publishes can say which one came last.
  assert.equal(relayVerdict({last_ok_unix:900,last_error_unix:900,last_verdict:'error'},now).tone,'error');
  assert.equal(relayVerdict({last_ok_unix:900,last_error_unix:900,last_verdict:'ok'},now).tone,'ok');
});
test('a completed attestation refreshes an old success but never hides a failure', () => {
  const ok = {last_verdict:'ok',last_ok_unix:100,last_error_unix:90};
  assert.equal(mergeAttestationHealth(ok,200000).last_ok_unix,200);
  const failed = {last_verdict:'error',last_ok_unix:100,last_error_unix:150};
  assert.equal(mergeAttestationHealth(failed,200000),failed);
  assert.equal(mergeAttestationHealth(null,200000).last_ok_unix,200);
});
test('an old result is a record, not a live alarm', () => {
  const now = 2000 * 1000;
  assert.equal(relayVerdict({last_ok_unix:100,last_error_unix:900},now).tone,'warn');
  assert.equal(relayVerdict({last_ok_unix:900,last_error_unix:100},now).tone,'muted');
});
test('the status emblem follows the recorded outcome and missing B properties stay unknown', () => {
  assert.notEqual(requestIcon('ok'),requestIcon('error'));
  assert.notEqual(requestIcon('error'),requestIcon('warn'));
  assert.equal(deviceSnapshot(null).model,'—');
  const device = deviceSnapshot({properties:{'ro.product.model':' B Model ', 'ro.product.brand':'Brand', 'ro.product.manufacturer':'Maker'}});
  assert.equal(device.model,'B Model');
  assert.equal(device.brand,'Brand / Maker');
  assert.equal(device.device,'—');
  assert.equal(formatRelayLatency(100),'100ms');
  assert.equal(formatRelayLatency(0),'0ms');
  assert.equal(formatRelayLatency(null),'—');
  assert.equal(formatRelayLatency(-1),'—');
});
test('the relay reason is shown without its JSON wrapper', () => {
  assert.equal(healthMessage({last_message:'{"error":"no key for alias ommega-remote-1"}'}),'no key for alias ommega-remote-1');
  assert.equal(healthMessage({last_message:'plain text'}),'plain text');
  assert.match(healthMessage({last_kind:'attestation'}), /validation|验证/i);
  assert.equal(healthMessage({}),'');
});
test('B online status comes from the configured device in the server response', () => {
  const config = parseBStatusConfig('url: https://relay.example.test/\ndevice_id: b-one\nremote: true\ntls_insecure: true\ntoken: secret\n');
  assert.equal(config.url,'https://relay.example.test');
  assert.equal(config.deviceId,'b-one');
  assert.equal(config.insecure,true);
  assert.equal(Object.hasOwn(config,'token'),false);
  assert.equal(bOnlineState({status:'ok',online_b_devices:[{device_id:'b-one'}]},'b-one'),'online');
  assert.equal(bOnlineState({status:'ok',online_b_devices:[{device_id:'b-two'}]},'b-one'),'offline');
  assert.equal(bOnlineState({status:'ok',online_b_devices:[{}]},'b-one'),'unknown');
  assert.equal(bOnlineState('not json','b-one'),'unknown');
  assert.equal(parseBStatusConfig('url: https://user:pass@example.test\ndevice_id: b'),null);
  assert.equal(parseBStatusConfig('url: https://relay.example.test\ndevice_id: b\nremote: false'),null);
});
test('battery telemetry uses exact B identity, fresh receipt and real nullable values', () => {
  const now = 1800000000000;
  const row = {device_id:'b-one',battery_percent:0,battery_charging:false,battery_updated_at_ms:now-1000};
  const status = {status:'ok',online_b_devices:[row]};
  assert.deepEqual(bBatterySnapshot(status,'b-one',now),{percent:0,charging:false,at:now-1000});
  row.battery_percent=100;row.battery_charging=true;
  assert.equal(bBatterySnapshot(status,'b-one',now).percent,100);
  assert.equal(bBatterySnapshot(status,'other',now),null);
  for (const value of [null,undefined,'76',NaN,Infinity,-1,101,75.5]) { row.battery_percent=value;assert.equal(bBatterySnapshot(status,'b-one',now),null); }
  row.battery_percent=76;row.battery_charging=null;
  assert.equal(bBatterySnapshot(status,'b-one',now).charging,null);
  for (const at of [null,undefined,0,now-90001,now+60001,'1800000000000']) { row.battery_updated_at_ms=at;assert.equal(bBatterySnapshot(status,'b-one',now),null); }
  row.battery_updated_at_ms=now;row.battery_charging='false';
  assert.equal(bBatterySnapshot(status,'b-one',now).charging,null);
  assert.equal(bBatterySnapshot({status:'ok',online_b_devices:[{device_id:'b-one'}]},'b-one',now),null);
  assert.equal(bBatterySnapshot('malformed','b-one',now),null);
  status.server_time_ms=now;row.battery_updated_at_ms=now-5000;
  assert.equal(bBatterySnapshot(status,'b-one',now+86400000).percent,76,'A clock skew must not invalidate a fresh server receipt');
  row.battery_updated_at_ms=now-90001;
  assert.equal(bBatterySnapshot(status,'b-one',now-86400000),null,'server-age stale reports cannot appear fresh through A clock skew');
});
test('overview data handles equals signs and markup as inert text', () => {
  assert.equal(parseProperties('URL=https://example.test?q=x=y\nOS=17').URL,'https://example.test?q=x=y');
  assert.equal(escapeHtml('<img onerror="x">'),'&lt;img onerror=&quot;x&quot;&gt;');
});
test('compatibility switch precedes the capability and SOTER entries', () => {
  const compatibility = aSource.indexOf('id="om-compat-settings-slot"');
  const capability = aSource.indexOf('id="om-capability-mask-slot"');
  const soter = aSource.indexOf("action('ommega-soterta'");
  assert.ok(compatibility >= 0 && compatibility < capability && capability < soter);
  const preview = readFileSync(new URL('../material-you-preview/app.js',import.meta.url),'utf8');
  const settings = preview.slice(preview.indexOf('function aSettings()'),preview.indexOf('function connection()'));
  assert.ok(settings.indexOf("toggle('正数密钥 ID 兼容'") < settings.indexOf("row('shield','系统能力屏蔽'"));
});
test('new B forms retain their controller targets and all static labels have English strings', () => {
  const html = readFileSync(new URL('../../b-side/source/template/webroot/index.html',import.meta.url),'utf8');
  const dictionary = readFileSync(new URL('../../b-side/source/template/webroot/i18n.js',import.meta.url),'utf8');
  const controller = readFileSync(new URL('../../b-side/source/template/webroot/app.js',import.meta.url),'utf8');
  const ids = new Set([...html.matchAll(/id="([^"]+)"/g)].map(match=>match[1]));
  for (const match of controller.matchAll(/\$\('([^']+)'\)/g)) assert.ok(ids.has(match[1]),'Missing controller target '+match[1]);
  for (const match of html.matchAll(/data-(?:msg|placeholder|label)="([^"]+)"/g)) assert.ok(dictionary.includes(`'${match[1]}':`),'Untranslated label '+match[1]);
});
