import { dayLabel } from '../../charts/draw.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import type { CasualtyReply, CasualtySeries } from '../../generated/dashboard';
import { casualtiesAt, casualtyBarStretch, casualtyBarValue, casualtyIndex, casualtyLookSegments, casualtyRaceEntries, casualtyRequest, casualtyRatio, casualtyTooltip, formatCasualties, SUBTOTAL_NOTE, selectCasualties, validateCasualtyReply } from './casualtiesData.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';
import { countryRaceItems } from '../../charts/race/countryRace.ts';
import { barLength, raceMin, stepRace } from '../../charts/race/race.ts';
import { butterflyScale } from '../../charts/butterfly/layout.ts';
import { approachAcrossGap } from '../../charts/easing.ts';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import '../../widgets/definitions.ts';
import type { BoundTrack } from '../../dashboard/model';

const sample = (takenRaw = '20', dealtRaw = '60', unknownRaw: string | null = '5', firstHour = 119, lastHour = 144): Pick<CasualtySeries,
  'reads' | 'ends' | 'dealt' | 'taken' | 'unknown' | 'unknownUntil' | 'atLeastFrom'> => ({
    reads: [firstHour], ends: [[0, lastHour, null]], dealt: [dealtRaw, []], taken: [takenRaw, []],
    unknown: unknownRaw === null ? null : [unknownRaw, []], unknownUntil: null, atLeastFrom: null,
  });
function fixture(): CasualtyReply {
  return { branchId: '1', warnings: [], unknownLabel: 'Mod unknown losses', observed: [{ firstHour: 100, lastHour: 144 }], hasGap: false,
    series: ['a', 'b', 'c'].map(tag => ({ tag: `mod ${tag}|`, ...sample(), identities: [{ firstHour: 119, id: 'a' }], labels: ['Full campaign name'], membership: null })) };
}

test('switching Views retains the unknown setting and hides butterfly-only controls in the race', () => {
  const stored = { view: 'race', includeUnknown: true, ratio: false, values: false };
  const race = resolveOptions(OPTIONS, stored);
  assert.deepEqual(sectionFields(OPTIONS, 'options', race).map(field => field.key), ['view', 'includeUnknown']);
  const butterfly = resolveOptions(OPTIONS, { ...stored, view: 'butterfly' });
  assert.deepEqual(sectionFields(OPTIONS, 'options', butterfly).map(field => field.key), ['view', 'includeUnknown', 'values', 'ratio']);
  assert.equal(butterfly.includeUnknown, race.includeUnknown);
  assert.equal(butterfly.ratio, false); assert.equal(butterfly.values, false);
});

test('country totals are looked up directly and points never backfill earlier hours', () => {
  const index = casualtyIndex(validateCasualtyReply(fixture()));
  assert.equal(casualtiesAt(index, 118, false).length, 0);
  const a = casualtiesAt(index, 125, false)[0];
  assert.equal(a.taken, 20n); assert.equal(a.dealt, 60n); assert.equal(a.unknown, 5n);
  assert.equal(a.readHour, 125);
  assert.equal(casualtiesAt(index, 119, false)[0].readHour, 119);
});
test('unknown adds once, only to taken; unavailable aggregate never becomes zero', () => {
  const reply = fixture();
  assert.equal(casualtiesAt(casualtyIndex(reply), 125, true)[0].taken, 25n);
  reply.series[0].unknown = null;
  const missing = casualtiesAt(casualtyIndex(reply), 125, true)[0];
  assert.equal(missing.taken, null); assert.equal(missing.dealt, 60n);
  assert.equal(casualtiesAt(casualtyIndex(reply), 125, false)[0].taken, 20n);
  const tip = casualtyTooltip(missing, 'mod name', reply.unknownLabel);
  assert.doesNotMatch(tip.note ?? '', /across land, sea and air|hourly reading|As of/);
  assert.equal(tip.note, 'Losses from unknown attackers are unavailable.');
});

test('a lost war marks an exact recorded subtotal without claiming a lower bound', () => {
  const reply = fixture(); reply.series[0].atLeastFrom = 119;
  const row = casualtiesAt(casualtyIndex(validateCasualtyReply(reply)), 125, false)[0];
  assert.equal(formatCasualties(1234567n, row.atLeast), '† 1,234,567');
  assert.equal(casualtyRatio(row), '—');
  const tooltip = casualtyTooltip(row, 'Full country name', reply.unknownLabel);
  assert.equal(tooltip.rows[0].value, '† 60');
  assert.equal(tooltip.rows[2].label, 'Mod unknown losses');
  assert(tooltip.note?.includes(SUBTOTAL_NOTE));
  assert.equal(formatCasualties(-1234567n, row.atLeast), '† −1,234,567');
  assert.equal(formatCasualties(null, row.atLeast), '—');
});

