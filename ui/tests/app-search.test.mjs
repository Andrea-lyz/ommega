import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {appSearchKey,indexAppSearch,filterAppSearch,createSearchInput,prepareSearchEditor,installAppSearch} from '../../a-side/source/template/webroot/app-search.js';

function fixture() {
  const writes=[], listeners=[], focusCalls=[];
  let nativeValue='';
  class Control extends EventTarget {
    constructor(id) { super();this.id=id;this.hidden=false;this.textContent='';this.attributes={};this.value=''; }
    addEventListener(type,fn,options) {listeners.push([this.id,type]);super.addEventListener(type,fn,options);}
    setAttribute(name,value) {this.attributes[name]=value;}
    focus() {focusCalls.push(this.id);this.owner.active=this;this.owner.doc.activeElement=this;}
    blur() {if(this.owner.active===this){this.owner.active=null;this.owner.doc.activeElement=null;}}
    replaceWith(node) {this.owner.nodes[this.id]=node;node.owner=this.owner;if(this.owner.active===this){this.owner.active=null;this.owner.doc.activeElement=null;}}
  }
  const row=(name,pkg)=>{
    const container={hidden:false};
    return {dataset:{omOriginalName:name},getAttribute:()=>pkg,closest:()=>container,
      querySelector:()=>({textContent:name+' certificate 999'}),container};
  };
  let cards=[row('中文钱包 Wallet','com.example.wallet'),row('聊天 Chat','com.example.chat')];
  const list={querySelectorAll:()=>cards};
  const doc=new EventTarget();
  const owner={nodes:Object.create(null),active:null};
  const input=new Control('om-app-search'), clear=new Control('om-app-search-clear');
  Object.defineProperty(input,'value',{get:()=>nativeValue,set:value=>{writes.push(value);nativeValue=value;}});
  const label=new Control('om-app-search-label'), status=new Control('om-app-search-status');
  const region=new Control('om-app-search-region');
  const popup=new Control('menu-options');popup.open=false;popup.skipRestoreFocus=false;
  for (const node of [input,clear,label,status,region,popup]) {node.owner=owner;owner.nodes[node.id]=node;}
  owner.nodes['apps-list']=list;owner.doc=doc;
  doc.getElementById=id=>owner.nodes[id]||null;
  doc.createElement=tag=>{const node=new Control('');node.localName=tag;node.owner=owner;return node;};
  doc.activeElement=null;
  const fire=(target,type,extra={})=>{
    const event=new Event(type,{cancelable:true,bubbles:true});
    for (const [key,value] of Object.entries(extra)) Object.defineProperty(event,key,{configurable:true,value});
    target.dispatchEvent(event);return event;
  };
  const install=()=>installAppSearch({document:doc,text:(zh,en)=>zh});
  const search=()=>owner.nodes['om-app-search'];
  return {doc,input,clear,label,status,region,popup,list,writes,listeners,focusCalls,fire,install,search,
    native:value=>{nativeValue=value;},cards:()=>cards,replace:next=>{cards=next;},row};
}

test('search normalizes names and package ids without matching policy/certificate badges',()=>{
  const f=fixture(), entries=indexAppSearch(f.list);
  assert.equal(appSearchKey(' ＷＡＬＬＥＴ '),'wallet');
  assert.deepEqual(filterAppSearch(entries,' 中文 '),{query:'中文',matched:1,total:2});
  assert.equal(f.cards()[0].container.hidden,false);assert.equal(f.cards()[1].container.hidden,true);
  assert.equal(filterAppSearch(entries,'COM.EXAMPLE.CHAT').matched,1);
  assert.equal(f.cards()[0].container.hidden,true);assert.equal(f.cards()[1].container.hidden,false);
  assert.equal(filterAppSearch(entries,'999').matched,0,'badges are not searchable app identity');
  assert.equal(filterAppSearch(entries,'').matched,2);
});

test('a replacement editor is a real native text input with search semantics',()=>{
  const f=fixture();
  const fresh=createSearchInput(f.doc,f.input);
  assert.equal(fresh.localName,'input');
  assert.equal(fresh.id,'om-app-search');
  assert.equal(fresh.type,'text');
  assert.equal(fresh.attributes.inputmode,'text');
  assert.equal(fresh.attributes.enterkeyhint,'search');
  assert.equal(fresh.attributes.autocomplete,'off');
  assert.equal(fresh.value,'');
});

test('one owner installs once and binds without focusing or writing',()=>{
  const f=fixture();f.native('wallet');
  assert.equal(f.install(),true);assert.equal(f.install(),false);
  assert.equal(f.listeners.filter(([id,type])=>id==='om-app-search'&&type==='input').length,1);
  assert.match(f.label.textContent,/搜索/);
  assert.equal(f.clear.attributes['aria-label'],'清空搜索');
  f.doc.dispatchEvent(new Event('ommega-apps-rendered'));
  assert.equal(f.input.value,'wallet');assert.deepEqual(f.writes,[]);
  assert.deepEqual(f.focusCalls,[],'installing alone must not request the keyboard');
  assert.equal(f.cards()[1].container.hidden,true);
});

