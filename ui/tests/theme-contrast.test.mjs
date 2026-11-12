// Guards the theme contract between the shared tokens and the A-side styles.
//
// The theme attribute lives on :root (ui-kit.js applyAppearance), so a dark rule
// scoped to .om-modern[data-theme=dark] can never match, and a colour named
// without a token definition silently keeps its literal fallback in every theme.
// Both mistakes together left the diagnostics log boxes light-on-light in dark
// mode, so these checks resolve real computed colours instead of matching text.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const read = url => readFileSync(new URL(url, import.meta.url), 'utf8');
const shared = read('../shared/modern.css');
const aCss = read('../../a-side/source/template/webroot/a-modern.css');

/** Flat list of {prelude, body} for every rule in the sheet, at any depth. */
function rules(css) {
  const found = [];
  let prelude = '';
  for (let i = 0; i < css.length; i += 1) {
    const char = css[i];
    if (char === '/') {
      const end = css.indexOf('*/', i + 2);
      if (css[i + 1] === '*' && end !== -1) { i = end + 1; continue; }
    }
    if (char === '{') {
      let depth = 0;
      let close = i;
      for (; close < css.length; close += 1) {
        if (css[close] === '{') depth += 1;
        else if (css[close] === '}' && --depth === 0) break;
      }
      const selector = prelude.trim();
      // Comments may sit between declarations; strip them so a rule body is
      // always parseable as a plain declaration list.
      const body = css.slice(i + 1, close).replace(/\/\*[\s\S]*?\*\//g, ' ');
      // An at-rule such as @media wraps further rules; keep both the wrapper
      // (so its declarations remain reachable) and every nested rule.
      found.push({prelude: selector, body});
      if (selector.startsWith('@')) found.push(...rules(body));
      i = close;
      prelude = '';
      continue;
    }
    if (char === ';' || char === '}') { prelude = ''; continue; }
    prelude += char;
  }
  return found;
}

const allRules = rules(shared);

function declarations(body) {
  const map = new Map();
  for (const [, name, value] of body.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) {
    map.set(name, value.trim());
  }
  return map;
}

function lastBlock(predicate, label) {
  const matches = allRules.filter(predicate);
  assert.ok(matches.length, 'missing stylesheet block: ' + label);
  return matches[matches.length - 1];
}

// material-you.css owns the aliases; these literals are the resolved values.
const lightBlock = lastBlock(rule => rule.prelude === ':root' && /--om-ink:#1b1b25/.test(rule.body), 'light :root');
const darkExplicitBlock = lastBlock(rule => rule.prelude === ':root[data-theme=dark]', ':root[data-theme=dark]');
const darkMediaBlock = lastBlock(rule => rule.prelude === ':root:not([data-theme=light])', 'prefers-color-scheme:dark :root');

const themes = {
  light: declarations(lightBlock.body),
  'dark (system)': new Map([...declarations(lightBlock.body), ...declarations(darkMediaBlock.body)]),
  'dark (explicit)': new Map([...declarations(lightBlock.body), ...declarations(darkExplicitBlock.body)]),
};

function luminance(hex) {
  const channels = [1, 3, 5].map(at => {
    const value = parseInt(hex.slice(at, at + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(a, b) {
  const [high, low] = [luminance(a), luminance(b)].sort((left, right) => right - left);
  return (high + 0.05) / (low + 0.05);
}

const COLOUR_PROPERTY = /^(?:background|background-color|color|border|border-color|outline-color|fill|stroke)$/;

test('every colour token the A-side styles use is defined by the shared theme', () => {
  const undefinedColours = new Set();
  for (const rule of rules(aCss)) {
    for (const [, property, value] of rule.body.matchAll(/([-a-z]+)\s*:\s*([^;{}]+)(?:;|$)/g)) {
      if (!COLOUR_PROPERTY.test(property)) continue;
      for (const [, token] of value.matchAll(/var\((--om-[a-z0-9-]+)\s*(?:,|\))/g)) {
        if (!themes.light.has(token) && !themes['dark (explicit)'].has(token)) undefinedColours.add(token);
      }
    }
  }
  assert.deepEqual([...undefinedColours], [], 'colour tokens without a theme definition keep their literal fallback');
});

test('the diagnostics log surface follows both dark paths with readable contrast', () => {
  const rule = rules(aCss).find(entry => entry.prelude === '.om-diag-log');
  assert.ok(rule, 'missing the diagnostics log rule');
  const background = /(?:^|;)\s*background\s*:\s*([^;]+);/.exec(rule.body);
  const colour = /(?:^|;)\s*color\s*:\s*([^;]+);/.exec(rule.body);
  assert.ok(background && colour, 'the log rule must set its own background and colour');
  // A literal fallback pins one theme; that is exactly how this regressed.
  assert.doesNotMatch(background[1], /#[0-9a-f]{3,8}|rgba?\(/i, 'the log background must not pin a literal colour');

  const token = /^var\((--om-[a-z0-9-]+)\)$/.exec(background[1].trim());
  assert.ok(token, 'the log background must be a theme token, got: ' + background[1]);

  const surfaces = {};
  for (const [label, tokens] of Object.entries(themes)) {
    const surface = tokens.get(token[1]);
    const ink = /^var\((--om-[a-z0-9-]+)\)$/.exec(colour[1].trim());
    assert.ok(ink, 'the log colour must be a theme token, got: ' + colour[1]);
    const inkValue = tokens.get(ink[1]);
    assert.ok(surface, label + ' does not define ' + token[1]);
    assert.ok(inkValue, label + ' does not define ' + ink[1]);
    const ratio = contrast(surface, inkValue);
    assert.ok(ratio >= 4.5, label + ' log contrast is too low: ' + ratio.toFixed(2));
    surfaces[label] = surface;
  }
  assert.notEqual(surfaces.light, surfaces['dark (system)'], 'dark mode must actually change the log surface');
  assert.equal(surfaces['dark (system)'], surfaces['dark (explicit)'], 'both dark paths must agree');
});

test('dark overrides are rooted where the theme is actually applied', () => {
  assert.match(read('../shared/ui-kit.js'), /documentElement\.dataset\.theme/);
  for (const rule of rules(aCss)) {
    if (!/\[data-theme/.test(rule.prelude)) continue;
    assert.doesNotMatch(rule.prelude, /^\.om-modern\b/, 'theme selector is scoped to .om-modern: ' + rule.prelude);
    assert.match(rule.prelude, /:root/, 'theme selector must be rooted at :root: ' + rule.prelude);
  }
});