test('including unknown losses never changes known-total ordering', () => {
  const reply = fixture(); reply.series[0].dealt[0] = '1'; reply.series[0].unknown![0] = '1000';
  reply.series[1].unknown = null;
  for (const includeUnknown of [false, true]) {
    const rows = selectCasualties(casualtiesAt(casualtyIndex(reply), 125, includeUnknown), { kind: 'all' }, undefined, 125, 144);
    assert.deepEqual(rows.map(row => row.tag), ['mod b|', 'mod c|', 'mod a|']);
  }
});

test('taken race uses the same reply, ranks known taken, and keeps exact subtotals and unknown apart', () => {
  const reply = fixture();
  reply.series[0].taken[0] = '18446744073709551614'; reply.series[0].atLeastFrom = 119;
  reply.series[1].taken[0] = '10'; reply.series[1].unknown = ['1000', []];
  reply.series[2].taken[0] = '15'; reply.series[2].unknown = null;
  const index = casualtyIndex(validateCasualtyReply(reply));
  const known = casualtyRaceEntries(casualtiesAt(index, 125, false));
  assert.deepEqual(known.map(entry => entry.tag), ['mod a|', 'mod c|', 'mod b|']);
  assert.equal(known[0].label, '† 18,446,744,073,709,551,614');
  const snapshot = casualtiesAt(index, 125, true), inclusive = casualtyRaceEntries(snapshot);
  assert.deepEqual(inclusive.map(entry => entry.tag), ['mod a|', 'mod b|']);
  assert.equal(inclusive[1].label, '1,010');
  assert.equal(snapshot.find(row => row.tag === 'mod c|')?.taken, null);
  const tip = casualtyTooltip(snapshot[1], 'sampled country name', reply.unknownLabel, true);
  assert.equal(tip.rows[1].value, '10'); assert.equal(tip.rows[2].value, '1,000');
  assert(tip.note?.includes('Some losses may be missing.'));
});

test('a wide native correction keeps its signed row, ranking and read while bars stay positive-only', () => {
  const reply = fixture(), series = reply.series[0];
  series.reads = [119, 24]; series.ends = [[1, 144, 144]];
  series.dealt = ['60', [-62]];
  series.taken = ['18446744073709551614', ['-36893488147419103228']];
  series.unknown = ['5', [0]]; series.atLeastFrom = 143;
  const index = casualtyIndex(validateCasualtyReply(reply));
  const before = casualtiesAt(index, 142, false)[0], rows = casualtiesAt(index, 143, false), corrected = rows[0];
  assert.equal(before.taken, 18446744073709551614n); assert.equal(before.atLeast, false);
  assert.equal(corrected.taken, -18446744073709551614n); assert.equal(corrected.dealt, -2n);
  assert.equal(corrected.id, before.id); assert.equal(corrected.segment, before.segment);
  assert.equal(corrected.readHour, 143); assert.equal(corrected.atLeast, true);
  assert.deepEqual(selectCasualties(rows, { kind: 'all' }, undefined, 143, 144).map(row => row.tag), ['mod b|', 'mod c|', 'mod a|']);
  const ranked = casualtyRaceEntries(rows);
  assert.deepEqual(ranked.map(entry => entry.tag), ['mod b|', 'mod c|', 'mod a|']);
  const race = countryRaceItems(ranked, new Map(), () => '#123456', '#abcdef', false);
  assert.equal(race.length, 3); assert.equal(race[2].label, '† −18,446,744,073,709,551,614');
  assert.equal(barLength(race[2]), 0); assert.equal(raceMin(race), 0);
  assert.equal(race[0].segments[0].color, '#123456');
  const initial = countryRaceItems(casualtyRaceEntries(casualtiesAt(index, 142, false)), new Map(), () => '#123456', '#abcdef', false);
  const state = stepRace(stepRace(null, initial, 0, 16, 100), race, 16, 16, 100);
  assert.equal(state.rows.find(row => row.key === corrected.tag)?.widths.get('total')?.solid, 0);
  assert.notEqual(casualtyBarStretch(before), casualtyBarStretch(corrected));
  assert.equal(casualtyBarStretch({ ...before, taken: 40n }), casualtyBarStretch(before));
  assert.notEqual(casualtyBarStretch({ ...before, taken: 0n }), casualtyBarStretch(before));
  assert.equal(approachAcrossGap(casualtyBarValue(before.taken), casualtyBarValue(corrected.taken), 16, 100,
    casualtyBarStretch(before) !== casualtyBarStretch(corrected)), 0);
  const butterfly = rows.map(row => ({ id: row.id, left: casualtyBarValue(row.taken), right: casualtyBarValue(row.dealt) }));
  assert.deepEqual(butterfly[0], { id: corrected.id, left: 0, right: 0 });
  assert.equal(butterflyScale(butterfly), 63);
  assert.equal(casualtyBarValue(null), null); assert.equal(casualtyBarValue(0n), 0);
  assert.equal(formatCasualties(0n), '0');
  const tooltip = casualtyTooltip(corrected, 'name at supplying read', reply.unknownLabel);
  assert.equal(tooltip.rows[0].value, '† −2');
  assert.equal(tooltip.rows[1].value, '† −18,446,744,073,709,551,614');
  assert.doesNotMatch(tooltip.note ?? '', /positive amounts draw bars/);
  assert.match(tooltip.note ?? '', /Ratio unavailable for corrections/);
  assert.doesNotMatch(tooltip.note ?? '', /Read|reading|Hour 143/);
});

