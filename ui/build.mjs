// Keep the shared offline resources in each independently packaged module.
import {readFile, writeFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
const root = new URL('../', import.meta.url);
const check = process.argv.includes('--check');
for (const side of ['a', 'b']) for (const name of ['ui-kit.js', 'modern.css']) {
  const source = await readFile(new URL(`ui/shared/${name}`, root));
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
const integration = `<!--ommega-modern--><script>try{var l=localStorage.getItem('ommega.ui.language')||'auto';localStorage.setItem('trickyAddonLanguage',l==='auto'?(/^zh\\b/i.test(navigator.language)?'zh-CN':'en'):l);document.documentElement.dataset.theme=localStorage.getItem('ommega.ui.theme')||'auto'}catch(e){}</script><link rel="stylesheet" href="./modern.css"><link rel="stylesheet" href="./a-modern.css"><script type="module" src="./modern.js"></script><!--/ommega-modern-->`;
const clean = original.replace(/<!--ommega-modern-->[\s\S]*?<!--\/ommega-modern-->/g, '');
const integrated = clean.replace('</head>', integration + '</head>');
if (check && original !== integrated) throw new Error('A WebUI shell is stale. Run node ui/build.mjs');
if (!check) await writeFile(indexPath, integrated);