test('opening Apps pre-creates a fresh editor without taking focus',()=>{
  const f=fixture();f.install();
  const before=f.search();
  const prepared=prepareSearchEditor(f.doc);
  assert.notEqual(prepared,before,'the parsed input is replaced before the user taps');
  assert.equal(f.search(),prepared);
  assert.equal(f.doc.activeElement,null,'preparing must not move focus or request the keyboard');
  assert.deepEqual(f.focusCalls,[]);
  assert.equal(prepared.attributes.inputmode,'text');
  assert.equal(prepareSearchEditor(f.doc),prepared,'only one replacement per page');
  prepared.value='中文';f.fire(prepared,'input',{isComposing:false});
  assert.equal(f.cards()[0].container.hidden,false);assert.equal(f.cards()[1].container.hidden,true);
  assert.deepEqual(f.writes,[]);
});

test('composition survives a list replacement and filters only on native commit',()=>{
  const f=fixture();f.install();
  f.fire(f.input,'compositionstart');f.native('zhongwen');
  const preedit=f.fire(f.input,'input',{isComposing:true,inputType:'insertCompositionText'});
  assert.equal(preedit.defaultPrevented,false);
  assert.ok(f.cards().every(card=>!card.container.hidden));
  const fresh=[f.row('中文测试','com.example.test'),f.row('Other','com.example.other')];
  f.replace(fresh);f.doc.dispatchEvent(new Event('ommega-apps-rendered'));
  assert.ok(fresh.every(card=>!card.container.hidden));
  assert.equal(f.input.value,'zhongwen');
  f.native('中文');f.fire(f.input,'compositionend',{data:'中文'});
  assert.equal(fresh[0].container.hidden,false);assert.equal(fresh[1].container.hidden,true);
  f.fire(f.input,'input',{isComposing:false,inputType:'insertText'});
  assert.equal(f.input.value,'中文');assert.deepEqual(f.writes,[]);
});

test('ordinary text, paste, deletion and composition cancellation retain native editing',()=>{
  const f=fixture();f.install();
  f.native('com.example.chat');f.fire(f.input,'input',{inputType:'insertFromPaste'});
  assert.equal(f.cards()[1].container.hidden,false);assert.equal(f.cards()[0].container.hidden,true);
  f.native('');f.fire(f.input,'input',{inputType:'deleteContentBackward'});
  assert.ok(f.cards().every(card=>!card.container.hidden));
  f.fire(f.input,'compositionstart');f.native('discard');f.fire(f.input,'blur');
  f.native('Wallet');f.fire(f.input,'input',{isComposing:false});
  assert.equal(f.cards()[0].container.hidden,false);
  assert.equal(f.fire(f.input,'keydown',{key:'Enter',isComposing:false}).defaultPrevented,false);
  assert.deepEqual(f.writes,[]);
});

test('only explicit Clear writes the editor, restores all rows and leaves focus alone',()=>{
  const f=fixture();f.native('absent');f.install();
  assert.equal(f.status.hidden,false);assert.match(f.status.textContent,/未找到/);
  f.fire(f.clear,'click');
  assert.equal(f.input.value,'');assert.deepEqual(f.writes,['']);
  assert.ok(f.cards().every(card=>!card.container.hidden));
  assert.equal(f.clear.hidden,true);assert.equal(f.status.hidden,true);
});

test('menu dismissal cannot restore its opener over Search',()=>{
  const f=fixture();f.install();f.popup.open=true;
  const pointer=f.fire(f.input,'pointerdown',{isTrusted:true});
  assert.equal(pointer.defaultPrevented,false);assert.equal(f.popup.skipRestoreFocus,true);
  f.popup.open=false;f.fire(f.popup,'closed');
  assert.equal(f.popup.skipRestoreFocus,false);
  f.popup.open=true;f.popup.skipRestoreFocus=true;f.fire(f.region,'pointerdown',{isTrusted:true});
  f.fire(f.popup,'closed');assert.equal(f.popup.skipRestoreFocus,true,'preserve an existing menu policy');
});

test('the shipped search module cannot hook the vendor editor or perform device operations',()=>{
  const source=readFileSync(new URL('../../a-side/source/template/webroot/app-search.js',import.meta.url),'utf8');
  assert.doesNotMatch(source,/\b(?:ksu|eval|setTimeout|requestAnimationFrame|scrollTo|preventDefault|stopImmediatePropagation)\s*[.(]/);
  assert.doesNotMatch(source,/\.blur\s*\(|getElementById\(['"]search['"]\)/);
  assert.equal((source.match(/field\.value\s*=/g)||[]).length,1,'only Clear may write');
});