test('loss ratios require complete nonnegative totals while signed amounts remain exact', () => {
  const row = casualtiesAt(casualtyIndex(fixture()), 125, false)[0];
  assert.equal(casualtyRatio(row), '3.00 : 1');
  assert.equal(casualtyRatio({ ...row, taken: 0n }), '—');
  assert.equal(casualtyRatio({ ...row, taken: null }), '—');
  assert.equal(casualtyRatio({ ...row, atLeast: true }), '—');
  for (const corrected of [{ ...row, dealt: -60n }, { ...row, taken: -20n }, { ...row, dealt: -60n, taken: -20n }]) {
    assert.equal(casualtyRatio(corrected), '—');
    const tooltip = casualtyTooltip(corrected, 'country', 'Native unknown label');
    assert.equal(tooltip.rows[0].value, formatCasualties(corrected.dealt));
    assert.equal(tooltip.rows[3].value, formatCasualties(corrected.taken));
    assert(tooltip.note?.includes('Ratio unavailable for corrections.'));
  }
});

test('a clipped top membership retains hourly confirmations without duplicated fields', () => {
  const reply = fixture(); const series = reply.series[0];
  series.membership = [{ firstHour: 125, lastHour: 144 }];
  const row = casualtiesAt(casualtyIndex(validateCasualtyReply(reply)), 130, false).find(row => row.tag === series.tag)!;
  assert.equal(row.readHour, 130); assert.equal(row.taken, 20n);
  assert.equal(casualtiesAt(casualtyIndex(reply), 124, false).some(row => row.tag === series.tag), false);
});
test('hourly tooltip dates both a held value and the exact reading with kit read dating', () => {
  const index = casualtyIndex(fixture());
  for (const hour of [119, 125]) {
    const base = HOUR_EPOCH + 1943 * 365 * 24;
    const row = { ...casualtiesAt(index, hour, false)[0], hour: base + hour, readHour: base + 119 };
    const tip = casualtyTooltip(row, 'name', fixture().unknownLabel);
    assert.doesNotMatch(tip.note ?? '', /As of|reading|Read/);
    assert.equal(tip.subtitle, dayLabel(base + hour));
  }
});
test('gaps never carry points; identity changes break motion without inheriting totals', () => {
  const reply = fixture();
  reply.observed = [{ firstHour: 100, lastHour: 120 }, { firstHour: 130, lastHour: 144 }];
  reply.hasGap = true;
  for (const series of reply.series) {
    series.reads = [119, 24]; series.ends = [[0, 120, 120], [1, 144, 144]];
    series.dealt = ['60', [-56]]; series.taken = ['20', [-17]]; series.unknownUntil = 143;
  }
  for (const series of reply.series) series.identities.push({ firstHour: 143, id: 'new' });
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert(casualtiesAt(index, 125, false).every(row => row.taken == null && row.dealt == null && row.readHour == null));
  assert.deepEqual(casualtyRaceEntries(casualtiesAt(index, 125, false)), []);
  assert.equal(casualtiesAt(index, 130, false).length, 0);
  const old = casualtiesAt(index, 119, false)[0];
  const changed = casualtiesAt(index, 143, false)[0];
  assert.notEqual(old.segment, changed.segment);
  assert.equal(changed.id, 'new'); assert.equal(changed.taken, 3n);
  assert.notEqual(casualtyRaceEntries([old])[0].stretch, casualtyRaceEntries([changed])[0].stretch);
});
test('payload rejects overlapping, future-dated, malformed and out-of-membership samples', () => {
  for (const change of [(r: CasualtyReply) => { r.series.push(r.series[0]); },
    (r: CasualtyReply) => { r.series[0].taken[0] = '1.5'; },
    (r: CasualtyReply) => { r.series[0].reads[0] = NaN; },
    (r: CasualtyReply) => { r.series[0].reads.push(0); },
    (r: CasualtyReply) => { r.series[0].membership = [{ firstHour: 145, lastHour: 144 }]; },
    (r: CasualtyReply) => { r.observed[0].lastHour = 125; }]) {
    const reply = fixture(); change(reply); assert.throws(() => validateCasualtyReply(reply));
  }
  assert.throws(() => validateCasualtyReply(null));
});
test('requests keep every chosen tag and any user-selected top N; campaign and follow use their own hours', () => {
  const widget = createWidget('casualties', 't', 'w');
  assert.deepEqual(casualtyRequest('1', widget.countries, null, undefined, 144).params,
    { branchId: '1', tags: [], top: { count: 10 } });
  assert.deepEqual(casualtyRequest('1', { kind: 'selected' }, 'mod a|', undefined, 144).params,
    { branchId: '1', tags: ['mod a|'], top: null });
  const picked = casualtyRequest('1', { kind: 'tags', tags: Array.from({ length: 5000 }, (_, i) => `mod ${i}| country`) }, null, undefined, 144);
  assert.equal(picked.params.tags.length, 5000);
  assert.deepEqual(casualtyRequest('1', { kind: 'all' }, null, undefined, 144).params, { branchId: '1', tags: [], top: null });
  assert.equal(casualtyRequest('1', { kind: 'group', group: { type: 'top', count: 5000 }, membership: 'hour', aggregate: false }, null, undefined, 144).params.top?.count, 5000);
  const track = { id: 't', branch: { firstHour: 100, lastHour: 144 }, axisOffset: 0 } as BoundTrack;
  assert.equal(resolveWindow(widget, track, [track], 125)?.lastHour, 144);
  assert.equal(resolveWindow({ ...widget, time: { mode: 'follow' } }, track, [track], 125)?.lastHour, 125);
});
test('top membership and recorded war groups keep opaque tags and local hour/end/ever rules', () => {
  const reply = fixture();
  reply.series[0].membership = [{ firstHour: 119, lastHour: 120 }]; reply.series[0].ends[0] = [0, 120, 120];
  assert.equal(casualtiesAt(casualtyIndex(reply), 125, false).length, 2);
  const rows = casualtiesAt(casualtyIndex(fixture()), 125, false);
  const groups = { branchId: '1', humans: [], factions: [], wars: [{ first: 'mod a|', second: 'mod b|', firstHour: 100, lastHour: 126 }], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] },
    countryIdentities: ['mod a|', 'mod b|'].map((tag, index) => ({ tag, identities: [{ firstHour: 100, lastHour: 126, slot: index + 1, countryId: index + 10 }] })) };
  const selection = { kind: 'group', group: { type: 'war', tag: 'mod a|' }, membership: 'hour', aggregate: false } as const;
  assert.deepEqual(casualtyRequest('1', selection, null, groups, 144).params.tags, ['mod b|']);
  assert.equal(selectCasualties(rows, selection, groups, 125, 144).length, 1);
  assert.equal(selectCasualties(rows, selection, groups, 127, 144).length, 0);
  const picked = fixture(); picked.series = [picked.series[2]];
  assert.deepEqual(selectCasualties(casualtiesAt(casualtyIndex(validateCasualtyReply(picked)), 125, false),
    { kind: 'tags', tags: ['mod c|'] }, undefined, 125, 144).map(row => row.tag), ['mod c|']);
});

