import assert from 'node:assert/strict';
import { test } from 'node:test';
import { visibleRange } from '../../components/countryList.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { validateFactionTotalsReply } from '../../dashboard/factionTotals.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { FactionMemberTrack, FactionTotalsReply } from '../../generated/dashboard';
import '../../widgets/definitions.ts';
import { runAt } from '../series.ts';
import { chooseFaction, factionNumbers, factionParts, factionPartShare, factionPartWitness, factionPick, factionSelection,
  pinFactionParts, savedFactionSelection } from './factionParts.ts';

function reply(tracks: FactionMemberTrack[], total = 10, lastHour = 9): FactionTotalsReply {
  return { branchId: '1', membershipMode: 'hour', recorded: [{ firstHour: 0, lastHour }], disjoint: [], unit: null,
    unavailableReason: null, warnings: [], memberTracks: tracks,
    series: [{ id: '401', runs: [{ firstHour: 0, lastHour, startsSegment: true, value: total }], names: [],
      members: [], countedMembers: tracks.map(track => ({ tag: track.tag, firstHour: 0, lastHour })),
      membership: [{ firstHour: 0, lastHour }], dissolutionHour: null, unavailableReason: null }] };
}
function track(tag: string, countryId: number, value: number | null, firstHour = 0, lastHour = 9): FactionMemberTrack {
  return { tag, runs: [{ firstHour, lastHour, startsSegment: true, value: value == null ? null : String(value) }], identities: [{ firstHour, lastHour, slot: countryId, countryId }] };
}

test('overlapping or keyboard faction picks keep their exact eligible set and clicked hour, with no ranked fallback', () => {
  const pending = factionPick('source-A', 6.8, 'hidden', ['900', '401', '900']);
  assert.deepEqual(pending.ids, ['900', '401']);
  assert.equal(pending.selected, null);
  assert.equal(pending.hour, 6);
  assert.equal(pending.follow, false);
  assert.equal(chooseFaction(pending, 'source-B', '401'), pending);
  assert.equal(chooseFaction(pending, 'source-A', 'hidden'), pending);
  assert.equal(chooseFaction(pending, 'source-A', '401').selected, '401');
  assert.deepEqual(factionPick('source-A', 6, '401', []).ids, []);
  assert.equal(factionPick('source-A', 6, '401').selected, '401');
  // A held campaign/pin/range never passes the unrelated global clock into open.
  const frozen = factionPick('source-A', 6.8, '401');
  assert.equal(frozen.hour, 6);
  assert.equal(frozen.follow, false);
  const following = factionPick('source-A', 6.8, '401', undefined, { playing: true, playhead: 7 });
  assert.equal(following.follow, true);
  assert.equal(following.origin, 6);
  assert.equal(factionPick('source-A', 6.8, undefined, ['401', '900'], { playing: true, playhead: 7 }).follow, false);
});

