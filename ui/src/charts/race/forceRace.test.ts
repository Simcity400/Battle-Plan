import assert from 'node:assert/strict';
import test from 'node:test';
import { breakdownValue, breakdownWindows, forceWindows, joinForceWindows, validateForceReply } from './forceRace.ts';
import type { ForceSizeReply } from '../../generated/dashboard';

const windows = [{ firstHour: 0, lastHour: 719 }, { firstHour: 720, lastHour: 1439 }, { firstHour: 1440, lastHour: 1500 }];
const reply = (window: number, parts: string[], runs: ForceSizeReply['series'][number]['runs'], membership: { firstHour: number; lastHour: number }[] | null = null): ForceSizeReply => ({
  branchId: '1', windows: [], window, interval: windows[window], warnings: [`w${window}`],
  parts: parts.map(key => ({ key, name: key, unresolved: key.startsWith(':') })),
  series: [{ country: 'GER', identities: [{ firstHour: windows[window].firstHour, lastHour: windows[window].lastHour, slot: 3, countryId: 30 }],
    segments: [{ firstHour: windows[window].firstHour, lastHour: windows[window].lastHour }], membership, runs }],
});

test('a view reads the windows it meets, and following the one after the moment', () => {
  assert.deepEqual(forceWindows(windows, { firstHour: 700, lastHour: 700 }, 700, false), [0]);
  assert.deepEqual(forceWindows(windows, { firstHour: 700, lastHour: 700 }, 700, true), [0, 1]);
  assert.deepEqual(forceWindows(windows, { firstHour: 0, lastHour: 2000 }, 1500, true), [0, 1, 2]);
  assert.deepEqual(forceWindows(windows, { firstHour: 1500, lastHour: 1500 }, 1500, true), [2]);
});

test('windows join on every part any of them names, a known split holding none of a part its window lacks', () => {
  const first = reply(0, ['fighter'], [{ firstHour: 0, lastHour: 719, startsSegment: true, total: 5, values: [5] }], [{ firstHour: 0, lastHour: 719 }]);
  const second = reply(1, [':not-identified', 'cas'], [
    { firstHour: 720, lastHour: 799, startsSegment: false, total: 7, values: [3, 4] },
    { firstHour: 800, lastHour: 1439, startsSegment: false, total: 9, values: [null, null] },
  ], [{ firstHour: 720, lastHour: 1000 }]);
  const joined = joinForceWindows({ branchId: '1', warnings: ['m'] }, new Map([[1, second], [0, first]]));
  assert.deepEqual(joined.parts.map(part => part.key), [':not-identified', 'cas', 'fighter']);
  assert.deepEqual(joined.series[0].runs.map(run => [run.firstHour, run.total, run.values]), [
    [0, 5, [0, 0, 5]], [720, 7, [3, 4, 0]], [800, 9, [null, null, null]]]);
  assert.deepEqual(joined.series[0].identities, [{ firstHour: 0, lastHour: 1439, slot: 3, countryId: 30 }]);
  assert.deepEqual(joined.series[0].segments, [{ firstHour: 0, lastHour: 1439 }]);
  assert.deepEqual(joined.series[0].membership, [{ firstHour: 0, lastHour: 1000 }]);
  assert.deepEqual(joined.warnings, ['m', 'w0', 'w1']);
  validateForceReply({ ...first, windows });
});


test('a breakdown reads the windows its view meets, its origin’s and its moment’s, and following, the next', () => {
  const windows = [{ firstHour: 0, lastHour: 719 }, { firstHour: 720, lastHour: 1439 }, { firstHour: 1440, lastHour: 1999 }];
  assert.deepEqual(breakdownWindows(windows, { firstHour: 700, lastHour: 800 }, [0, 1999], 750, 10, false), [0, 1]);
  assert.deepEqual(breakdownWindows(windows, { firstHour: 1500, lastHour: 1500 }, [0, 1999], 1500, 10, false), [0, 2], 'the origin’s window holds the holder chosen');
  assert.deepEqual(breakdownWindows(windows, { firstHour: 100, lastHour: 100 }, [0, 1999], 100, 100, true), [0, 1], 'following, the next too');
});

test('a breakdown’s remainder is what no part explains; an unknown total leaves it unknown, never zero', () => {
  const run = { firstHour: 0, lastHour: 1, startsSegment: true, total: 10, values: [3, null, 4], referenceTotal: 20 };
  assert.deepEqual(breakdownValue({ total: 10, values: [3, null, 4], run, stretch: 0 }), { total: 10, values: [3, null, 4], remainder: 3, reference: 20 });
  assert.deepEqual(breakdownValue({ total: null, values: [null], run: { ...run, total: null }, stretch: 0 }).remainder, null);
  assert.deepEqual(breakdownValue(null), { total: null, values: [], remainder: null, reference: null });
});