test('manual casualty picks retain native-holder alias projections and exact signed rows without backfilling gaps', () => {
  const reply: CasualtyReply = { ...fixture(), hasGap: true,
    observed: [{ firstHour: 100, lastHour: 102 }, { firstHour: 104, lastHour: 105 }], series: [
      { tag: 'renamed | country', ...sample('12', '-18446744073709551614', '0', 101, 101),
        identities: [{ firstHour: 101, id: '1:10' }], labels: ['Renamed country'], membership: [{ firstHour: 101, lastHour: 101 }] },
      { tag: 'renamed again', ...sample('-2', '5', null, 104, 104),
        identities: [{ firstHour: 104, id: '1:10' }], labels: ['Renamed again'], membership: null },
    ] };
  const index = casualtyIndex(validateCasualtyReply(reply));
  const selection = { kind: 'tags' as const, tags: ['peaceful original'] };
  const selectedAt = (hour: number) => selectCasualties(casualtiesAt(index, hour, false), selection, undefined, hour, 105);
  assert.equal(selectedAt(100).length, 0);
  const first = selectedAt(101)[0];
  assert.equal(first.tag, 'renamed | country'); assert.equal(first.id, '1:10');
  assert.equal(first.readHour, 101); assert.equal(first.dealt, -18446744073709551614n);
  assert.equal(casualtyBarValue(first.dealt), 0);
  assert.equal(selectedAt(102).length, 0);
  const gap = selectedAt(103);
  assert.equal(gap.length, 1); assert.equal(gap[0].tag, first.tag); assert.equal(gap[0].id, first.id);
  assert.equal(gap[0].dealt, null); assert.equal(gap[0].taken, null); assert.equal(gap[0].readHour, null);
  const later = selectedAt(104)[0];
  assert.equal(later.tag, 'renamed again'); assert.equal(later.id, first.id);
  assert.equal(later.readHour, 104); assert.equal(later.taken, -2n); assert.equal(later.unknown, null);
  assert.notEqual(later.segment, first.segment);
  assert.equal(selectedAt(105).length, 0);
  const groups = { branchId: '1', humans: [{ tag: 'peaceful original', firstHour: 100, lastHour: 105 }],
    countryIdentities: [{ tag: 'peaceful original', identities: [{ firstHour: 100, lastHour: 100, slot: 1, countryId: 10 }] },
      { tag: 'renamed | country', identities: [{ firstHour: 101, lastHour: 101, slot: 1, countryId: 10 }] },
      { tag: 'renamed again', identities: [{ firstHour: 104, lastHour: 104, slot: 1, countryId: 10 }] }],
    factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] } };
  assert.deepEqual(selectCasualties(casualtiesAt(index, 101, false),
    { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, groups, 101, 105), []);
});
test('a gap keeps one unknown row per native holder under its latest sampled alias', () => {
  const reply: CasualtyReply = { ...fixture(), hasGap: true,
    observed: [{ firstHour: 100, lastHour: 103 }, { firstHour: 106, lastHour: 107 }], series: [
      { tag: 'later alias', ...sample('-3', '18446744073709551614', '0', 102, 103),
        identities: [{ firstHour: 102, id: '1:10' }], labels: ['Later alias'], membership: null },
      { tag: 'original', ...sample('0', '0', '0', 100, 101),
        identities: [{ firstHour: 100, id: '1:10' }], labels: ['Original'], membership: null },
      { tag: 'after gap', ...sample('-4', '5', null, 106, 107),
        identities: [{ firstHour: 106, id: '1:10' }], labels: ['After gap'], membership: null },
      { tag: 'other holder', ...sample('7', '8', '0', 100, 103),
        identities: [{ firstHour: 100, id: '2:20' }], labels: ['Other holder'], membership: null },
    ] };
  const index = casualtyIndex(validateCasualtyReply(reply));
  const selection = { kind: 'tags' as const, tags: ['original', 'other holder'] };
  const selectedAt = (hour: number, includeUnknown = false) => selectCasualties(
    casualtiesAt(index, hour, includeUnknown), selection, undefined, hour, 107);
  const zero = selectedAt(100).find(row => row.id === '1:10')!;
  assert.equal(zero.tag, 'original'); assert.equal(zero.taken, 0n); assert.equal(zero.dealt, 0n);
  const signed = selectedAt(102).find(row => row.id === '1:10')!;
  assert.equal(signed.tag, 'later alias'); assert.equal(signed.taken, -3n); assert.equal(signed.dealt, 18446744073709551614n);
  for (const includeUnknown of [false, true]) {
    const gap = selectedAt(104, includeUnknown);
    assert.equal(gap.length, 2); assert.equal(new Set(gap.map(row => row.id)).size, 2);
    assert.equal(gap.find(row => row.id === '1:10')?.tag, 'later alias');
    assert(gap.every(row => row.dealt === null && row.taken === null && row.knownTaken === null && row.unknown === null && row.readHour === null));
    assert.deepEqual(casualtyRaceEntries(gap), []);
  }
  const resumed = selectedAt(106);
  assert.equal(resumed.length, 1); assert.equal(resumed[0].tag, 'after gap');
  assert.equal(resumed[0].id, signed.id); assert.equal(resumed[0].readHour, 106); assert.equal(resumed[0].taken, -4n);
});

