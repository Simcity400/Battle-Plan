import assert from 'node:assert/strict';
import test from 'node:test';
import { VirtualListIndex } from './virtualListIndex.ts';
import { selectIndex } from './selectNavigation.ts';
test('5000 measured rows retain full offsets while selecting only a viewport', () => {
  const list = new VirtualListIndex(5000, 60);
  assert.equal(list.offset(5000), 300000);
  assert.deepEqual(list.range(60000, 300), [997, 1009]);
  list.set(1000, 300);
  assert.equal(list.offset(1001), 60300);
  assert.equal(list.rowAt(60299), 1000);
  assert.equal(list.rowAt(60300), 1001);
  assert.equal(list.offset(5000), 300240);
  list.set(1000, 60); assert.equal(list.offset(5000), 300000);
});
test('keyboard navigation reaches every option beyond the viewport and skips disabled rows', () => {
  const options = Array.from({ length: 5000 }, (_, i) => ({ disabled: i === 4998 }));
  assert.equal(selectIndex(options, 0, 'End'), 4999);
  assert.equal(selectIndex(options, 4999, 'ArrowUp'), 4997);
  assert.equal(selectIndex(options, 4999, 'ArrowDown'), 0);
  assert.equal(selectIndex(options, 4999, 'Home'), 0);
  assert.equal(selectIndex([{ disabled: true }], 0, 'Home'), null);
  const explained = [{ disabled: false }, { disabled: true, reason: 'Load a campaign first' }, { disabled: true }, { disabled: false }];
  assert.equal(selectIndex(explained, 0, 'ArrowDown'), 1, 'a disabled option with a reason stays keyboard-focusable');
  assert.equal(selectIndex(explained, 1, 'ArrowDown'), 3, 'a disabled option without a reason is skipped');
  assert.equal(selectIndex(explained, 3, 'ArrowUp'), 1);
});

test('authored row gaps contribute to prefixes and viewport bounds without trailing spacing', () => {
  const list = new VirtualListIndex(5000, 52, 1);
  assert.equal(list.offset(1), 53);
  assert.equal(list.offset(56), 56 * 53);
  assert.equal(list.offset(5000), 5000 * 52 + 4999, 'only the spaces between authored rows contribute to total height');
  assert.equal(list.rowAt(52), 0, 'the gap following a row remains before the next row start');
  assert.equal(list.rowAt(53), 1);
  assert.deepEqual(list.range(2500 * 53, 52, 0), [2500, 2501], 'window bounds use the same spaced prefixes as drawing');
  list.set(0, 172);
  assert.equal(list.offset(1), 173, 'expansion and spacing both precede the next row');
  assert.equal(list.offset(5000), 5000 * 52 + 4999 + 120);
  assert.equal(list.rowAt(172), 0);
  assert.equal(list.rowAt(173), 1);
  assert.equal(list.setGap(2.5), true);
  assert.equal(list.setGap(2.5), false, 'unchanged spacing does not request another render');
  assert.equal(list.offset(1), 174.5);
  assert.equal(list.offset(5000), 5000 * 52 + 4999 * 2.5 + 120);
  assert.equal(new VirtualListIndex(1, 52, 10).offset(1), 52, 'a single row has no gap');
  assert.equal(new VirtualListIndex(0, 52, 10).offset(0), 0, 'an empty list has no gap');
});
