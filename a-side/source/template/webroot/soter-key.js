// SOTER identity key helpers: PEM (PKCS#8/PKCS#1) -> public key + fingerprint,
// and applying one identity into the software TA ledger.
const DIR = '/data/adb/ommega/soterta';
function b64bytes(t) { const b = atob(String(t).replace(/\s+/g, '')); const o = new Uint8Array(b.length); for (let i = 0; i < b.length; i++) o[i] = b.charCodeAt(i); return o; }
function tlv(b, p) {
  const tag = b[p]; let len = b[p + 1]; let at = p + 2;
  if (len & 0x80) { const n = len & 0x7f; len = 0; for (let i = 0; i < n; i++) len = (len << 8) | b[at + i]; at += n; }
  if (at + len > b.length) throw new Error('DER overrun');
  return {tag: tag, body: at, end: at + len};
}
function children(b, node) { const out = []; let at = node.body; while (at < node.end) { const c = tlv(b, at); out.push(c); at = c.end; } return out; }
function encode(tag, body) {
  const len = body.length; let header;
  if (len < 0x80) header = [tag, len]; else if (len < 0x100) header = [tag, 0x81, len];
  else if (len < 0x10000) header = [tag, 0x82, len >> 8, len & 0xff];
  else header = [tag, 0x83, len >> 16, (len >> 8) & 0xff, len & 0xff];
  const out = new Uint8Array(header.length + len); out.set(header, 0); out.set(body, header.length); return out;
}
function concat(parts) { let size = 0; for (const p of parts) size += p.length; const out = new Uint8Array(size); let at = 0; for (const p of parts) { out.set(p, at); at += p.length; } return out; }
const ALG = new Uint8Array([0x30,0x0d,0x06,0x09,0x2a,0x86,0x48,0x86,0xf7,0x0d,0x01,0x01,0x01,0x05,0x00]);
export function parseRsaPrivate(pem) {
  const m = String(pem || '').trim().match(/-----BEGIN (RSA )?PRIVATE KEY-----([\s\S]*?)-----END (RSA )?PRIVATE KEY-----/);
  if (!m) throw new Error('not a PEM private key');
  const der = b64bytes(m[2]); let pkcs1 = der;
  if (!m[1]) {
    const octet = children(der, tlv(der, 0)).find(n => n.tag === 0x04);
    if (!octet) throw new Error('PKCS#8 has no key body');
    pkcs1 = der.slice(octet.body, octet.end);
  }
  const fields = children(pkcs1, tlv(pkcs1, 0));
  if (fields.length < 3) throw new Error('short RSA key');
  const n = pkcs1.slice(fields[1].body, fields[1].end), e = pkcs1.slice(fields[2].body, fields[2].end);
  if (n.length < 128) throw new Error('RSA key too small');
  const pkcs8 = encode(0x30, concat([new Uint8Array([0x02,0x01,0x00]), ALG, encode(0x04, pkcs1)]));
  const rsaPublic = encode(0x30, concat([encode(0x02, n), encode(0x02, e)]));
  const spki = encode(0x30, concat([ALG, encode(0x03, concat([new Uint8Array([0]), rsaPublic]))]));
  return {pkcs8: pkcs8, spki: spki, bits: n.length * 8};
}
export function pemWrap(label, bytes) {
  let bin = ''; for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  const b64 = btoa(bin); const out = []; for (let i = 0; i < b64.length; i += 64) out.push(b64.slice(i, i + 64));
  return '-----BEGIN ' + label + '-----\n' + out.join('\n') + '\n-----END ' + label + '-----\n';
}
export async function describe(pem) {
  const parsed = parseRsaPrivate(pem);
  const privatePem = pemWrap('PRIVATE KEY', parsed.pkcs8);
  const publicPem = pemWrap('PUBLIC KEY', parsed.spki);
  let fingerprint = '';
  const subtle = (typeof crypto !== 'undefined' && crypto.subtle) ? crypto.subtle : null;
  if (subtle) {
    try { const d = await subtle.digest('SHA-256', parsed.spki); fingerprint = [...new Uint8Array(d)].slice(0,8).map(b => b.toString(16).padStart(2,'0')).join(''); } catch (e) { fingerprint = ''; }
    try { await subtle.importKey('pkcs8', parsed.pkcs8, {name:'RSA-PSS', hash:'SHA-256'}, false, ['sign']); } catch (e) { throw new Error('the key is not usable by WebCrypto'); }
  }
  return {privatePem: privatePem, publicPem: publicPem, bits: parsed.bits, fingerprint: fingerprint};
}
function b64utf8(t) { const bytes = new TextEncoder().encode(t); let bin = ''; for (const b of bytes) bin += String.fromCharCode(b); return btoa(bin); }
