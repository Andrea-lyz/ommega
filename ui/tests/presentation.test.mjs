import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {resolveLanguage, parseProperties, freshness, escapeHtml, healthMessage} from '../shared/ui-kit.js';
const aSource = readFileSync(new URL('../../a-side/source/template/webroot/modern.js',import.meta.url),'utf8')
  .replace(/from '\.\/ui-kit\.js(?:\?[^']*)?'/,`from '${new URL('../shared/ui-kit.js',import.meta.url).href}'`)
  .replace(/from '\.\/app-search\.js(?:\?[^']*)?'/,`from '${new URL('../../a-side/source/template/webroot/app-search.js',import.meta.url).href}'`)
  .replace(/from '\.\/soter\.js(?:\?[^']*)?'/,`from '${new URL('../../a-side/source/template/webroot/soter.js',import.meta.url).href}'`);
const {relayVerdict,requestIcon,deviceSnapshot,formatRelayLatency,parseBStatusConfig,bOnlineState,mergeAttestationHealth,bBatterySnapshot,
  relayMode,parseRelaySettings,parseTomlRemote,certificateBytes,certificateSummary,keyboxSummary,formatKeyboxDays,
  localActivityVerdict,localVerdictIcon,parseLogTime,formatAge} = await import(`data:text/javascript;base64,${Buffer.from(aSource).toString('base64')}`);

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

/** Minimal DER encoder so the keybox summary can be tested without a real chain. */
const derTlv = (tag, content) => {
  const body = Buffer.from(content);
  if (body.length < 0x80) return Buffer.concat([Buffer.from([tag, body.length]), body]);
  const length = [];
  for (let value = body.length; value > 0; value = Math.floor(value / 256)) length.unshift(value % 256);
  return Buffer.concat([Buffer.from([tag, 0x80 | length.length]), Buffer.from(length), body]);
};
const derSeq = (...parts) => derTlv(0x30, Buffer.concat(parts));
const derOid = bytes => derTlv(0x06, Buffer.from(bytes));
const derString = value => derTlv(0x13, Buffer.from(value, 'ascii'));
function testCertificate({o = 'TEE', notAfter = '270101000000Z'} = {}) {
  const name = value => derSeq(derTlv(0x31, derSeq(derOid([0x55,0x04,0x0a]), derString(value))));
  const validity = derSeq(derTlv(0x17, Buffer.from('250101000000Z', 'ascii')), derTlv(0x17, Buffer.from(notAfter, 'ascii')));
  const tbs = derSeq(derTlv(0xa0, derTlv(0x02, Buffer.from([2]))), derTlv(0x02, Buffer.from([1])),
    derSeq(), name('Issuer Org'), validity, name(o), derSeq());
  return derSeq(tbs, derSeq(), derTlv(0x03, Buffer.from([0])));
}
const pem = der => '-----BEGIN CERTIFICATE-----\n' + der.toString('base64').replace(/(.{64})/g, '$1\n') + '-----END CERTIFICATE-----';
const FIXTURE_NOW = Date.UTC(2026, 10, 2);

test('local mode is the effective flat relay setting, with config.toml as the fallback', () => {
  assert.equal(relayMode('url: https://relay.example.test\ndevice_id: b-one\nremote: false\n', ''), 'local');
  assert.equal(relayMode('url: https://relay.example.test\ndevice_id: b-one\nremote: true\n', ''), 'remote');
  assert.equal(relayMode('url: https://relay.example.test\ndevice_id: b-one\n', ''), 'remote');
  assert.equal(relayMode('url: none\ndevice_id: b-one\n', ''), 'local');
  assert.equal(relayMode('', '[remote]\nenabled = true\nurl = "https://relay.example.test"\n'), 'remote');
  assert.equal(relayMode('', '[remote]\nenabled = false\nurl = "https://relay.example.test"\n'), 'local');
  assert.equal(relayMode('', ''), 'unknown');
  assert.equal(relayMode('url: https://flat.test\ndevice_id: b-one\nremote: false\n',
    '[remote]\nenabled = true\nurl = "https://table.test"\n'), 'local', 'the flat file wins, as it does in config.rs');
  assert.equal(parseRelaySettings(''), null);
  assert.equal(parseTomlRemote(''), null);
  assert.equal(parseRelaySettings('url: https://relay.example.test\ndevice_id: b-one\ntls_insecure: true\n').insecure, true);
});

test('keybox material reports the level it was issued for and flags a wrong slot', () => {
  const tee = keyboxSummary(pem(testCertificate({o: 'TEE'})), 'TEE', FIXTURE_NOW);
  assert.equal(tee.state, 'loaded');
  assert.equal(tee.level, 'TEE');
  assert.equal(tee.mismatch, false);
  assert.equal(tee.o, 'TEE', 'the leaf subject, never the issuer');

  const strongbox = keyboxSummary(pem(testCertificate({o: 'StrongBox'})), 'StrongBox', FIXTURE_NOW);
  assert.equal(strongbox.level, 'StrongBox');
  assert.equal(strongbox.mismatch, false);

  const wrongSlot = keyboxSummary(pem(testCertificate({o: 'StrongBox'})), 'TEE', FIXTURE_NOW);
  assert.equal(wrongSlot.level, 'StrongBox');
  assert.equal(wrongSlot.mismatch, true, 'a StrongBox chain in the TEE slot is what the detector reports as A20');

  assert.equal(keyboxSummary('', 'TEE', FIXTURE_NOW).state, 'missing');
  assert.equal(keyboxSummary('   ', 'TEE', FIXTURE_NOW).state, 'missing');
  assert.equal(keyboxSummary('-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----', 'TEE', FIXTURE_NOW).state, 'invalid');
  // The chain is a sequence: a second block must never be glued onto the leaf.
  const chained = pem(testCertificate({o: 'TEE'})) + '\n' + pem(testCertificate({o: 'StrongBox'}));
  assert.equal(keyboxSummary(chained, 'TEE', FIXTURE_NOW).level, 'TEE');
  assert.equal(keyboxSummary(chained, 'TEE', FIXTURE_NOW).days, 60, 'the default fixture expires 2027-01-01');
  assert.equal(certificateBytes('@@@'), null, 'nothing decodable');
  assert.equal(keyboxSummary('not a certificate', 'TEE', FIXTURE_NOW).state, 'invalid');
  assert.equal(certificateSummary(new Uint8Array([0x31, 0x00])), null);
});



