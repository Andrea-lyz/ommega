import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {resolveLanguage, parseProperties, freshness, escapeHtml, healthMessage} from '../shared/ui-kit.js';
const aSource = readFileSync(new URL('../../a-side/source/template/webroot/modern.js',import.meta.url),'utf8')
  .replace("from './ui-kit.js'",`from '${new URL('../shared/ui-kit.js',import.meta.url).href}'`);
const {relayVerdict} = await import(`data:text/javascript;base64,${Buffer.from(aSource).toString('base64')}`);

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
test('an old result is a record, not a live alarm', () => {
  const now = 2000 * 1000;
  assert.equal(relayVerdict({last_ok_unix:100,last_error_unix:900},now).tone,'warn');
  assert.equal(relayVerdict({last_ok_unix:900,last_error_unix:100},now).tone,'muted');
});
test('the relay reason is shown without its JSON wrapper', () => {
  assert.equal(healthMessage({last_message:'{"error":"no key for alias ommega-remote-1"}'}),'no key for alias ommega-remote-1');
  assert.equal(healthMessage({last_message:'plain text'}),'plain text');
  assert.equal(healthMessage({}),'');
});
test('overview data handles equals signs and markup as inert text', () => {
  assert.equal(parseProperties('URL=https://example.test?q=x=y\nOS=17').URL,'https://example.test?q=x=y');
  assert.equal(escapeHtml('<img onerror="x">'),'&lt;img onerror=&quot;x&quot;&gt;');
});
test('new B forms retain their controller targets and all static labels have English strings', () => {
  const html = readFileSync(new URL('../../b-side/source/template/webroot/index.html',import.meta.url),'utf8');
  const dictionary = readFileSync(new URL('../../b-side/source/template/webroot/i18n.js',import.meta.url),'utf8');
  const controller = readFileSync(new URL('../../b-side/source/template/webroot/app.js',import.meta.url),'utf8');
  const ids = new Set([...html.matchAll(/id="([^"]+)"/g)].map(match=>match[1]));
  for (const match of controller.matchAll(/\$\('([^']+)'\)/g)) assert.ok(ids.has(match[1]),'Missing controller target '+match[1]);
  for (const match of html.matchAll(/data-(?:msg|placeholder|label)="([^"]+)"/g)) assert.ok(dictionary.includes(`'${match[1]}':`),'Untranslated label '+match[1]);
});
