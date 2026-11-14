// Keep the shared offline resources in each independently packaged module.
import {createHash} from 'node:crypto';
import {readFile, writeFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
const root = new URL('../', import.meta.url);
const args = process.argv.slice(2);
if (args.some(arg => arg !== '--check' && !/^--side=(a|b|all)$/.test(arg))) throw new Error('Usage: node ui/build.mjs [--check] [--side=a|b|all]');
const check = args.includes('--check');
const selectedSide = args.find(arg => arg.startsWith('--side='))?.slice(7) || 'all';
const sides = selectedSide === 'all' ? ['a', 'b'] : [selectedSide];
for (const side of sides) for (const name of ['ui-kit.js', 'modern.css', 'material-you.css', 'icons.js']) {
  const source = Buffer.from((await readFile(new URL(`ui/shared/${name}`, root), 'utf8')).replaceAll('\r\n', '\n'));
  const destination = new URL(`${side}-side/source/template/webroot/${name}`, root);
  if (check) {
    const built = await readFile(destination).catch(() => Buffer.alloc(0));
    if (!source.equals(built)) throw new Error(`Shared UI resource is stale: ${fileURLToPath(destination)}. Run node ui/build.mjs`);
  } else await writeFile(destination, source);
}
console.log(`Shared UI assets ${check ? 'verified' : 'copied'}: ${sides.join(', ')}.`);
if (!sides.includes('a')) process.exit(0);
const indexPath = new URL('a-side/source/template/webroot/index.html', root);
const original = await readFile(indexPath, 'utf8');
// Set the legacy translation preference before its deferred bundle starts.
// Asset versions come from file content so a WebView cache refreshes on every
// build, matching the values scripts/webui-patch.py writes into the bundle tag.
const versionOf = async relative => {
  try {
    return createHash('sha256').update(await readFile(new URL(relative, root))).digest('hex').slice(0, 10);
  } catch (cause) {
    throw new Error(`Missing local WebUI resource: ${relative}`, {cause});
  }
};
const assetVersion = await versionOf('a-side/source/template/webroot/modern.js');
const cssVersions = {
  'modern.css': await versionOf('a-side/source/template/webroot/modern.css'),
  'material-you.css': await versionOf('a-side/source/template/webroot/material-you.css'),
  'a-modern.css': await versionOf('a-side/source/template/webroot/a-modern.css')
};
const integration = `<!--ommega-modern--><script>try{var l=localStorage.getItem('ommega.ui.language')||'auto';localStorage.setItem('trickyAddonLanguage',l==='auto'?(/^zh\\b/i.test(navigator.language)?'zh-CN':'en'):l);document.documentElement.dataset.theme=localStorage.getItem('ommega.ui.theme')||'auto'}catch(e){}</script><link rel="stylesheet" href="./modern.css?v=${cssVersions['modern.css']}"><link rel="stylesheet" href="./material-you.css?v=${cssVersions['material-you.css']}"><link rel="stylesheet" href="./a-modern.css?v=${cssVersions['a-modern.css']}"><script type="module" src="./modern.js?v=${assetVersion}"></script><!--/ommega-modern-->`;
const bundleName = original.match(/src=["']\.\/assets\/(index-[^"'?]+\.js)(?:\?[^"']*)?["']/)?.[1];
if (!bundleName) throw new Error('Missing A shipped vendor bundle');
const bundleHash = await versionOf('a-side/source/template/webroot/assets/' + bundleName);
let clean = original.replace(/<!--ommega-modern-->[\s\S]*?<!--\/ommega-modern-->/g, '')
  .replace(/(src="\.\/assets\/index-[^"]+\.js)(?:\?[^\"]*)?"/, `$1?v=${bundleHash}"`);
// Keep the original dialog bindings available to the Settings view. The Apps
// overflow menu contains only app-selection actions, including its refresh.
const settingsActions = [];
for (const id of ['automation-settings', 'boot-hash', 'boot-key', 'remote-config', 'help', 'about']) {
  const pattern = new RegExp(`<md-menu-item\\b[^>]*\\bid="${id}"[^>]*>[\\s\\S]*?<\\/md-menu-item>`);
  const item = clean.match(pattern)?.[0];
  if (!item) throw new Error(`Missing A Settings action: ${id}`);
  settingsActions.push(item);
  clean = clean.replace(pattern, '');
}
const languagePattern = /<md-sub-menu\b[^>]*>[\s\S]*?<md-menu\b[^>]*\bid="language-menu"[^>]*>[\s\S]*?<\/md-menu><\/md-sub-menu>/;
const languageMenu = clean.match(languagePattern)?.[0];
if (!languageMenu) throw new Error('Missing A legacy language binding');
settingsActions.push(languageMenu);
clean = clean.replace(languagePattern, '')
  .replace(/<!--ommega-legacy-settings-->[\s\S]*?<!--\/ommega-legacy-settings-->/g, '')
  .replace(/(<md-menu\b[^>]*\bid="menu-options"[^>]*>)([\s\S]*?)(<\/md-menu>)/,
    (_, open, items, close) => open + items.replace(/<md-divider\b[^>]*><\/md-divider>/g, '') + close);
const legacySettings = `<!--ommega-legacy-settings--><div id="om-legacy-settings" hidden>${settingsActions.join('')}</div><!--/ommega-legacy-settings-->`;
// The modern editor is static native HTML, outside the vendor header and its
// selection/focus/transition rules. Retain disabled vendor nodes solely because
// the opaque bundle still resolves their ids at module initialization.
const search = `<!--ommega-app-search--><section id="om-app-search-region" role="search" hidden><div class="om-app-search-control"><label id="om-app-search-label" for="om-app-search" class="om-search-label">Search app name or package</label><input id="om-app-search" type="text" inputmode="text" enterkeyhint="search" aria-controls="apps-list"><button id="om-app-search-clear" type="button" aria-label="Clear search" hidden><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg></button></div><p id="om-app-search-status" role="status" aria-live="polite" aria-atomic="true" hidden></p></section><!--/ommega-app-search-->`;
clean = clean.replace(/<!--ommega-app-search-->[\s\S]*?<!--\/ommega-app-search-->/g, '')
  .replace(/<div\b[^>]*class="header header-bg"[^>]*>/, '<div class="header header-bg" hidden inert aria-hidden="true">')
  .replace(/<input\b[^>]*id="search"[^>]*>/, '<input id="search" type="text" disabled tabindex="-1" aria-hidden="true">');
let integrated = clean.replace('</head>', integration + '</head>')
  .replace(/<body\b[^>]*>/, match => match + search)
  .replace('</body>', legacySettings + '</body>');
// Hash every real local shell resource, including vendor CSS and the bundle.
// --check compares only; neither missing files nor stale queries are accepted.
for (const match of [...integrated.matchAll(/\b(?:src|href)=["'](\.\/[^"'?]+\.(?:js|css))(?:\?[^"']*)?["']/g)]) {
  if (match[1].split('/').includes('..')) throw new Error(`Invalid local WebUI resource: ${match[1]}`);
  const digest = await versionOf('a-side/source/template/webroot/' + match[1].slice(2));
  integrated = integrated.replaceAll(match[0], match[0].replace(/(["']).*$/, `$1${match[1]}?v=${digest}$1`));
}
if (check && original !== integrated) throw new Error('A WebUI shell is stale. Run node ui/build.mjs --side=a');
if (!check) await writeFile(indexPath, integrated);
