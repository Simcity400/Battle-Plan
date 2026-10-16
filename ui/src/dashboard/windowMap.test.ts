import assert from 'node:assert/strict';
import test from 'node:test';
import { WindowMap, withWindowEntries } from './windowMap.ts';
test('5000 arrivals preserve earlier snapshots and reuse identical values without copying entries', () => {
  let windows = new WindowMap<object>(); const values = Array.from({ length: 5000 }, () => ({}));
  for (let i = 0; i < values.length; i++) windows = windows.with(i, values[i]);
  const next = windows.with(4999, { changed: true });
  assert.equal(windows.get(4999), values[4999]); assert.equal(next.get(0), values[0]);
  assert.equal(windows.with(1000, values[1000]), windows);
  assert.equal(windows.size, 5000); assert.equal([...windows.keys()].length, 5000);
  const iterable = new Proxy(windows, { get(target, key) { if (key === Symbol.iterator) throw new Error('arrival copied all windows'); const value = Reflect.get(target, key); return typeof value === 'function' ? value.bind(target) : value; } });
  assert.equal(withWindowEntries(iterable, [[5000, {}]]).size, 5001);
});
