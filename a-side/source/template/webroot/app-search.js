// A single owner for the native Apps search field.
//
// Android WebView does not register a text-editor connection for an input that
// was parsed with the document: the field can take DOM focus and even raise the
// keyboard, yet the IME still reports TYPE_NULL, so committed text (Chinese
// candidates) is discarded. A node created later does get a real connection.
//
// Order matters: replacing or focusing the field *during* the user's tap leaves
// the connection missing, so the editor is prepared before the touch and focus
// is always left to the user.
export function appSearchKey(value) {
  return String(value || '').normalize('NFKC').trim().toLowerCase();
}

export function indexAppSearch(list) {
  return [...list.querySelectorAll('.card[data-package]')].map(card => ({
    row:card.closest('.card-box') || card,
    key:appSearchKey((card.dataset.omOriginalName || card.querySelector('.app-name')?.textContent || '') + ' ' + card.getAttribute('data-package')),
  }));
}

export function filterAppSearch(entries, value) {
  const query = appSearchKey(value);
  let matched = 0;
  for (const {row,key} of entries) {
    const show = !query || key.includes(query);
    row.hidden = !show;
    if (show) matched++;
  }
  return {query,matched,total:entries.length};
}

export function createSearchInput(doc, previous) {
  const fresh = doc.createElement('input');
  fresh.id = 'om-app-search';
  fresh.type = 'text';
  fresh.setAttribute('inputmode', 'text');
  fresh.setAttribute('enterkeyhint', 'search');
  fresh.setAttribute('autocomplete', 'off');
  fresh.setAttribute('aria-controls', 'apps-list');
  if (previous && previous.value) fresh.value = previous.value;
  return fresh;
}

const state = new WeakMap();

export function prepareSearchEditor(doc = document) {
  const current = state.get(doc);
  if (!current || current.replaced) return current ? current.input : null;
  const previous = doc.getElementById('om-app-search');
  if (!previous) return null;
  const fresh = createSearchInput(doc, previous);
  previous.replaceWith(fresh);
  current.replaced = true;
  current.input = fresh;
  current.bind(fresh);
  current.render();
  return fresh;
}

export function installAppSearch({document:doc = document, text = (zh,en) => en} = {}) {
  const input = doc.getElementById('om-app-search');
  const clear = doc.getElementById('om-app-search-clear');
  const label = doc.getElementById('om-app-search-label');
  const status = doc.getElementById('om-app-search-status');
  const list = doc.getElementById('apps-list');
  if (!input || !clear || !list || state.has(doc)) return false;
  const prompt = text('搜索应用名称或包名','Search app name or package');
  if (label) label.textContent = prompt;
  clear.setAttribute('aria-label',text('清空搜索','Clear search'));
  const current = { input, replaced:false, bind:null, render:null };
  let entries = indexAppSearch(list), composing = false, bound = new WeakSet();

  const render = () => {
    const field = current.input;
    if (composing || !field) return;
    const result = filterAppSearch(entries,field.value);
    clear.hidden = !field.value;
    if (status) {
      const empty = Boolean(result.query) && result.matched === 0;
      status.textContent = empty ? text('未找到匹配的应用','No matching apps') : '';
      status.hidden = !empty;
    }
  };
  const refresh = () => { entries = indexAppSearch(list); render(); };
  const bind = field => {
    if (bound.has(field)) return;
    bound.add(field);
    field.placeholder = prompt;
    field.addEventListener('compositionstart',() => { composing = true; });
    field.addEventListener('compositionend',() => { composing = false; render(); });
    field.addEventListener('input',event => {
      if (!composing && !event.isComposing) render();
    });
    field.addEventListener('blur',() => { composing = false; render(); });
    // Closing the old overflow menu must not restore its opener over a user who
    // is entering Search. Do not cancel the pointer event or focus anything.
    field.addEventListener('pointerdown',() => {
      const popup = doc.getElementById('menu-options');
      if (!popup?.open) return;
      const previous = popup.skipRestoreFocus;
      popup.skipRestoreFocus = true;
      popup.addEventListener('closed',() => { popup.skipRestoreFocus = previous; },{once:true});
    });
  };
  current.bind = bind;
  current.render = render;
  state.set(doc,current);
  clear.addEventListener('click',() => {
    const field = current.input;
    if (!field) return;
    composing = false;
    field.value = ''; // The only editor write: an explicit user Clear action.
    render();
  });
  doc.addEventListener('ommega-apps-rendered',refresh);
  bind(input);
  render(); // A bootstrap that finished before this module is indexed here.
  return true;
}
