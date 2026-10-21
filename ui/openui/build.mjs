import {build} from 'esbuild';
import {mkdir,writeFile} from 'node:fs/promises';
await mkdir(new URL('../openui-dist/',import.meta.url),{recursive:true});
await build({entryPoints:[new URL('./main.jsx',import.meta.url).pathname.replace(/^\/([A-Za-z]:)/,'$1')],outdir:new URL('../openui-dist/',import.meta.url).pathname.replace(/^\/([A-Za-z]:)/,'$1'),bundle:true,minify:true,format:'esm',loader:{'.woff2':'file','.woff':'file','.ttf':'file'},define:{'process.env.NODE_ENV':'"production"'}});
await writeFile(new URL('../openui-dist/index.html',import.meta.url),'<html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Ommega · OpenUI design preview</title><link rel="stylesheet" href="./main.css"><body><div id="root"></div><script type="module" src="./main.js"></script></body></html>');
console.log('OpenUI design preview built. It is not packaged in either device module.');
