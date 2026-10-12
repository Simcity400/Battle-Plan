import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { LABEL_WEIGHT, VALUE_WEIGHT } from '../draw.ts';

const css = readFileSync(new URL('./table.css', import.meta.url), 'utf8');
const weight = (selector: string) => {
  const rule = css.split('\n').find(line => line.startsWith(`${selector} {`));
  assert.ok(rule, `${selector} has a rule`);
  return Number(rule.match(/font-weight:\s*(\d+)/)?.[1]);
};

test('table cells use the bar charts\' name and value weights; ranks stay quiet', () => {
  assert.equal(weight('.chart-table td'), LABEL_WEIGHT);
  assert.equal(weight('.chart-table td.is-numeric'), VALUE_WEIGHT);
  assert.equal(weight('.chart-table .table-rank'), 400);
});
