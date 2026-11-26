// Liquid glass for the phone navigation dock.
// The dock refracts the page behind it through an SVG displacement filter
// (backdrop-filter: url(), Chromium/Android WebView only); other engines keep
// the plain frosted dock. The selection pill follows the finger and the view
// changes as soon as the finger is lifted, without waiting for a click.

const FILTER_ID = 'om-liquid-glass';
// Tuned on the reference dock: refraction, bezel width, chromatic spread,
// backdrop blur and saturation.
export const LIQUID_GLASS_PARAMS = Object.freeze({scale: 70, bezel: 18, chroma: 6, blur: 4, saturate: 1.5});

// Signed-distance displacement map of a rounded rectangle. The centre stays
// neutral (128); inside the bezel R/G point inwards with a convex falloff.
export function liquidGlassMap(width, height, radius, bezel) {
  const data = new Uint8ClampedArray(width * height * 4);
  const hw = width / 2, hh = height / 2, r = Math.min(radius, hw, hh);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const px = x + .5 - hw, py = y + .5 - hh;
      const qx = Math.abs(px) - (hw - r), qy = Math.abs(py) - (hh - r);
      let distance, nx = 0, ny = 0;
      if (qx > 0 && qy > 0) {
        const length = Math.hypot(qx, qy);
        distance = length - r; nx = qx / length * Math.sign(px); ny = qy / length * Math.sign(py);
      } else if (qx > qy) { distance = qx - r; nx = Math.sign(px); }
      else { distance = qy - r; ny = Math.sign(py); }
      const inside = -distance;
      const strength = inside > 0 && inside < bezel ? (1 - inside / bezel) ** 2 : 0;
      const i = (y * width + x) * 4;
      data[i] = 128 - 127 * nx * strength;
      data[i + 1] = 128 - 127 * ny * strength;
      data[i + 2] = 128;
      data[i + 3] = 255;
    }
  }
  return data;
}

// Index of the tab whose centre is closest to a finger at `offset`.
export function liquidGlassIndex(offset, slots) {
  let best = 0;
  slots.forEach((slot, i) => {
    if (Math.abs(offset - (slot.x + slot.w / 2)) < Math.abs(offset - (slots[best].x + slots[best].w / 2))) best = i;
  });
  return best;
}