test('member amounts, signed shares, measured zero and unavailable amounts stay distinct', () => {
  const data = reply([track('A', 7, 12), track('B', 8, -2), track('C', 9, 0)]);
  const model = factionParts(validateFactionTotalsReply(data), '401', { firstHour: 0, lastHour: 9 })!;
  assert.deepEqual(model.parts.map(part => runAt(part.runs, 5)?.raw), [12n, -2n, 0n]);
  assert.equal(runAt(model.remainder, 5)?.raw, 0n);
  assert.equal(factionPartShare(12n, 10n), '120%');
  assert.equal(factionPartShare(-2n, 10n), '-20%');
  assert.equal(factionPartShare(0n, 10n), '0%');
  assert.equal(factionPartShare(null, 10n), '—');
  assert.equal(factionPartShare(1n, 0n), '—');
  data.memberTracks![0].runs[0].value = null;
  const unknown = factionParts(data, '401', { firstHour: 0, lastHour: 9 })!;
  assert.equal(runAt(unknown.parts[0].runs, 5)?.raw, null);
  assert.equal(runAt(unknown.remainder, 5)?.raw, 12n);
  data.series[0].runs = [];
  assert.deepEqual(factionParts(data, '401', { firstHour: 0, lastHour: 9 })!.remainder, []);
  const old = { ...data }; delete old.memberTracks;
  assert.equal(factionParts(old, '401', { firstHour: 0, lastHour: 9 }), null);
  const missingValues = reply([track('A', 7, 1)]);
  missingValues.memberTracks![0].runs = [];
  missingValues.series[0].runs = [];
  const stillIdentified = factionParts(missingValues, '401', { firstHour: 0, lastHour: 9 })!;
  assert.equal(stillIdentified.parts.length, 1);
  assert.equal(stillIdentified.unidentified, 0);
  assert.equal(runAt(stillIdentified.parts[0].runs, 5)?.raw, null);
  const signed = reply([track('A', 7, 1), track('B', 8, 1)], 1);
  signed.memberTracks![0].runs[0].value = '9223372036854775807';
  signed.memberTracks![1].runs[0].value = '-9223372036854775806';
  const exact = factionParts(validateFactionTotalsReply(signed), '401', { firstHour: 0, lastHour: 9 })!;
  assert.equal(runAt(exact.parts[0].runs, 5)?.raw, 9223372036854775807n);
  assert.equal(runAt(exact.remainder, 5)?.raw, 0n);
  signed.memberTracks![0].runs[0].value = '9223372036854775808';
  assert.throws(() => validateFactionTotalsReply(signed), /malformed/);
});

test('native identity joins renamed aliases and splits reused tags, with labels restricted to proved hours', () => {
  const data = reply([track('A', 7, 2, 0, 3), track('B', 7, 2, 4, 9)], 2);
  data.memberTracks![0].runs.push({ firstHour: 4, lastHour: 9, startsSegment: true, value: '5' });
  data.memberTracks![0].identities.push({ firstHour: 4, lastHour: 9, slot: 8, countryId: 8 });
  data.series[0].countedMembers = [{ tag: 'A', firstHour: 0, lastHour: 9 }, { tag: 'B', firstHour: 4, lastHour: 9 }];
  data.series[0].runs = [{ firstHour: 0, lastHour: 3, startsSegment: true, value: 2 },
    { firstHour: 4, lastHour: 9, startsSegment: true, value: 7 }];
  const model = factionParts(data, '401', { firstHour: 0, lastHour: 9 })!;
  assert.equal(model.parts.length, 2);
  const original = model.parts.find(part => part.id === '[7,7]')!;
  const reused = model.parts.find(part => part.id === '[8,8]')!;
  assert.equal(runAt(original.runs, 5)?.raw, 2n);
  assert.equal(runAt(original.runs, 5)?.firstHour, 0, 'proved same-native rename does not create a graph break');
  assert.equal(runAt(reused.runs, 2)?.raw, 0n);
  assert.equal(runAt(reused.counted, 2), null);
  assert.equal(runAt(reused.runs, 5)?.raw, 5n);
  assert.deepEqual(factionPartWitness(original, 5), { tag: 'B', hour: 5, firstHour: 4 });
  assert.deepEqual(factionPartWitness(reused, 2), null);
  const past = factionParts(reply([track('A', 7, 2, 0, 3)], 2), '401', { firstHour: 0, lastHour: 9 })!.parts[0];
  assert.deepEqual(factionPartWitness(past, 8), { tag: 'A', hour: 3, firstHour: 0 });
  data.memberTracks![0].identities = [];
  assert.equal(factionParts(data, '401', { firstHour: 0, lastHour: 9 })!.unidentified, 1);
});

