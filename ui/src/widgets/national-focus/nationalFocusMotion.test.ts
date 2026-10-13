import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('./nationalFocus.css', import.meta.url), 'utf8');
const rules = [...css.replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(match => ({ selector: match[1].trim(), body: match[2] }));
const rule = (selector: string) => {
  const found = rules.find(item => item.selector === selector)?.body;
  assert.ok(found, selector);
  return found;
};

test('completed focus rows open and close by revealing their own clipped height, never painting over the next row', () => {
  assert.match(rule('.national-focus .focus-details'), /interpolate-size:\s*allow-keywords/);
  const closed = rule('.national-focus .focus-details::details-content');
  assert.match(closed, /block-size:\s*0;/);
  assert.match(closed, /overflow-y:\s*clip/);
  assert.match(closed, /opacity:\s*0;/);
  assert.match(closed, /transition:[^;]*\bblock-size var\(--t-slow\) var\(--ease\)/);
  assert.match(closed, /transition:[^;]*\bopacity var\(--t-slow\) var\(--ease\)/);
  // Hidden content stays painted until the closing reveal ends.
  assert.match(closed, /transition:[^;]*\bcontent-visibility var\(--t-slow\) allow-discrete/);
  const open = rule('.national-focus .focus-details[open]::details-content');
  assert.match(open, /block-size:\s*auto/);
  assert.match(open, /opacity:\s*1/);
  // The shared entrance starts below its box, so inside a row it would overlap the row beneath.
  assert.match(rule('.national-focus .focus-description'), /animation:\s*none/);
});

test('a description that arrives after its row opened grows in rather than stepping', () => {
  assert.match(rule('.national-focus .focus-description p'), /transition:[^;]*\bblock-size var\(--t-slow\)/);
  const start = css.match(/@starting-style\s*\{\s*\.national-focus \.focus-description p\s*\{([^{}]*)\}\s*\}/)?.[1];
  assert.ok(start, 'the paragraph starts collapsed');
  assert.match(start, /block-size:\s*0/);
  assert.match(start, /margin-top:\s*0/);
});

test('focus disclosure motion uses only the shared durations that reduced motion sets to zero', () => {
  const moving = rules.filter(item => item.selector.startsWith('.national-focus') && /transition|animation/.test(item.body));
  assert.ok(moving.length >= 2);
  for (const { selector, body } of moving) assert.doesNotMatch(body, /\b\d+(?:\.\d+)?m?s\b/, `${selector} has a fixed duration`);
  const root = readFileSync(new URL('../../styles.css', import.meta.url), 'utf8');
  assert.match(root, /:root\[data-motion='reduced'\]\s*\{[^}]*--t-slow:\s*0ms/);
});