test('a gap uses a selected holder\'s clipped latest alias without admitting its unrelated reuse', () => {
  const reply: CasualtyReply = { ...fixture(), hasGap: true,
    observed: [{ firstHour: 100, lastHour: 103 }, { firstHour: 106, lastHour: 107 }], series: [
      { tag: 'reused alias', ...sample('-3', '18446744073709551614', '0', 102, 103),
        identities: [{ firstHour: 102, id: '1:10' }], labels: ['Later alias'], membership: [{ firstHour: 102, lastHour: 103 }] },
      { tag: 'original', ...sample('0', '0', '0', 100, 101),
        identities: [{ firstHour: 100, id: '1:10' }], labels: ['Original'], membership: null },
      { tag: 'other holder', ...sample('7', '8', '0', 100, 103),
        identities: [{ firstHour: 100, id: '2:20' }], labels: ['Other holder'], membership: null },
    ] };
  const index = casualtyIndex(validateCasualtyReply(reply));
  const selection = { kind: 'tags' as const, tags: ['original', 'other holder'] };
  const selectedAt = (hour: number, includeUnknown = false) => selectCasualties(
    casualtiesAt(index, hour, includeUnknown), selection, undefined, hour, 107);
  const signed = selectedAt(103).find(row => row.id === '1:10')!;
  assert.equal(signed.tag, 'reused alias'); assert.equal(signed.readHour, 103);
  assert.equal(signed.dealt, 18446744073709551614n); assert.equal(signed.taken, -3n);
  for (const includeUnknown of [false, true]) {
    const gap = selectedAt(104, includeUnknown);
    assert.equal(gap.length, 2); assert.equal(new Set(gap.map(row => row.id)).size, 2);
    assert.equal(gap.find(row => row.id === '1:10')?.tag, 'reused alias');
    assert(gap.every(row => row.dealt === null && row.taken === null && row.knownTaken === null && row.unknown === null && row.readHour === null));
  }
  assert.deepEqual(selectedAt(106), []);
  const groups = { branchId: '1', humans: [{ tag: 'original', firstHour: 100, lastHour: 107 }], factions: [], wars: [], warStarts: [],
    countryIdentities: [{ tag: 'original', identities: [{ firstHour: 100, lastHour: 101, slot: 1, countryId: 10 }] },
      { tag: 'reused alias', identities: [{ firstHour: 102, lastHour: 103, slot: 1, countryId: 10 }] },
      { tag: 'other holder', identities: [{ firstHour: 100, lastHour: 103, slot: 2, countryId: 20 }] }],
    capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] } };
  assert.deepEqual(selectCasualties(casualtiesAt(index, 104, false),
    { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, groups, 104, 107), []);

  const clippedOnly: CasualtyReply = { ...reply, series: [reply.series[0]] };
  assert.deepEqual(casualtiesAt(casualtyIndex(validateCasualtyReply(clippedOnly)), 104, false), []);
  const top: CasualtyReply = { ...reply, series: reply.series.map(series => ({ ...series,
    membership: [{ firstHour: series.reads[0], lastHour: series.ends.at(-1)![1] }] })) };
  assert.equal(casualtiesAt(casualtyIndex(validateCasualtyReply(top)), 103, false).length, 2);
  assert.deepEqual(casualtiesAt(casualtyIndex(top), 104, false), []);
});

