import test from 'node:test';
import assert from 'node:assert/strict';
import {pollWhenVisible} from '../shared/ui-kit.js';

function fixture(t, read) {
  const events = new Map(), attrs = new Map([['aria-label','Refresh status']]);
  const button = {disabled:false,setAttribute:(k,v)=>attrs.set(k,v),getAttribute:k=>attrs.get(k)};
  const previous = Object.fromEntries(['window','document','setInterval','clearInterval'].map(key=>[key,globalThis[key]]));
  const document = {hidden:false,addEventListener:(name,fn)=>events.set(name,fn),removeEventListener:name=>events.delete(name)};
  const window = {addEventListener:(name,fn)=>events.set(name,fn)};
  let tick;
  Object.assign(globalThis,{window,document,setInterval:fn=>{tick=fn;return 1;},clearInterval:()=>{tick=null;}});
  t.after(()=>Object.assign(globalThis,previous));
  return {button,attrs,events,document,manual:pollWhenVisible(read,1000,button),tick:()=>tick?.()};
}
const flush = async () => { for(let i=0;i<12;i++) await Promise.resolve(); };

test('manual refresh bypasses hidden state, passes force and waits for queued follow-up', async t => {
  const calls=[],release=[];
  const ui=fixture(t,forced=>{calls.push(forced);return new Promise(resolve=>release.push(resolve));});
  await flush();
  assert.deepEqual(calls,[false]);
  let finished=false;
  const pending=ui.manual().then(()=>{finished=true;});
  const coalesced=ui.manual();
  ui.document.hidden=true;
  assert.equal(ui.button.disabled,true);
  assert.equal(ui.attrs.get('aria-busy'),'true');
  release.shift()();await flush();
  assert.deepEqual(calls,[false,true]);
  assert.equal(finished,false);
  assert.equal(ui.button.disabled,true);
  release.shift()();await Promise.all([pending,coalesced]);
  assert.equal(finished,true);
  assert.equal(ui.button.disabled,false);
  assert.equal(ui.attrs.get('aria-busy'),'false');
  ui.tick();await flush();assert.equal(calls.length,2);
  const hidden=ui.manual();await flush();assert.deepEqual(calls,[false,true,true]);
  release.shift()();await hidden;
});

test('failed automatic read does not swallow a queued manual retry; pagehide stops polling', async t => {
  let reject,resolve,count=0;
  const ui=fixture(t,()=>++count===1 ? new Promise((_,r)=>{reject=r;}) : new Promise(r=>{resolve=r;}));
  await flush();
  const manual=ui.manual();reject(new Error('mock read failed'));await flush();
  assert.equal(count,2);assert.equal(ui.button.disabled,true);
  resolve();await manual;assert.equal(ui.button.disabled,false);
  ui.events.get('pagehide')();await ui.manual();assert.equal(count,2);
});

test('manual errors release busy state and remain awaitable', async t => {
  let fail=false;
  const ui=fixture(t,()=>{if(fail)throw new Error('failed');});
  await flush();fail=true;
  await assert.rejects(ui.manual(),/failed/);
  assert.equal(ui.button.disabled,false);assert.equal(ui.attrs.get('aria-busy'),'false');
  fail=false;await ui.manual();
});
