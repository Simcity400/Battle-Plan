import assert from 'node:assert/strict';
import { test } from 'node:test';
import { sinkingWindowRows } from './shipsSunkWindows.ts';
import type { ShipsSunkWindowReply } from './useShipsSunk';
import { validateSinkingManifest, validateSinkingWindow } from './useShipsSunk.ts';
import { deepFreeze } from '../../testFrozen.ts';
import type { ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';
const row = (sinkHour: number, knownAt: number): ShipsSunkRow => ({ sinkHour, knownAt, name: 'recorded ship', classIndex: 0,
  recordedNameIndex: 0, victim: 'mod victim', attacker: null, victimAsOf: null, attackerAsOf: null, placeIndex: 0,
  cause: 'hostile', attributionName: null, assistedKill: false,
  sinkingCause: { kind: 'notRecorded', reason: 'sinking cause was not recorded' },
  battle: { kind: 'notRecorded', reason: 'sinking battle was not recorded' } });
function page(window: number, indices: number[], rows: ShipsSunkRow[], token: number): ShipsSunkWindowReply {
  const reply: ShipsSunkReply = { branchId: '1', window, available: true, firstHour: 100, lastHour: 130, firstRead: 100, unread: [],
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

test('sinking content is prepared once and retained until a visibility boundary or replacement', () => {
  let reads = 0;
  const old = row(100, 100), future = row(110, 130);
  Object.defineProperty(future, 'knownAt', { get() { reads++; return 130; } });
  const original = deepFreeze(page(0, [0, 1], [old, future], 70));
  const pages: ReadonlyMap<number, ShipsSunkWindowReply> = deepFreeze(new Map([[0, original]]));
  const country = () => true;
  const read = (hour: number) => sinkingWindowRows(pages, hour, [], 'both', country, null, 'campaign');
  const first = read(101);
  reads = 0;
  for (let hour = 102; hour < 130; hour++) assert.equal(read(hour), first);
  assert.equal(reads, 0);
  const crossed = read(130);
  assert.notEqual(crossed, first);
  assert.equal(crossed[1], first[0]);
  assert.equal(crossed.length, 2);
  const repaired = deepFreeze({ ...original, reply: { ...original.reply,
    rows: original.reply.rows.map(row => ({ ...row, name: 'repaired' })) } });
  const replacement = deepFreeze(new Map([[0, repaired]]));
  assert.equal(sinkingWindowRows(replacement, 130, [], 'both', country, null, 'campaign')[0].item.name, 'repaired');
});


test('a window reply keys its rows by window and place, and the manifest refuses an unordered window list', () => {
  const reply = page(3, [], [row(100, 100), row(101, 124)], 5).reply;
  const window = validateSinkingWindow(reply);
  assert.equal(window.window, 3);
  assert.deepEqual(window.indices, [3 * 2 ** 32, 3 * 2 ** 32 + 1]);
  assert.throws(() => validateSinkingWindow({ ...reply, window: -1 }));
  const manifest = { branchId: '1', windows: [{ firstHour: 100, lastHour: 130 }], available: true, firstRead: 100, classes: [], recordedNames: [], warnings: [] };
  assert.equal(validateSinkingManifest(manifest), manifest);
  assert.throws(() => validateSinkingManifest({ ...manifest, windows: [{ firstHour: 130, lastHour: 100 }] }));
});

test('a sinking first listed after it sank is revealed at its listing, so a block draws it ahead and the clock shows it then', () => {
  const pages = new Map([[0, page(0, [0, 1, 2], [row(99, 100), row(105, 130), row(110, 110)], 70)]]);
  const drawn = sinkingWindowRows(pages, 130, [], 'both', () => true, null, 'campaign');
  assert.deepEqual(drawn.map(row => [row.key, row.hour, row.revealedHour]), [['2', 110, undefined], ['1', 105, 130], ['0', 99, 100]]);
  assert.deepEqual(sinkingWindowRows(pages, 129, [], 'both', () => true, null, 'campaign').map(row => row.key), ['2', '0']);
});