test('manual, all, and smart groups keep more than thirty rows; top uses its chosen N', () => {
  const base = casualtiesAt(casualtyIndex(fixture()), 125, false)[0];
  const rows = Array.from({ length: 5000 }, (_, i) => ({ ...base, id: String(i), tag: `mod ${i}|`, dealt: BigInt(i) }));
  const tags = rows.map(row => row.tag);
  assert.equal(selectCasualties(rows, { kind: 'all' }, undefined, 125, 144).length, 5000);
  assert.equal(selectCasualties(rows, { kind: 'tags', tags }, undefined, 125, 144).length, 5000);
  const top = { kind: 'group', group: { type: 'top', count: 300 }, membership: 'hour', aggregate: false } as const;
  assert.equal(selectCasualties(rows, top, undefined, 125, 144).length, 300);
  const groups = { branchId: '1', humans: tags.map(tag => ({ tag, firstHour: 100, lastHour: 144 })), factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] },
    countryIdentities: tags.map((tag, index) => ({ tag, identities: [{ firstHour: 100, lastHour: 144, slot: index + 1, countryId: index + 10 }] })) };
  const group = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  assert.equal(casualtyRequest('1', group, null, groups, 144).params.tags.length, 5000);
  assert.equal(selectCasualties(rows, group, groups, 125, 144).length, 5000);
});

test('wide and signed sums keep exact integers and labels', () => {
  const reply = fixture(); reply.series[0].taken[0] = '18446744073709551614';
  const value = casualtiesAt(casualtyIndex(validateCasualtyReply(reply)), 125, false)[0].taken;
  assert.equal(value, 18446744073709551614n);
  assert.equal(formatCasualties(value), '18,446,744,073,709,551,614');
  reply.series[0].taken[0] = '-2';
  assert.equal(formatCasualties(casualtiesAt(casualtyIndex(validateCasualtyReply(reply)), 125, false)[0].taken), '−2');
});
test('tied Unicode mod tags use native UTF-8 ordering, including astral characters', () => {
  const reply = fixture();
  reply.series[0].tag = '\u{10000}'; reply.series[1].tag = '\uE000'; reply.series[2].tag = 'aaa';
  const rows = selectCasualties(casualtiesAt(casualtyIndex(validateCasualtyReply(reply)), 125, false), { kind: 'all' }, undefined, 125, 144);
  assert.deepEqual(rows.map(row => row.tag), ['aaa', '\uE000', '\u{10000}']);
});
test('5000 hourly ever-top countries resolve only ten current rows', () => {
  const reply = fixture(); reply.observed = [{ firstHour: 0, lastHour: 500 }];
  reply.series = Array.from({ length: 5000 }, (_, i): CasualtySeries => {
    const firstHour = Math.floor(i / 10), lastHour = firstHour;
    return { tag: `mod ${i}| country`, ...sample(String(i), String(i * 2), '0', firstHour, lastHour), ends: [[0, lastHour, null]], labels: [`mod ${i}| country`], identities: [{ firstHour, id: `country ${i}` }], membership: [{ firstHour, lastHour }] };
  });
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert.equal(casualtiesAt(index, -1, false).length, 0);
  assert.equal(casualtiesAt(index, 499, false).length, 10);
  assert.equal(casualtiesAt(index, 500, false).length, 0);
  index.membership.comparisons = 0;
  for (let i = 0; i < 30; i++) {
    const rows = selectCasualties(casualtiesAt(index, 250, false), { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false }, undefined, 250, 500);
    assert.equal(rows.length, 10);
    assert(rows.every(row => row.readHour === 250));
  }
  assert(index.membership.comparisons < 30 * 30, 'lookups walk the interval index rather than all ever-top countries');
});

