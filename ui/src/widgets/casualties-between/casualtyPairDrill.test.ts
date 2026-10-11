import assert from 'node:assert/strict';
import test from 'node:test';
import { runAt } from '../../charts/series.ts';
import { compactStackExtent } from '../../charts/stack.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import type { BoundTrack } from '../../dashboard/model';
import type { CasualtyPairsReply } from '../../generated/dashboard';
import { casualtyPairsAt } from './casualtyPairsData.ts';
import { casualtyPairIds, pairHistory, pinPairs, readPairSelection } from './casualtyPairDrill.ts';
import '../definitions.ts';

const windows = [{ firstHour: 100, lastHour: 119 }, { firstHour: 120, lastHour: 139 }];
const reply = (window: number, identity: string, first: number, last: number, value: string): CasualtyPairsReply => ({
  branchId: '1', available: true, window, windows, observed: [{ firstHour: first, lastHour: last }], hasGap: true, warnings: [],
  pairs: [{ identity, firstCountry: 'mod attacker', secondCountry: 'mod victim', runs: [
    { firstHour: first, lastHour: last, readHour: first, hourly: true, firstTaken: '0', secondTaken: value, incomplete: false },
  ] }],
});

test('pair parts history retains signed rows, observed gaps and selected native holders', () => {
  const original = reply(0, '1:10|2:20', 100, 110, '-12');
  const continued = reply(1, '1:10|2:20', 120, 124, '9007199254740993');
  continued.pairs[0].firstCountry = 'renamed mod attacker';
  const replacement = reply(1, '1:11|2:20', 125, 139, '999');
  const ids = casualtyPairIds([original]);
  const model = pairHistory([original, { ...continued, observed: windows.slice(1), pairs: [...continued.pairs, ...replacement.pairs] }], ids);
  assert.equal(model.bands.length, 2);
  assert.equal(pairHistory([original], ids).bands[0][0], model.bands[0][0], 'a window is decoded once, however many arrive after it');
  assert.equal(casualtyPairsAt(original, 105).flows[0].value, -12n);
  assert.equal(casualtyPairsAt(original, 105).flows[1].value, 0n);
  assert.equal(runAt(model.bands[0], 105)?.value, 0, 'a correction has no positive geometry');
  assert.equal(runAt(model.bands[1], 105)?.value, 0, 'recorded zero stays reachable');
  assert.equal(runAt(model.bands[0], 115), null, 'unobserved hours break the history');
  assert.equal(compactStackExtent(model, 115, 119), null);
  assert.equal(runAt(model.bands[0], 123)?.value, Number(9007199254740993n));
  assert.equal(casualtyPairsAt(continued, 123).flows[0].id, ids[0], 'the same holder keeps its directed identity through a rename');
  assert.equal(runAt(model.bands[0], 130), null, 'a reused country tag cannot contribute another holder or a synthesized zero');
  assert.equal(casualtyPairsAt({ ...continued, observed: windows.slice(1), pairs: replacement.pairs }, 130).flows.filter(flow => ids.includes(flow.id)).length, 0);
});

test('pair Pin retains directed identities, branch-relative day and explicit Follow', () => {
  const source = createWidget('casualties-between', 'main');
  const track = { id: 'main', axisOffset: 30, source: { metadata: { campaignKey: 'synthetic campaign' } },
    branch: { id: '1', firstHour: 100, lastHour: 500 } } as BoundTrack;
  const ids = casualtyPairIds([reply(0, '1:10|2:20', 100, 110, '-12')]);
  const held = pinPairs(source, track, ids, 127.9, false);
  assert.deepEqual(held.time, { mode: 'pin', windows: { main: { firstOffset: 27, lastOffset: 27 } } });
  assert.equal(resolveWindow(held, track, [track], 400)?.lastHour, 127);
  assert.equal(held.options?.show, 'pairs');
  assert.deepEqual(readPairSelection(held.options?.pairSelection, track), ids);
  assert.equal(readPairSelection(held.options?.pairSelection, { ...track, branch: { ...track.branch, id: 'other' } }), null);
  assert.equal(readPairSelection(held.options?.pairSelection, { ...track, source: { ...track.source, metadata: { ...track.source.metadata, campaignKey: 'other' } } }), null);
  const live = pinPairs(source, track, ids, 127, true);
  assert.deepEqual(live.time, { mode: 'follow' });
  assert.equal(resolveWindow(live, track, [track], 170)?.lastHour, 200);
});
