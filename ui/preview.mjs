// Host-only preview. All root operations are replaced by inert fixtures.
import http from 'node:http';
import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
const root = new URL('../',import.meta.url);
const config = 'OMMEGA_RELAY_SERVER=https://relay.example.com:8443\nOMMEGA_RELAY_DEVICE_ID=dedicated-b\nOMMEGA_RELAY_TOKEN=preview-placeholder\n';
const fixture = {raw:Buffer.from(config).toString('base64'),digest:createHash('sha256').update(config).digest('hex')};
const mock = `window.__OM_FIXTURE__=${JSON.stringify(fixture)};\n` + await readFile(new URL('./preview-bridge.js',import.meta.url),'utf8');
const mime = {html:'text/html; charset=utf-8',js:'text/javascript; charset=utf-8',css:'text/css; charset=utf-8',json:'application/json',xml:'application/xml',png:'image/png',woff2:'font/woff2'};
const server = http.createServer(async(req,res) => {
  try {
    if (req.method !== 'GET') {res.writeHead(405).end();return;}
    const url = new URL(req.url,'http://localhost');
    if (url.pathname === '/') { res.writeHead(302,{Location:'/a/','Cache-Control':'no-store'}).end(); return; }
    if (url.pathname === '/phone') {
      const width = Math.min(1024,Math.max(320,Number(url.searchParams.get('width')) || 393));
      const height = Math.min(1200,Math.max(400,Number(url.searchParams.get('height')) || 640));
      const html = '<!doctype html><meta charset="utf-8"><title>Ommega mobile UI preview</title><style>body{margin:0;padding:12px;background:#dedee5;display:grid;justify-content:center}iframe{border:1px solid #777;border-radius:12px;width:'+width+'px;height:'+height+'px}</style><iframe title="Ommega mobile preview" src="/a/#om-settings"></iframe>';
      res.writeHead(200,{'Content-Type':mime.html}).end(html); return;
    }
    if (url.pathname === '/preview-bridge.js') { res.writeHead(200,{'Content-Type':mime.js,'Cache-Control':'no-store'}).end(mock);return; }
    const match = /^\/(a|b|openui)\/(.*)$/.exec(decodeURIComponent(url.pathname));
    if (!match || match[2].split('/').some(part=>part==='..') || match[2].includes('\\')) {res.writeHead(404).end();return;}
    const rel = match[2] || 'index.html';
    const path = new URL(match[1] === 'openui' ? `ui/openui-dist/${rel}` : `${match[1]}-side/source/template/webroot/${rel}`,root);
    let body = await readFile(path);
    if (rel.endsWith('.html') && match[1] !== 'openui') body = Buffer.from(body.toString().replace('<head>','<head><script src="/preview-bridge.js"></script>'));
    res.writeHead(200,{'Content-Type':mime[rel.split('.').at(-1)] || 'application/octet-stream','Cache-Control':'no-store'}).end(body);
  } catch {res.writeHead(404).end('Not found');}
});
const port = Number(process.env.PORT || 4179);
server.listen(port,'127.0.0.1',()=>console.log(`UI previews: http://127.0.0.1:${port}/a/ · /b/ · /openui/ (mock data only)`));