function supportsRefraction() {
  const ua = navigator.userAgent || '';
  if (!/Chrome\/\d+/.test(ua) || /Firefox|Edg\//.test(ua)) return false;
  return !!globalThis.CSS?.supports?.('backdrop-filter', 'url(#a)');
}

function ensureFilter() {
  let svg = document.getElementById(FILTER_ID + '-defs');
  if (svg) return svg;
  const {scale, chroma, blur, saturate} = LIQUID_GLASS_PARAMS;
  svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.id = FILTER_ID + '-defs';
  svg.setAttribute('width', '0'); svg.setAttribute('height', '0'); svg.setAttribute('aria-hidden', 'true');
  svg.style.cssText = 'position:absolute;width:0;height:0;overflow:hidden';
  const channel = (row, id, amount) =>
    `<feDisplacementMap in="src" in2="map" scale="${amount}" xChannelSelector="R" yChannelSelector="G" result="d${id}"/>` +
    `<feColorMatrix in="d${id}" type="matrix" values="${row}" result="${id}"/>`;
  svg.innerHTML = `<filter id="${FILTER_ID}" x="0" y="0" width="100%" height="100%" color-interpolation-filters="sRGB">` +
    '<feImage id="' + FILTER_ID + '-map" x="0" y="0" width="1" height="1" preserveAspectRatio="none" result="map"/>' +
    `<feGaussianBlur in="SourceGraphic" stdDeviation="${blur}" result="src"/>` +
    channel('1 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0', 'r', scale + chroma) +
    channel('0 0 0 0 0  0 1 0 0 0  0 0 0 0 0  0 0 0 1 0', 'g', scale) +
    channel('0 0 0 0 0  0 0 0 0 0  0 0 1 0 0  0 0 0 1 0', 'b', Math.max(0, scale - chroma)) +
    '<feBlend in="r" in2="g" mode="screen" result="rg"/><feBlend in="rg" in2="b" mode="screen" result="rgb"/>' +
    `<feColorMatrix in="rgb" type="saturate" values="${saturate}"/></filter>`;
  document.body.append(svg);
  return svg;
}

export function installLiquidGlass(nav, {onSelect = () => {}} = {}) {
  if (!nav || nav.dataset.omLiquid) return;
  nav.dataset.omLiquid = 'true';
  const buttons = [...nav.querySelectorAll('button[data-om-view]')];
  const pill = document.createElement('span');
  pill.className = 'om-nav-pill';
  pill.setAttribute('aria-hidden', 'true');
  nav.prepend(pill);
  nav.classList.add('om-liquid');
  const phone = matchMedia('(max-width: 760px)');
  const refraction = supportsRefraction();
  if (refraction) nav.classList.add('om-liquid-refract');

  let active = Math.max(0, Number(nav.dataset.active) || 0);
  // Slots follow the real button boxes (the dock has gaps between buttons),
  // in the pill's own coordinates: it starts at the nav's left padding.
  let origin = 0, slots = buttons.map(() => ({x: 0, w: 0}));
  const measure = () => {
    const pad = parseFloat(getComputedStyle(nav).paddingLeft) || 0;
    origin = nav.getBoundingClientRect().left + (nav.clientLeft || 0) + pad;
    slots = buttons.map(button => ({x: button.offsetLeft - pad, w: button.offsetWidth}));
  };
  const place = (x, width, dragging) => {
    pill.classList.toggle('dragging', dragging);
    pill.style.setProperty('--om-pill-x', x + 'px');
    pill.style.setProperty('--om-pill-w', width + 'px');
  };
  // Placing on a route change reuses the measured slots: reading layout there
  // would force a full-page reflow of the incoming view before it paints.
  let placed = false;
  const placeIndex = index => {
    if (!(slots[index]?.w > 0)) measure();
    const slot = slots[index] || slots[0];
    if (!placed) pill.style.transition = 'none';
    place(slot.x, slot.w, false);
    if (!placed) { placed = true; requestAnimationFrame(() => requestAnimationFrame(() => { pill.style.transition = ''; })); }
  };
  const highlight = index => {
    nav.classList.toggle('om-dragging', index >= 0);
    buttons.forEach((button, i) => button.classList.toggle('om-nav-hover', i === index));
  };

  // The route sets data-active; keep the pill on it unless a finger holds it.
  let pointer = null;
  new MutationObserver(() => {
    active = Math.max(0, Number(nav.dataset.active) || 0);
    if (!pointer) placeIndex(active);
  }).observe(nav, {attributes: true, attributeFilter: ['data-active']});

  let mapFrame = 0;
  const refreshMap = () => {
    mapFrame = 0;
    measure(); if (!pointer) placeIndex(active);
    if (!refraction || !phone.matches) return;
    const width = Math.round(nav.offsetWidth), height = Math.round(nav.offsetHeight);
    if (width < 2 || height < 2) return;
    const radius = Math.min(height / 2, parseFloat(getComputedStyle(nav).borderTopLeftRadius) || height / 2);
    const canvas = document.createElement('canvas');
    canvas.width = width; canvas.height = height;
    canvas.getContext('2d').putImageData(new ImageData(liquidGlassMap(width, height, radius, LIQUID_GLASS_PARAMS.bezel), width, height), 0, 0);
    ensureFilter();
    const map = document.getElementById(FILTER_ID + '-map');
    map.setAttribute('href', canvas.toDataURL());
    map.setAttribute('width', width); map.setAttribute('height', height);
    // Some WebView builds keep the old filter output until the property changes.
    nav.style.backdropFilter = 'none';
    requestAnimationFrame(() => { nav.style.backdropFilter = ''; });
  };
  const scheduleMap = () => { if (!mapFrame) mapFrame = requestAnimationFrame(refreshMap); };
  new ResizeObserver(scheduleMap).observe(nav);
  phone.addEventListener?.('change', scheduleMap);
  // Late styles or fonts can move the buttons without resizing the nav.
  const remeasure = () => requestAnimationFrame(() => { measure(); if (!pointer) placeIndex(active); });
  document.fonts?.ready.then(remeasure).catch(() => {});
  if (document.readyState !== 'complete') window.addEventListener('load', remeasure, {once: true});
  refreshMap();

  const follow = clientX => {
    const offset = clientX - origin, width = slots[0].w, last = slots[slots.length - 1];
    place(Math.max(slots[0].x, Math.min(offset - width / 2, last.x)), width, true);
    highlight(liquidGlassIndex(offset, slots));
  };
  let suppressClick = 0;
  nav.addEventListener('pointerdown', event => {
    if (!phone.matches || pointer !== null || event.button > 0) return;
    pointer = event.pointerId;
    try { nav.setPointerCapture(pointer); } catch { /* released before capture */ }
    measure();
    pill.classList.add('pressed');
    follow(event.clientX);
  });
  nav.addEventListener('pointermove', event => { if (event.pointerId === pointer) follow(event.clientX); });
  const finish = (event, cancelled) => {
    if (event.pointerId !== pointer) return;
    pointer = null;
    pill.classList.remove('pressed');
    highlight(-1);
    const index = liquidGlassIndex(event.clientX - origin, slots);
    if (cancelled) { placeIndex(active); return; }
    // The captured pointer turns the tap into a click on the nav itself;
    // swallow that click so the button handler does not route twice.
    suppressClick = performance.now();
    placeIndex(index);
    if (index !== active) {
      active = index;
      try { navigator.vibrate?.(8); } catch { /* vibration is optional */ }
      onSelect(buttons[index].dataset.omView);
    }
  };
  nav.addEventListener('pointerup', event => finish(event, false));
  nav.addEventListener('pointercancel', event => finish(event, true));
  nav.addEventListener('click', event => {
    if (performance.now() - suppressClick < 500) { event.preventDefault(); event.stopImmediatePropagation(); }
  }, true);
}
