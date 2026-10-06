import test from 'node:test';
import assert from 'node:assert/strict';
import type { CasualtyReply, CasualtySeries } from '../../generated/dashboard';
import { casualtiesAt, casualtyIndex, casualtyLookSegments, casualtyRaceEntries, casualtyRequest, casualtyRatio, casualtyTooltip, formatCasualties, selectCasualties, validateCasualtyReply } from './casualtiesData.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import { OPTIONS } from './definition.ts';
import { HOUR_EPOCH, formatDateTime } from '../../dashboard/calendar.ts';
import '../../widgets/definitions.ts';
import type { BoundTrack } from '../../dashboard/model';

const sample = (takenRaw = '20', dealtRaw = '60', unknownRaw: string | null = '5', firstHour = 119, lastHour = 144): Pick<CasualtySeries,
  'reads' | 'ends' | 'dealt' | 'taken' | 'unknown' | 'unknownUntil' | 'atLeastFrom'> => ({
    reads: [firstHour], ends: [[0, lastHour, lastHour]], dealt: [dealtRaw, []], taken: [takenRaw, []],
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
  assert.equal(a.readHour, 119);
  assert.equal(casualtiesAt(index, 119, false)[0].readHour, 119);
});
test('unknown adds once, only to taken; unavailable aggregate never becomes zero', () => {
  const reply = fixture();
  assert.equal(casualtiesAt(casualtyIndex(reply), 125, true)[0].taken, 25n);
  reply.series[0].unknown = null;
  const missing = casualtiesAt(casualtyIndex(reply), 125, true)[0];
  assert.equal(missing.taken, null); assert.equal(missing.dealt, 60n);
  assert.equal(casualtiesAt(casualtyIndex(reply), 125, false)[0].taken, 20n);
  const tip = casualtyTooltip(missing, 'mod name', true, reply.unknownLabel);
  assert(tip.note?.includes('across land, sea and air'));
  assert(tip.note?.includes('cannot be recovered'));
});

test('a lost war displays exact lower bounds, no ratio, and a plain explanation', () => {
  const reply = fixture(); reply.series[0].atLeastFrom = 119;
  const row = casualtiesAt(casualtyIndex(validateCasualtyReply(reply)), 125, false)[0];
  assert.equal(formatCasualties(1234567n, row.atLeast), '≥ 1,234,567');
  assert.equal(casualtyRatio(row), '—');
  const tooltip = casualtyTooltip(row, 'Full country name', false, reply.unknownLabel);
  assert.equal(tooltip.rows[0].value, '≥ 60');
  assert.equal(tooltip.rows[2].label, 'Mod unknown losses');
  assert(tooltip.note?.includes('At least: a war ended while the recording was not running, so its last losses are missing.'));
});

test('including unknown losses never changes known-total ordering', () => {
  const reply = fixture(); reply.series[0].dealt[0] = '1'; reply.series[0].unknown![0] = '1000';
  reply.series[1].unknown = null;
  for (const includeUnknown of [false, true]) {
    const rows = selectCasualties(casualtiesAt(casualtyIndex(reply), 125, includeUnknown), { kind: 'all' }, undefined, 125, 144);
    assert.deepEqual(rows.map(row => row.tag), ['mod b|', 'mod c|', 'mod a|']);
  }
});

test('taken race uses the same reply, ranks known taken, and keeps exact lower bounds and unknown apart', () => {
  const reply = fixture();
  reply.series[0].taken[0] = '18446744073709551614'; reply.series[0].atLeastFrom = 119;
  reply.series[1].taken[0] = '10'; reply.series[1].unknown = ['1000', []];
  reply.series[2].taken[0] = '15'; reply.series[2].unknown = null;
  const index = casualtyIndex(validateCasualtyReply(reply));
  const known = casualtyRaceEntries(casualtiesAt(index, 125, false));
  assert.deepEqual(known.map(entry => entry.tag), ['mod a|', 'mod c|', 'mod b|']);
  assert.equal(known[0].label, '≥ 18,446,744,073,709,551,614');
  const snapshot = casualtiesAt(index, 125, true), inclusive = casualtyRaceEntries(snapshot);
  assert.deepEqual(inclusive.map(entry => entry.tag), ['mod a|', 'mod b|']);
  assert.equal(inclusive[1].label, '1,010');
  assert.equal(snapshot.find(row => row.tag === 'mod c|')?.taken, null);
  const tip = casualtyTooltip(snapshot[1], 'sampled country name', true, reply.unknownLabel, true);
  assert.equal(tip.rows[1].value, '10'); assert.equal(tip.rows[2].value, '1,000');
  assert(tip.note?.includes('Wars fought entirely while the recording was not running are not counted.'));
});

