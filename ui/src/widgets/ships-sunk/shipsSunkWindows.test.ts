import assert from 'node:assert/strict';
import { test } from 'node:test';
import { sinkingWindowRows } from './shipsSunkWindows.ts';
import type { ShipsSunkWindowReply } from './useShipsSunk';
import { validateSinkingManifest, validateSinkingWindow } from './useShipsSunk.ts';
import type { ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';
const row = (sinkHour: number, knownAt: number): ShipsSunkRow => ({ sinkHour, knownAt, name: 'recorded ship', classIndex: 0,
  recordedNameIndex: 0, victim: 'mod victim', attacker: null, victimAsOf: null, attackerAsOf: null, placeIndex: 0,
  cause: 'hostile', attributionName: null, assistedKill: false });
function page(window: number, indices: number[], rows: ShipsSunkRow[], token: number): ShipsSunkWindowReply {
  const reply: ShipsSunkReply = { branchId: '1', available: true, firstHour: 100, lastHour: 130, firstRead: 100, unread: [],
    classes: [{ token, key: `mod class ${token}`, name: null, icon: null }], places: [`mod sea ${token}`],
    recordedNames: [`recorded class ${token}`], rows, warnings: [] };
  return { window, indices, reply };
}
test('window dictionaries and stable sinking indices survive a late native-date repair', () => {
  const first = page(0, [0, 3, 4], [row(99, 100), row(105, 130), row(105, 130)], 70);
  const second = page(1, [1, 2], [row(115, 120), row(115, 120)], 4);
  const pages = new Map([[0, first], [1, second]]);
  const read = (hour: number, from: number | null = null) => sinkingWindowRows(pages, hour, [], 'victim', () => true, from, 'mod campaign');
  assert.deepEqual(read(120).map(row => row.key), ['1', '2', '0']);
  const visible = read(130);
  assert.deepEqual(visible.map(row => row.key), ['1', '2', '3', '4', '0']);
  assert.equal(visible[0].item.page.places[visible[0].item.placeIndex!], 'mod sea 4');
  assert.equal(visible[2].item.page.recordedNames[visible[2].item.recordedNameIndex!], 'recorded class 70');
  assert.deepEqual(read(130, 105).map(row => row.key), ['1', '2', '3', '4']);
  assert.deepEqual(sinkingWindowRows(pages, 130, ['mod class 70'], 'both', () => true, null, 'mod campaign').map(row => row.key), ['3', '4', '0']);
});

test('pending sinking pages preserve their coverage so held windows can follow repairs', () => {
  const pending = [{ firstHour: 111, lastHour: 130 }];
  const value = { ...page(0, [0], [row(105, 110)], 70),
    coverage: { schema: 1, firstHour: 100, lastHour: 110, complete: false }, pending };
  assert.deepEqual(validateSinkingWindow(value).coverage, value.coverage);
  assert.deepEqual(validateSinkingWindow(value).pending, pending);
  assert.deepEqual(validateSinkingManifest({ windows: [{ firstHour: 100, lastHour: 130 }], classes: value.reply,
    coverage: { firstHour: 100, lastHour: 99, complete: false }, pending }).pending, pending);
  assert.throws(() => validateSinkingWindow({ ...value, pending: [{ firstHour: 130, lastHour: 111 }] }));
});