test('columnar values accumulate in BigInt and equal-value runs confirm every hour', () => {
  const reply = fixture();
  reply.series[0].reads = [119, 24]; reply.series[0].ends = [[1, 144, 144]];
  reply.series[0].dealt = ['18446744073709551614', [3]];
  reply.series[0].taken = ['18446744073709551614', [-2]];
  reply.series[0].unknown = ['5', [1]];
  const index = casualtyIndex(validateCasualtyReply(reply));
  const row = casualtiesAt(index, 144, false)[0];
  assert.equal(row.dealt, 18446744073709551617n); assert.equal(row.taken, 18446744073709551612n);
  assert.equal(row.readHour, 144);
  const equal = fixture(); equal.observed[0].lastHour = 200;
  for (const series of equal.series) series.ends = [[0, 200, 200]];
  const held = casualtyIndex(validateCasualtyReply(equal));
  assert.equal(casualtiesAt(held, 170, false)[0].readHour, 170);
  assert.equal(casualtiesAt(held, 191, false)[0].readHour, 191);
});

test('repeated hourly deltas retain every exact signed value and confirmation', () => {
  const reply = fixture(), wide = 18446744073709551614n;
  reply.observed = [{ firstHour: 0, lastHour: 7 }];
  reply.series = [reply.series[0]];
  const series = reply.series[0];
  series.reads = [0, 1, 1, 1, 1, 1, 1, 1]; series.ends = [[7, 7, null]];
  series.identities[0].firstHour = 0;
  series.taken = [String(wide), [[4, String(-wide)], [2, '3'], String(wide)]];
  series.dealt = ['60', [[7, '-2']]]; series.unknown = ['0', [[7, '0']]];
  const index = casualtyIndex(validateCasualtyReply(reply));
  const expected = [wide, 0n, -wide, -wide * 2n, -wide * 3n, -wide * 3n + 3n, -wide * 3n + 6n, -wide * 2n + 6n];
  for (const [hour, taken] of expected.entries()) {
    const row = casualtiesAt(index, hour, true)[0];
    assert.equal(row.taken, taken); assert.equal(row.dealt, 60n - BigInt(hour) * 2n);
    assert.equal(row.unknown, 0n); assert.equal(row.readHour, hour);
  }
  for (const repeats of [0, -1, 1.5, 8]) {
    const malformed = structuredClone(reply);
    malformed.series[0].taken[1] = [[repeats, '1']];
    assert.throws(() => validateCasualtyReply(malformed), /values/);
  }
  const missing = structuredClone(reply); missing.series[0].taken[1] = [[6, '1']];
  assert.throws(() => validateCasualtyReply(missing), /values/);
});

test('hourly rankings and the taken race retain a missing hour as a gap', () => {
  const reply = fixture(); reply.series = reply.series.slice(0, 2);
  reply.observed = [{ firstHour: 100, lastHour: 102 }, { firstHour: 104, lastHour: 105 }];
  for (const [i, series] of reply.series.entries()) {
    series.reads = [100, 1, 1, 2]; series.ends = [[2, 102, null], [3, 105, null]];
    series.identities = [{ firstHour: 100, id: `holder ${i}` }];
    series.dealt = ['0', [0, 0, 0]];
    series.taken = i ? ['0', [100, -100, 0]] : ['50', [-50, 50, 0]];
    series.unknown = ['0', [0, 0, 0]];
  }
  const index = casualtyIndex(validateCasualtyReply(reply));
  for (const hour of [100, 101, 102, 104, 105]) {
    const rows = casualtiesAt(index, hour, false), tag = hour === 101 ? 'mod b|' : 'mod a|';
    assert(rows.every(row => row.readHour === hour));
    assert.equal(selectCasualties(rows, { kind: 'all' }, undefined, hour, 105)[0].tag, tag);
    assert.equal(casualtyRaceEntries(rows)[0].tag, tag);
  }
  const gap = casualtiesAt(index, 103, false);
  assert(gap.every(row => row.readHour === null && row.taken === null));
  assert.deepEqual(casualtyRaceEntries(gap), []);
});