test('a clipped top membership keeps its earlier sample date without duplicated fields', () => {
  const reply = fixture(); const series = reply.series[0];
  series.membership = [{ firstHour: 125, lastHour: 144 }];
  const row = casualtiesAt(casualtyIndex(validateCasualtyReply(reply)), 130, false).find(row => row.tag === series.tag)!;
  assert.equal(row.readHour, 119); assert.equal(row.taken, 20n);
  assert.equal(casualtiesAt(casualtyIndex(reply), 124, false).some(row => row.tag === series.tag), false);
});
test('daily tooltip dates both a held value and the exact sample hour with kit read dating', () => {
  const index = casualtyIndex(fixture());
  for (const hour of [119, 125]) {
    const base = HOUR_EPOCH + 1943 * 365 * 24;
    const row = { ...casualtiesAt(index, hour, false)[0], hour: base + hour, readHour: base + 119 };
    const tip = casualtyTooltip(row, 'name', false, fixture().unknownLabel);
    assert.match(tip.note!, /^As of /);
    assert(tip.note?.includes('casualties are sampled once a day.'));
    assert(tip.note?.includes(formatDateTime(base + 119)));
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
  const groups = { branchId: '1', humans: [], factions: [], wars: [{ first: 'mod a|', second: 'mod b|', firstHour: 100, lastHour: 126 }], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] } };
  const selection = { kind: 'group', group: { type: 'war', tag: 'mod a|' }, membership: 'hour', aggregate: false } as const;
  assert.deepEqual(casualtyRequest('1', selection, null, groups, 144).params.tags, ['mod b|']);
  assert.equal(selectCasualties(rows, selection, groups, 125, 144).length, 1);
  assert.equal(selectCasualties(rows, selection, groups, 127, 144).length, 0);
  assert.deepEqual(selectCasualties(rows, { kind: 'tags', tags: ['mod c|'] }, undefined, 125, 144).map(row => row.tag), ['mod c|']);
});
test('manual, all, and smart groups keep more than thirty rows; top uses its chosen N', () => {
  const base = casualtiesAt(casualtyIndex(fixture()), 125, false)[0];
  const rows = Array.from({ length: 5000 }, (_, i) => ({ ...base, id: String(i), tag: `mod ${i}|`, dealt: BigInt(i) }));
  const tags = rows.map(row => row.tag);
  assert.equal(selectCasualties(rows, { kind: 'all' }, undefined, 125, 144).length, 5000);
  assert.equal(selectCasualties(rows, { kind: 'tags', tags }, undefined, 125, 144).length, 5000);
  const top = { kind: 'group', group: { type: 'top', count: 300 }, membership: 'hour', aggregate: false } as const;
  assert.equal(selectCasualties(rows, top, undefined, 125, 144).length, 300);
  const groups = { branchId: '1', humans: tags.map(tag => ({ tag, firstHour: 100, lastHour: 144 })), factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [], warnings: [], unobserved: { humans: [], factions: [], wars: [] } };
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
test('5000 ever-top countries resolve only ten current rows', () => {
  const reply = fixture(); reply.observed = [{ firstHour: 0, lastHour: 12023 }];
  reply.series = Array.from({ length: 5000 }, (_, i): CasualtySeries => {
    const firstHour = Math.floor(i / 10) * 24 + 23, lastHour = firstHour + 23;
    return { tag: `mod ${i}| country`, ...sample(String(i), String(i * 2), '0', firstHour, lastHour), ends: [[0, lastHour, null]], labels: [`mod ${i}| country`], identities: [{ firstHour, id: `country ${i}` }], membership: [{ firstHour, lastHour }] };
  });
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert.equal(casualtiesAt(index, 22, false).length, 0);
  assert.equal(casualtiesAt(index, 12022, false).length, 10);
  assert.equal(casualtiesAt(index, 12023, false).length, 0);
  index.membership.comparisons = 0;
  for (let i = 0; i < 30; i++) {
    const rows = selectCasualties(casualtiesAt(index, 6023, false), { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false }, undefined, 6023, 12023);
    assert.equal(rows.length, 10);
  }
  assert(index.membership.comparisons < 30 * 30, 'lookups walk the interval index rather than all ever-top countries');
});

test('columnar values accumulate in BigInt and equal-value runs are confirmed at every day end', () => {
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
  assert.equal(casualtiesAt(held, 170, false)[0].readHour, 167);
  assert.equal(casualtiesAt(held, 191, false)[0].readHour, 191);
});
test('columnar payload refuses unsafe number deltas and missing final endpoints', () => {
  const reply = fixture(); reply.series[0].reads.push(24);
  reply.series[0].dealt[1].push(Number.MAX_SAFE_INTEGER + 1); reply.series[0].taken[1].push(0);
  assert.throws(() => validateCasualtyReply(reply));
  const noEnd = fixture(); noEnd.series[0].ends = []; assert.throws(() => validateCasualtyReply(noEnd));
});
test('a branch gap is explained even for a country without a lower-bound flag', () => {
  const row = casualtiesAt(casualtyIndex(fixture()), 125, false)[0];
  assert(casualtyTooltip(row, 'name', false, 'Unknown', true).note?.includes('Wars fought entirely while the recording was not running are not counted.'));
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

test('top membership holds through hour 22 without inventing a confirming read', () => {
  const reply = fixture(); reply.series = [reply.series[0]];
  reply.observed = [{ firstHour: 0, lastHour: 23 }];
  const series = reply.series[0];
  series.reads = [0]; series.ends = [[0, 22, null]];
  series.identities[0].firstHour = 0;
  series.membership = [{ firstHour: 0, lastHour: 22 }];
  const index = casualtyIndex(validateCasualtyReply(reply));
  assert.equal(casualtiesAt(index, 22, false)[0].readHour, 0);
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
