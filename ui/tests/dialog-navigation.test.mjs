import test from 'node:test';
import assert from 'node:assert/strict';
import {installDialogGuard} from '../shared/ui-kit.js';

// A host-only history/event model: no browser, root bridge or device calls.
async function fixture(run) {
  const saved = new Map(['window','document','history','MutationObserver'].map(key=>[key,globalThis[key]]));
  const window = new EventTarget();
  let notify = () => {};
  class Dialog extends EventTarget {
    constructor(localName) { super(); this.localName=localName; this.open=false; }
    getAttribute(name) { return name==='data-om-return-focus' ? this.returnFocus || null : null; }
    show() { this.open=true; notify(); }
    close() { this.open=false; notify(); }
  }
  const parent = new Dialog('dialog'), child = new Dialog('dialog'), legacy = new Dialog('md-dialog');
  const entries = [null]; let index=0;
  const history = {
    get state() { return entries[index]; },
    pushState(state) { entries.splice(++index); entries[index]=state; },
    back() { if(index>0) { index--; queueMicrotask(()=>window.dispatchEvent(new Event('popstate'))); } },
    forward() { if(index+1<entries.length) { index++; queueMicrotask(()=>window.dispatchEvent(new Event('popstate'))); } },
  };
  globalThis.window=window; globalThis.history=history;
  let focused=null;
  globalThis.document={
    getElementById:id=>({disabled:false,getClientRects:()=>[{}],focus:()=>{focused=id;}}),
    querySelectorAll:selector=>[parent,child,legacy].filter(dialog=>dialog.open &&
      (dialog.localName==='md-dialog' || selector.split(',').includes('dialog[open]'))),
  };
  globalThis.MutationObserver=class {constructor(callback) {notify=()=>queueMicrotask(callback);} observe() {}};
  let covered=false;
  installDialogGuard(value=>{covered=value;},{includeNative:true});
  const settle = () => new Promise(resolve=>setImmediate(resolve));
  try { await run({parent,child,legacy,history,window,settle,covered:()=>covered,focused:()=>focused}); }
  finally { for(const [key,value] of saved) {if(value===undefined) delete globalThis[key]; else globalThis[key]=value;} }
}

test('browser back closes the full-screen child, then the policy, one level at a time', async () => fixture(async f=>{
  f.parent.show(); await f.settle(); const parentToken=f.history.state.omDialog;
  f.child.show(); await f.settle(); assert.notEqual(f.history.state.omDialog,parentToken);
  f.history.back(); await f.settle();
  assert.equal(f.child.open,false); assert.equal(f.parent.open,true);
  assert.equal(f.history.state.omDialog,parentToken); assert.equal(f.covered(),true);
  f.history.back(); await f.settle();
  assert.equal(f.parent.open,false); assert.equal(f.covered(),false); assert.equal(f.history.state,null);
}));

test('each native WebView back event closes only the topmost layer', async () => {
  for(const type of ['back','backbutton']) await fixture(async f=>{
    f.parent.show(); await f.settle(); const parentToken=f.history.state.omDialog;
    f.child.show(); await f.settle();
    const event=new Event(type,{cancelable:true}); f.window.dispatchEvent(event); await f.settle();
    assert.equal(event.defaultPrevented,true);
    assert.equal(f.child.open,false); assert.equal(f.parent.open,true);
    assert.equal(f.history.state.omDialog,parentToken);
  });
});

test('a child button or Escape consumes its own history entry without closing the parent', async () => {
  for(const close of ['button','cancel']) await fixture(async f=>{
    f.parent.show(); await f.settle(); const parentToken=f.history.state.omDialog;
    f.child.show(); await f.settle();
    if(close==='button') f.child.close(); else f.child.dispatchEvent(new Event('cancel',{cancelable:true}));
    await f.settle();
    assert.equal(f.child.open,false); assert.equal(f.parent.open,true);
    assert.equal(f.history.state.omDialog,parentToken); assert.equal(f.covered(),true);
  });
});

test('forward into a closed child does not close or replace the open parent', async () => fixture(async f=>{
  f.parent.show(); await f.settle(); f.child.show(); await f.settle();
  f.history.back(); await f.settle(); f.history.forward(); await f.settle();
  assert.equal(f.parent.open,true); assert.equal(f.child.open,false); assert.equal(f.covered(),true);
}));

test('legacy Material dialogs keep the same history ownership', async () => fixture(async f=>{
  f.legacy.show(); await f.settle(); assert.equal(f.covered(),true);
  f.window.dispatchEvent(new Event('back',{cancelable:true})); await f.settle();
  assert.equal(f.legacy.open,false); assert.equal(f.history.state,null); assert.equal(f.covered(),false);
}));

test('focus returns to the correct opener after the history rewind completes', async () => fixture(async f=>{
  f.parent.returnFocus='settings-policy'; f.child.returnFocus='policy-apps';
  f.parent.show(); await f.settle(); f.child.show(); await f.settle();
  f.window.dispatchEvent(new Event('back',{cancelable:true})); await f.settle();
  assert.equal(f.focused(),'policy-apps'); assert.equal(f.parent.open,true);
  f.history.back(); await f.settle();
  assert.equal(f.focused(),'settings-policy'); assert.equal(f.parent.open,false);
}));
