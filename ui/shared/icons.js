// Shared Ommega line icons. The path data is copied verbatim from the
// Material You design prototype so the packaged WebUI and the design
// preview cannot drift. Keep this file dependency-free: it runs inside the
// offline Android WebView.
const paths = {
  "shield": "<path d=\"M12 2.8 19 5.5v5.7c0 4.4-2.6 7.8-7 10-4.4-2.2-7-5.6-7-10V5.5L12 2.8Z\"/><path d=\"m9 12 2 2 4-4\"/>",
  "shield-error": "<path d=\"M12 2.8 19 5.5v5.7c0 4.4-2.6 7.8-7 10-4.4-2.2-7-5.6-7-10V5.5L12 2.8Z\"/><path d=\"m9.5 9.5 5 5m0-5-5 5\"/>",
  "shield-alert": "<path d=\"M12 2.8 19 5.5v5.7c0 4.4-2.6 7.8-7 10-4.4-2.2-7-5.6-7-10V5.5L12 2.8Z\"/><path d=\"M12 8v4.5m0 3h.01\"/>",
  "grid": "<rect x=\"3.5\" y=\"3.5\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"13.5\" y=\"3.5\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"3.5\" y=\"13.5\" width=\"7\" height=\"7\" rx=\"2\"/><rect x=\"13.5\" y=\"13.5\" width=\"7\" height=\"7\" rx=\"2\"/>",
  "apps": "<rect x=\"3\" y=\"3\" width=\"8\" height=\"8\" rx=\"2.5\"/><rect x=\"13\" y=\"3\" width=\"8\" height=\"8\" rx=\"2.5\"/><rect x=\"3\" y=\"13\" width=\"8\" height=\"8\" rx=\"2.5\"/><rect x=\"13\" y=\"13\" width=\"8\" height=\"8\" rx=\"2.5\"/>",
  "settings": "<path d=\"M4 7h16M4 17h16\"/><circle cx=\"9\" cy=\"7\" r=\"2.5\"/><circle cx=\"15\" cy=\"17\" r=\"2.5\"/>",
  "link": "<path d=\"m9.5 14.5 5-5M7 16.8l-1 .9a3.7 3.7 0 0 1-5.2-5.2l3-3a3.7 3.7 0 0 1 5.2 0M17 7.2l1-.9a3.7 3.7 0 1 1 5.2 5.2l-3 3a3.7 3.7 0 0 1-5.2 0\" transform=\"translate(0 .2) scale(.92)\"/>",
  "chevron": "<path d=\"m9 5 7 7-7 7\"/>",
  "back": "<path d=\"M20 12H4m7-7-7 7 7 7\"/>",
  "more": "<circle cx=\"5\" cy=\"12\" r=\"1\"/><circle cx=\"12\" cy=\"12\" r=\"1\"/><circle cx=\"19\" cy=\"12\" r=\"1\"/>",
  "arrow": "<path d=\"M4 12h15m-5-5 5 5-5 5\"/>",
  "refresh": "<path d=\"M19.5 8A8 8 0 0 0 5.3 6.5L3.5 9M4.5 16A8 8 0 0 0 18.7 17.5L20.5 15M3.5 4.5V9H8M20.5 19.5V15H16\"/>",
  "moon": "<path d=\"M20.3 14.5A8.7 8.7 0 0 1 9.5 3.7a8.7 8.7 0 1 0 10.8 10.8Z\"/>",
  "sun": "<circle cx=\"12\" cy=\"12\" r=\"4\"/><path d=\"M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.4 1.4m11.2 11.2L19 19M19 5l-1.4 1.4M6.4 17.6 5 19\"/>",
  "cpu": "<rect x=\"6\" y=\"6\" width=\"12\" height=\"12\" rx=\"3\"/><path d=\"M9 9h6v6H9zM9 2v4m6-4v4M9 18v4m6-4v4M2 9h4m-4 6h4m12-6h4m-4 6h4\"/>",
  "key": "<circle cx=\"8\" cy=\"12\" r=\"4\"/><path d=\"M12 12h9m-3 0v3m-3-3v2\"/>",
  "clock": "<circle cx=\"12\" cy=\"12\" r=\"8.5\"/><path d=\"M12 7v5l3 2\"/>",
  "server": "<rect x=\"4\" y=\"3.5\" width=\"16\" height=\"7\" rx=\"2\"/><rect x=\"4\" y=\"13.5\" width=\"16\" height=\"7\" rx=\"2\"/><path d=\"M7.5 7h.1m-.1 10h.1M12 7h5m-5 10h5\"/>",
  "phone": "<rect x=\"6\" y=\"2.5\" width=\"12\" height=\"19\" rx=\"3\"/><path d=\"M10 5h4m-3 13h2\"/>",
  "battery": "<rect x=\"3\" y=\"7\" width=\"16\" height=\"10\" rx=\"2.5\"/><path d=\"M21 10v4\"/>",
  "check": "<path d=\"m5 12 4.5 4.5L19 7\"/>",
  "search": "<circle cx=\"10.5\" cy=\"10.5\" r=\"6.5\"/><path d=\"m16 16 4.5 4.5\"/>",
  "eye": "<path d=\"M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6Z\"/><circle cx=\"12\" cy=\"12\" r=\"2.6\"/>",
  "palette": "<path d=\"M12 3a9 9 0 1 0 0 18h1.4a2 2 0 0 0 1.5-3.3 2 2 0 0 1 1.5-3.2H19A2 2 0 0 0 21 12a9 9 0 0 0-9-9Z\"/><path d=\"M7.5 10h.01M10 7h.01M14 7h.01M17 10h.01\"/>",
  "globe": "<circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18\"/>",
  "sliders": "<path d=\"M5 4v16m7-16v16m7-16v16\"/><circle cx=\"5\" cy=\"9\" r=\"2\"/><circle cx=\"12\" cy=\"15\" r=\"2\"/><circle cx=\"19\" cy=\"8\" r=\"2\"/>",
  "lock": "<rect x=\"4.5\" y=\"10\" width=\"15\" height=\"11\" rx=\"3\"/><path d=\"M8 10V7a4 4 0 0 1 8 0v3m-4 4v3\"/>",
  "file": "<path d=\"M6 3.5h8l4 4V20a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4.5a1 1 0 0 1 1-1Z\"/><path d=\"M14 3.5V8h4M8 12h8m-8 4h8\"/>",
  "info": "<circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M12 10.5v6m0-9h.01\"/>",
  "close": "<path d=\"m6 6 12 12M18 6 6 18\"/>",
  "help": "<circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M9.5 9a2.5 2.5 0 1 1 4 2c-1 .7-1.5 1.1-1.5 2.3M12 17h.01\"/>",
};

// The packaged pages use a few shorter names for the same glyphs.
const aliases = {overview: 'grid'};

export const iconNames = Object.freeze(Object.keys(paths));

export function icon(name, {size = 20, className = 'icon'} = {}) {
  const body = paths[aliases[name] || name] || paths.shield;
  return `<svg class="${className}" viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;
}