test('five thousand members retain sparse many-read histories and bounded keyboard-accessible rows', () => {
  const reads = 48;
  const tracks = Array.from({ length: 5000 }, (_, index) => ({ ...track(`mod_${index}`, index + 1, 1, 0, reads - 1),
    runs: Array.from({ length: reads }, (_, hour) => ({ firstHour: hour, lastHour: hour, startsSegment: hour === 0, value: String(hour + 1) })) }));
  const data = reply(tracks, 5000, reads - 1);
  data.series[0].runs = Array.from({ length: reads }, (_, hour) => ({ firstHour: hour, lastHour: hour, startsSegment: hour === 0, value: 5000 * (hour + 1) }));
  const model = factionParts(data, '401', { firstHour: 0, lastHour: reads - 1 })!;
  assert.equal(model.parts.length, 5000);
  assert.equal(model.parts.reduce((sum, part) => sum + part.runs.length, 0), 5000 * reads);
  assert.equal(model.remainder.length, 1);
  for (const hour of [0, 17, reads - 1]) {
    assert.equal(model.parts.reduce((sum, part) => sum + runAt(part.runs, hour)!.raw!, 0n), BigInt(5000 * (hour + 1)));
  }
  for (const top of [0, 2500 * 36, 4999 * 36]) {
    const range = visibleRange(5000, top, 340, 36);
    assert.ok(range.end - range.start <= Math.ceil(340 / 36) + 13);
  }
  const picks = factionPick('source', 17, undefined, Array.from({ length: 5000 }, (_, index) => String(index + 1)));
  assert.equal(chooseFaction(picks, 'source', '5000').selected, '5000');
});

test('numeric ownership survives content-only loading and renaming but changes with native operands or selection', () => {
  const store = new NumericReply<FactionTotalsReply>();
  const data = reply([track('A', 7, 10)]);
  const numbers = store.update('source-A', data, factionNumbers)!;
  assert.equal(store.update('source-A', undefined, factionNumbers), numbers);
  const renamed = structuredClone(data);
  renamed.series[0].names = [{ firstHour: 0, lastHour: 9, name: 'Another content name', reason: null }];
  renamed.warnings = ['Content pending'];
  assert.equal(store.update('source-A', renamed, factionNumbers), numbers);
  const changed = structuredClone(renamed);
  changed.memberTracks![0].identities[0].countryId = 8;
  assert.notEqual(store.update('source-A', changed, factionNumbers), numbers);
  assert.equal(store.update('source-B', undefined, factionNumbers), undefined);
  assert.notEqual(store.update('source-A', data, factionNumbers), numbers);
});

test('pins keep native faction origin, campaign and branch scope, while Follow overrides an older widget pin', () => {
  const bound = { id: 'track', branch: { id: '1', firstHour: 100, lastHour: 200 },
    source: { metadata: { campaignKey: 'campaign-A' } } } as unknown as BoundTrack;
  const widget = { id: 'widget', kind: 'faction-strength', configVersion: 1, title: 'Faction strength', trackIds: ['track'],
    countries: { kind: 'factions', count: 1, membership: 'ever' }, time: { mode: 'pin', windows: {} } } as WidgetSpec;
  const pinned = pinFactionParts(widget, bound, '401', 140, 160, true, 'ever');
  assert.deepEqual(pinned.time, { mode: 'follow' });
  assert.deepEqual(pinned.countries, { kind: 'group', group: { type: 'faction', id: '401' }, aggregate: true, membership: 'ever' });
  assert.deepEqual(savedFactionSelection(pinned.options?.memberFaction, bound), { id: '401', origin: 140 });
  const other = { ...bound, source: { ...bound.source, metadata: { ...bound.source.metadata, campaignKey: 'campaign-B' } } };
  assert.equal(savedFactionSelection(pinned.options?.memberFaction, other), null);
  assert.equal(savedFactionSelection(factionSelection(bound, '401', 99), bound), null);
  assert.deepEqual(pinFactionParts(widget, bound, '401', 140, 160, false, 'hour').time,
    { mode: 'pin', windows: { track: { firstOffset: 60, lastOffset: 60 } } });
});
