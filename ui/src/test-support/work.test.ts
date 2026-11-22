import assert from 'node:assert/strict';
import test from 'node:test';
import { assertLinearWork, measureWork } from './work.ts';

const items = (count: number) => Array.from({ length: count }, (_, at) => ({ key: `K${at}` }));
const linear = (build: (items: readonly { key: string }[]) => void) => () => assertLinearWork(count => measureWork(items(count), build), 'building');
const quadratic = /building: (copied|visited|reads|probes) at 50, 500, 5000 items were/;

test('appending is linear however it is spelled, and a prepend that shifts the whole list is not', () => {
  linear(input => { const out: unknown[] = []; for (const item of input) out.push(item); })();
  linear(input => { const out: unknown[] = []; for (const item of input) out.splice(out.length, 0, item); })();
  linear(input => { const out: unknown[] = []; for (const item of input) { out.push(item); out.unshift(...[]); } })();
  assert.throws(linear(input => { const out: unknown[] = []; for (const item of input) out.splice(undefined as unknown as number, 0, item); }), quadratic);
  assert.throws(linear(input => { const out: unknown[] = []; for (const item of input) out.unshift(item); out.reverse(); }), quadratic);
  assert.throws(linear(input => { let out: unknown[] = []; for (const item of input) out = out.concat(item); }), quadratic);
});

test('a scan is charged the elements it visits, and per-block overhead stays linear', () => {
  assert.equal(measureWork(items(5000), input => { input.find(item => item.key === 'K0'); }).visited, 1);
  assert.equal(measureWork(items(5000), input => { input.indexOf(input[3]!); }).visited, 4);
  linear(input => { for (let block = 0; block < input.length; block += 128) for (const item of input.slice(block, block + 128)) void item; })();
  assert.throws(linear(input => { const seen: string[] = []; for (const item of input) if (!seen.includes(item.key)) seen.push(item.key); }), quadratic);
  assert.throws(linear(input => { const held = new Map<string, unknown>(); for (const item of input) held.set(item.key, [...held.values()].find(other => other === item)); }), quadratic);
});
