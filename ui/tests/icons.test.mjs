import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {icon, iconNames} from '../shared/icons.js';

const designIcons = ['shield','shield-error','shield-alert','grid','apps','settings','link','chevron','back','more','arrow','refresh','moon','sun','cpu','key','clock','server','phone','battery','check','search','eye','palette','globe','sliders','lock','file','info','close','help'];

test('the shared set carries every design glyph', () => {
  assert.deepEqual([...iconNames].sort(), [...designIcons].sort());
  for (const name of designIcons) {
    const markup = icon(name);
    assert.match(markup, /^<svg class="icon" /, name);
    assert.match(markup, /viewBox="0 0 24 24"/, name);
    assert.match(markup, /<(path|rect|circle)\b/, name);
    assert.doesNotMatch(markup, /undefined/, name);
  }
});

test('page-specific names resolve to the design glyphs', () => {
  assert.match(icon('overview'), /<rect x="3.5"/);
  assert.match(icon('apps'), /<rect x="3" y="3" width="8"/);
  assert.match(icon('settings'), /<circle cx="9" cy="7"/);
  assert.match(icon('arrow'), /M4 12h15/);
  assert.match(icon('chevron'), /m9 5 7 7-7 7/);
  assert.match(icon('back'), /M20 12H4/);
  assert.match(icon('battery'), /<rect x="3" y="7"/);
  assert.equal(icon('not-a-real-icon'), icon('shield'));
});

test('option overrides keep the icon usable in cards and dialogs', () => {
  const markup = icon('shield', {size: 28, className: 'status-icon'});
  assert.match(markup, /class="status-icon"/);
  assert.match(markup, /width="28"/);
  assert.match(markup, /height="28"/);
});

test('every literal icon referenced by the packaged pages exists', async () => {
  for (const [side, file] of [['a', 'modern.js'], ['b', 'app.js']]) {
    const source = await readFile(new URL(`../../${side}-side/source/template/webroot/${file}`, import.meta.url), 'utf8');
    for (const match of source.matchAll(/icon\('([a-z-]+)'\)/g)) {
      const name = match[1];
      assert.ok(name === 'overview' || iconNames.includes(name), `${side}/${file}: ${name}`);
    }
  }
  for (const side of ['a', 'b']) {
    const html = await readFile(new URL(`../../${side}-side/source/template/webroot/index.html`, import.meta.url), 'utf8');
    for (const match of html.matchAll(/data-icon="([a-z-]+)"/g)) {
      assert.ok(match[1] === 'overview' || iconNames.includes(match[1]), `${side}: ${match[1]}`);
    }
  }
});

test('the shared stylesheet keeps the design tokens and has no mojibake', async () => {
  const css = await readFile(new URL('../shared/material-you.css', import.meta.url), 'utf8');
  for (const token of ['--bg','--surface','--primary','--success','--error','--warning','--shadow','--scrim']) {
    assert.ok(css.includes(token + ':'), token);
  }
  assert.match(css, /@media \(prefers-color-scheme: dark\)/);
  assert.doesNotMatch(css, /\uFFFD/);
});
