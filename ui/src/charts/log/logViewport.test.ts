import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('./log.css', import.meta.url), 'utf8');

/** The declarations of the rule for exactly `selector`, with `overflow` split into its axes. */
function declarations(selector: string): Map<string, string> {
  const rule = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(match => match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim() === selector);
  assert.ok(rule, `${selector} has a rule`);
  const found = new Map<string, string>();
  for (const declaration of rule[2].split(';')) {
    const at = declaration.indexOf(':');
    if (at < 0) continue;
    const name = declaration.slice(0, at).trim();
    const value = declaration.slice(at + 1).trim();
    if (name === 'overflow') {
      const [x, y = x] = value.split(/\s+/);
      found.set('overflow-x', x);
      found.set('overflow-y', y);
    } else found.set(name, value);
  }
  return found;
}

// LogList draws only the rows in view, chosen from the viewport's measured height,
// and measures cards at the viewport's width. A scrollbar that comes and goes with
// what is drawn changes that viewport, which changes what is drawn: the list
// re-renders without end ("Maximum update depth exceeded").
test('a row drawn at the edge of the view cannot bring a sideways scrollbar that shortens the view', () => {
  const scroller = declarations('.log-scroll');
  assert.equal(scroller.get('overflow-y'), 'auto');
  assert.ok(['hidden', 'clip'].includes(scroller.get('overflow-x') ?? 'visible'), `log rows clip rather than scroll sideways (overflow-x: ${scroller.get('overflow-x')})`);
});

test('card logs keep their width whether or not the cards overflow the view', () => {
  assert.equal(declarations('.log.is-card-log .log-scroll').get('scrollbar-gutter'), 'stable');
});