test('columnar payload refuses unsafe number deltas and missing final endpoints', () => {
  const reply = fixture(); reply.series[0].reads.push(24);
  reply.series[0].dealt[1].push(Number.MAX_SAFE_INTEGER + 1); reply.series[0].taken[1].push(0);
  assert.throws(() => validateCasualtyReply(reply));
  const noEnd = fixture(); noEnd.series[0].ends = []; assert.throws(() => validateCasualtyReply(noEnd));
});
test('a branch gap is explained even for a country without an incomplete-history flag', () => {
  const row = casualtiesAt(casualtyIndex(fixture()), 125, false)[0];
  assert(casualtyTooltip(row, 'name', 'Unknown', true).note?.includes('Some losses may be missing.'));
});

test('separate holders of a tag are chronological and cannot overlap', () => {
  const reply = fixture();
  const earlier = reply.series[0]; earlier.ends = [[0, 120, 120]];
  const later = { ...earlier, reads: [121], ends: [[0, 144, 144]] as [number, number, number | null][], identities: [{ firstHour: 121, id: 'new holder' }] };
  reply.series.push(later);
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert.equal(casualtiesAt(index, 120, false).filter(row => row.tag === earlier.tag).length, 1);
  assert.equal(casualtiesAt(index, 121, false).find(row => row.tag === earlier.tag)?.id, 'new holder');
  later.reads = [120]; later.identities[0].firstHour = 120;
  assert.throws(() => validateCasualtyReply(reply), /holders overlap/);
});

test('read deltas span the full recorded hour range without wrapping', () => {
  const reply = fixture(); reply.series = [reply.series[0]];
  reply.observed = [{ firstHour: -2147483648, lastHour: -2147483648 }, { firstHour: 2147483647, lastHour: 2147483647 }];
  const series = reply.series[0];
  series.reads = [-2147483648, 4294967295]; series.ends = [[0, -2147483648, null], [1, 2147483647, null]];
  series.identities[0].firstHour = -2147483648;
  series.dealt[1] = [0]; series.taken[1] = [0]; series.unknown![1] = [0];
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert.equal(casualtiesAt(index, -2147483648, false)[0].readHour, -2147483648);
  assert.equal(casualtiesAt(index, 2147483647, false)[0].readHour, 2147483647);
});

test('top membership clipping preserves confirmations from the original hourly run', () => {
  const reply = fixture(); reply.series = [reply.series[0]];
  reply.observed = [{ firstHour: 0, lastHour: 23 }];
  const series = reply.series[0];
  series.reads = [0]; series.ends = [[0, 22, null]];
  series.identities[0].firstHour = 0;
  series.membership = [{ firstHour: 0, lastHour: 22 }];
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert.equal(casualtiesAt(index, 22, false)[0].readHour, 22);
  assert.equal(casualtiesAt(index, 23, false).length, 0);
  series.ends[0][2] = 23;
  assert.throws(() => validateCasualtyReply(reply), /confirming reads/);
});

test('a non-read endpoint holds its last confirming read without dating to the endpoint', () => {
  const reply = fixture(); reply.series = [reply.series[0]];
  reply.observed = [{ firstHour: 100, lastHour: 125 }];
  const series = reply.series[0];
  series.reads = [100]; series.identities[0].firstHour = 100; series.ends = [[0, 125, 119]];
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert.equal(casualtiesAt(index, 125, false)[0].readHour, 119);
  assert.equal(casualtiesAt(index, 126, false)[0].readHour, null);
});


test('dated country looks follow holders and recorded stretches independently of top membership', () => {
  const reply = fixture(); reply.series = [reply.series[0]];
  reply.observed = [{ firstHour: 100, lastHour: 120 }, { firstHour: 130, lastHour: 144 }];
  const first = reply.series[0];
  first.reads = [100]; first.identities[0].firstHour = 100; first.ends = [[0, 120, 120]];
  first.membership = [{ firstHour: 119, lastHour: 120 }];
  const second: CasualtySeries = { ...first, reads: [130], identities: [{ firstHour: 130, id: 'second holder' }], ends: [[0, 144, 144]], membership: [{ firstHour: 143, lastHour: 144 }] };
  reply.series.push(second);
  const segmentAt = casualtyLookSegments(validateCasualtyReply(reply));
  const tag = first.tag;
  assert.equal(segmentAt(tag, 99), null);
  assert.equal(segmentAt(tag, 110), 100);
  assert.equal(segmentAt(tag, 125), null);
  assert.equal(segmentAt(tag, 132), 130);
  assert.equal(segmentAt('not recorded', 132), null);
});
