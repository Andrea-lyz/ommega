// Keep the shared offline resources in each independently packaged module.
import {readFile, writeFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
const root = new URL('../', import.meta.url);
const check = process.argv.includes('--check');
for (const side of ['a', 'b']) for (const name of ['ui-kit.js', 'modern.css']) {
  const source = Buffer.from((await readFile(new URL(`ui/shared/${name}`, root), 'utf8')).replaceAll('\r\n', '\n'));
  const destination = new URL(`${side}-side/source/template/webroot/${name}`, root);
  if (check) {
    const built = await readFile(destination).catch(() => Buffer.alloc(0));
    if (!source.equals(built)) throw new Error(`Shared UI resource is stale: ${fileURLToPath(destination)}. Run node ui/build.mjs`);
  } else await writeFile(destination, source);
}
console.log(check ? 'Shared UI assets verified.' : 'Shared UI assets copied to A and B.');
const indexPath = new URL('a-side/source/template/webroot/index.html', root);
const original = await readFile(indexPath, 'utf8');
// Set the legacy translation preference before its deferred bundle starts.
const integration = `<!--ommega-modern--><script>try{var l=localStorage.getItem('ommega.ui.language')||'auto';localStorage.setItem('trickyAddonLanguage',l==='auto'?(/^zh\\b/i.test(navigator.language)?'zh-CN':'en'):l);document.documentElement.dataset.theme=localStorage.getItem('ommega.ui.theme')||'auto'}catch(e){}</script><link rel="stylesheet" href="./modern.css?v=cpu-exp6-1"><link rel="stylesheet" href="./a-modern.css?v=cpu-exp6-1"><script type="module" src="./modern.js?v=cpu-exp6-1"></script><!--/ommega-modern-->`;
let clean = original.replace(/<!--ommega-modern-->[\s\S]*?<!--\/ommega-modern-->/g, '')
  .replace(/(src="\.\/assets\/index-[^"]+\.js)(?:\?[^\"]*)?"/, '$1?v=cpu-exp6-1"');
// Keep the original dialog bindings available to the Settings view. The Apps
// overflow menu contains only app-selection actions, including its refresh.
const settingsActions = [];
for (const id of ['automation-settings', 'boot-hash', 'boot-key', 'security-patch', 'remote-config', 'help', 'about']) {
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
const integrated = clean.replace('</head>', integration + '</head>')
  .replace('</body>', legacySettings + '</body>');
if (check && original !== integrated) throw new Error('A WebUI shell is stale. Run node ui/build.mjs');
if (!check) await writeFile(indexPath, integrated);