test('the local verdict follows the keymint log, not the relay counters', () => {
  const now = Date.UTC(2026, 10, 2, 0, 0, 0);
  assert.equal(localActivityVerdict(0, now).tone, 'unknown');
  assert.equal(localActivityVerdict(undefined, now).tone, 'unknown');

  const fresh = localActivityVerdict(now - 5000, now);
  assert.equal(fresh.tone, 'ok');
  assert.equal(fresh.failed, false);
  assert.equal(fresh.at, Math.floor((now - 5000) / 1000));
  assert.match(fresh.label, /刚刚|just now/i);

  const stale = localActivityVerdict(now - 3600000, now);
  assert.equal(stale.tone, 'muted', 'an hour-old record stays a record, not an alarm');
  assert.match(stale.label, /1 小时前|1 h ago/);
  assert.equal(localActivityVerdict(now + 30000, now).tone, 'ok', 'a little clock skew is tolerated');

  // Local mode never reads remote-health.json, so a relay failure cannot make
  // this card report one.
  assert.notEqual(localActivityVerdict(now - 5000, now).label, relayVerdict({last_verdict:'error', last_error_unix:1, last_ok_unix:0}, now).label);
});

test('the keymint log line drives the local timestamp', () => {
  const line = '2026-11-02 00:12:49 UTC [INFO] kmr_ta::keys - event=generate_total total_us=593';
  assert.equal(parseLogTime(line), Date.UTC(2026, 10, 2, 0, 12, 49));
  assert.equal(parseLogTime(''), 0);
  assert.equal(parseLogTime('no timestamp here'), 0);
  assert.equal(formatAge(0), '—');
  assert.match(formatAge(Date.UTC(2026, 10, 2, 0, 12, 49), Date.UTC(2026, 10, 2, 0, 13, 40)), /刚刚|just now/);
  assert.match(formatAge(Date.UTC(2026, 9, 1), Date.UTC(2026, 10, 2)), /天|d ago/);
});

test('the local emblem keeps the shield the relay icon drops on an old result', () => {
  for (const tone of ['ok','muted','unknown','error','warn']) {
    assert.notEqual(localVerdictIcon(tone), requestIcon(tone), 'local icon must differ for ' + tone);
    assert.match(localVerdictIcon(tone), /M12 2\.8 19 5\.5/, 'local icon always draws the shield');
  }
  assert.equal(requestIcon('muted').includes('M12 2.8 19 5.5'), false, 'the relay icon drops the shield when stale');
  assert.notEqual(localVerdictIcon('ok'), localVerdictIcon('muted'), 'the inner mark still follows the outcome');
});

test('the local-mode overview targets exist and the status read collects them', () => {
  for (const id of ['om-b-summary','om-local-summary','om-local-tee','om-local-strongbox','om-local-last',
    'om-local-attest','om-local-scope','om-local-mode','om-local-relay','om-local-expiry','om-local-warnings',
    'om-request-kind','om-route-mid','om-route-mid-detail','om-route-end','om-route-end-detail',
    'om-verdict-label','om-verdict-title','om-health-time-label','om-health-kind-label']) {
    assert.ok(aSource.includes('id="' + id + '"'), 'Missing overview target ' + id);
  }
  const required = /const complete = \[([^\]]+)\]/.exec(aSource);
  assert.ok(required, 'the status read must declare its required sections');
  for (const name of ['MODE','REMOTE','KEYBOX_TEE','KEYBOX_STRONGBOX','ACTIVITY']) {
    assert.ok(aSource.includes('[' + name + ']'), 'the status command must emit ' + name);
    assert.ok(required[1].includes("'" + name + "'"), 'the completeness list must require ' + name);
  }
});

test('keybox validity counts down and never hides an expired chain', () => {
  const certificate = pem(testCertificate({notAfter: '260102000000Z'}));
  assert.equal(keyboxSummary(certificate, 'TEE', Date.UTC(2026, 0, 2)).days, 0);
  assert.equal(keyboxSummary(certificate, 'TEE', Date.UTC(2026, 0, 3)).days, -1);
  assert.equal(keyboxSummary(certificate, 'TEE', Date.UTC(2025, 11, 1)).days, 32);
  assert.match(formatKeyboxDays(0), /今天|today/i);
  assert.match(formatKeyboxDays(-1), /过期|expired/i);
  assert.match(formatKeyboxDays(9), /9/);
  assert.equal(formatKeyboxDays(null), '—');
});

