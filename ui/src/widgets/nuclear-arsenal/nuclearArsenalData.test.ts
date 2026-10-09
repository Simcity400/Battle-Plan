import assert from 'node:assert/strict';
import test from 'node:test';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { linesTooltip } from '../../charts/race/lines.ts';
import { runAt } from '../../charts/series.ts';
import { countryRaceItems } from '../../charts/race/countryRace.ts';
import { stepRace } from '../../charts/race/race.ts';
import type { CountryNukesReply } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveCountries } from '../../dashboard/groups.ts';
import { registerWidgets } from '../../dashboard/definitions.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options.ts';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState.ts';
import { OPTIONS, definition } from './definition.ts';
import { formatNukes, nuclearDrillAliasAt, nuclearDrillSelection, nuclearIdentityModel, nuclearNumbers, pinNuclearDrill, reduceNuclearDrill, validateNuclearDrillReply, nuclearDrillAt, nuclearDrillBands, nuclearModel, nuclearRuns, nuclearSegmentAt, nuclearTooltip, nukesAt, nukesRequest, rankNukes, validateNukesReply } from './nuclearArsenalData.ts';

const reply = (): CountryNukesReply => ({ branchId: '1', readHours: [10, 11, 12, 14], emptyReadHours: [], warnings: [], types: [
  { ordinal: 0, token: 'mod_singularity_stockpile', name: 'Singularity bombs', supported: true },
  { ordinal: 1, token: 'mod_stellar_stockpile', name: 'Stellar bombs', supported: true },
], series: [{ tag: 'AAA', segments: [{ firstHour: 10, lastHour: 14 }], membership: null, runs: [
  { firstHour: 10, lastHour: 11, startsSegment: true, counts: [{ ordinal: 0, value: 2 }, { ordinal: 1, value: 3 }], total: 5 },
  { firstHour: 12, lastHour: 12, startsSegment: false, counts: [{ ordinal: 1, value: 3 }], total: null },
  { firstHour: 14, lastHour: 14, startsSegment: true, counts: [{ ordinal: 0, value: 4 }, { ordinal: 1, value: 3 }], total: 7 },
] }] });

test('nuclear drill exposes the shared controls and pins the selected country and branch-relative day', () => {
  registerWidgets([definition]);
  const defaults = resolveOptions(OPTIONS, undefined);
  assert.equal(defaults.show, 'main');
  assert.equal(defaults.drill, 'inline');
  const source = { id: 'source', kind: definition.kind, configVersion: 1, title: 'Nuclear arsenal', campaign: 'tracks', trackIds: ['track'],
    countries: definition.defaultCountries, time: { mode: 'follow' }, contentScale: 125, options: { drill: 'popout' } } satisfies WidgetSpec;
  const track = { id: 'track', branch: { firstHour: 1000 } } as Pick<BoundTrack, 'id' | 'branch'>;
  const pinned = pinNuclearDrill(source, track, 'later alias with spaces', 1030.9, 1010, false);
  assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['later alias with spaces'] });
  assert.deepEqual(pinned.time, { mode: 'pin', windows: { track: { firstOffset: 30, lastOffset: 30 } } });
  assert.equal(resolveOptions(OPTIONS, pinned.options).show, 'breakdown');
  assert.equal(resolveOptions(OPTIONS, pinned.options).drillOriginHour, 1010);
  const following = pinNuclearDrill(pinned, track, 'later alias with spaces', 1040, 1010, true);
  assert.deepEqual(following.time, { mode: 'follow' });
  assert.equal(resolveOptions(OPTIONS, following.options).drillOriginHour, 1010);
  const opened = reduceDrill(CLOSED, { type: 'open', breakdown: 'later alias with spaces', hour: 1030, playing: true, playhead: 1040 });
  const held = reduceDrill(opened, { type: 'pick', hour: 1020 });
  assert.equal(drillHour(held, 1050), 1020);
  assert.equal(drillHour(reduceDrill(held, { type: 'follow' }), 1050), 1050);
  assert.equal(OPTIONS.find(field => field.key === 'playback')!.visibleWhen!(resolveOptions(OPTIONS, { show: 'breakdown', view: 'race' })), true);
});

test('nuclear full drill keeps missing types unknown, exact signed wrapping differences and gaps', () => {
  const data = reply();
  data.series[0].runs[0] = { ...data.series[0].runs[0], counts: [{ ordinal: 0, value: 2 }], total: 5 };
  const model = nuclearModel(validateNukesReply(data));
  assert.deepEqual(nuclearDrillAt(model, 'AAA', 10), { total: 5, values: [2, null], remainder: 3 });
  assert.deepEqual(nuclearDrillAt(model, 'AAA', 12), { total: null, values: [null, 3], remainder: null });
  const bands = nuclearDrillBands(model, 'AAA', null, { firstHour: 10, lastHour: 14 });
  assert.deepEqual(bands.map(runs => runAt(runs, 10)?.value), [2, 0, 3]);
  assert.ok(bands.every(runs => runAt(runs, 12) == null && runAt(runs, 13) == null));
  assert.ok(bands.every(runs => runAt(runs, 14)?.breakBefore));
  assert.equal(nuclearDrillAt(model, 'ABSENT', 10), null);
  data.series[0].runs = [{ firstHour: 10, lastHour: 10, startsSegment: true,
    counts: [{ ordinal: 0, value: 2147483647 }, { ordinal: 1, value: 1 }], total: -2147483648 }];
  const wrapped = nuclearModel(validateNukesReply(data));
  assert.equal(nuclearDrillAt(wrapped, 'AAA', 10)?.remainder, -4294967296);
  assert.deepEqual(nuclearDrillBands(wrapped, 'AAA', null, { firstHour: 10, lastHour: 10 }).map(runs => runs[0].value), [2147483647, 1, -4294967296]);
});

test('nuclear country choices retain every one of 5000 recorded tags', () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD ${index}`);
  const resolved = resolveCountries({ kind: 'tags', tags }, [], undefined, 100, definition.countries.max);
  assert.deepEqual(resolved.tags, tags);
  assert.equal(resolved.dropped, 0);
});

test('nuclear arsenal uses content type names, totals and hourly read notes', () => {
  const data = validateNukesReply(reply());
  const model = nuclearModel(data);
  assert.equal(model.parts[0].label, 'Singularity bombs');
  assert.deepEqual(rankNukes(model, ['AAA', 'ABSENT'], 10, 10), [{ tag: 'AAA', value: 5, label: '5', stretch: 10,
    parts: [{ id: '0', value: 2 }, { id: '1', value: 3 }] }]);
  const tip = nuclearTooltip(model, 'AAA', 10, 'Name at read', new Map(), 'Hour 10')![0];
  assert.equal(tip.title, 'Name at read');
  assert.equal(tip.rows[0].label, 'Singularity bombs');
  assert.ok(tip.note!.includes(`Read ${formatDateTime(10)}`));
  assert.match(tip.note!, /queued drops/);
});

test('nuclear carried values and clipped runs retain the supplying read date', () => {
  const data = reply(); data.readHours = [10, 14];
  data.series[0].runs = [{ firstHour: 11, lastHour: 12, startsSegment: false, counts: [{ ordinal: 0, value: 2 }], total: 2 }];
  const model = nuclearModel(validateNukesReply(data));
  const tip = nuclearTooltip(model, 'AAA', 12, 'Country', new Map(), formatDateTime(12))![0];
  assert.equal(tip.subtitle, formatDateTime(12));
  assert.ok(tip.note!.includes(`Read ${formatDateTime(10)}`));
  assert.ok(!tip.note!.includes(`Read ${formatDateTime(12)}`));
});

test('nuclear missing rows and country gaps never become zero or connect lines', () => {
  const model = nuclearModel(reply());
  assert.equal(rankNukes(model, ['AAA'], 12, 10).length, 0);
  assert.equal(rankNukes(model, ['AAA'], 13, 10).length, 0);
  const runs = nuclearRuns(model, 'AAA', null, { firstHour: 10, lastHour: 14 });
  assert.equal(runAt(runs, 12), null);
  assert.equal(runAt(runs, 13), null);
  assert.equal(runAt(nuclearRuns(model, 'AAA', null, { firstHour: 10, lastHour: 14 }, 0), 12), null);
  assert.equal(runAt(runs, 14)?.breakBefore, true);
  assert.equal(nuclearTooltip(model, 'AAA', 12, 'Country', new Map(), 'Hour 12')![0].rows[0].value, 'Unknown');
  assert.equal(nuclearSegmentAt(model, 'ABSENT', 10), null);
});

test('nuclear shorter native arrays retain the game counter without inventing another type', () => {
  const data = reply();
  data.series[0].runs = [{ firstHour: 10, lastHour: 10, startsSegment: true, counts: [{ ordinal: 0, value: 2 }], total: 2 }];
  const model = nuclearModel(validateNukesReply(data));
  assert.equal(rankNukes(model, ['AAA'], 10, 10)[0].value, 2);
  assert.deepEqual(nuclearRuns(model, 'AAA', null, { firstHour: 10, lastHour: 10 }, 1), []);
  assert.equal(nuclearTooltip(model, 'AAA', 10, 'Country', new Map(), 'Hour 10')![0].rows[1].value, 'Unknown');
});

test('nuclear 5000-country input ranks all countries and excludes unknown totals', () => {
  const data = reply();
  data.series = Array.from({ length: 5000 }, (_, index) => ({ tag: `C${index.toString().padStart(5, '0')}`, membership: null,
    segments: [{ firstHour: 10, lastHour: 10 }], runs: [{ firstHour: 10, lastHour: 10, startsSegment: true,
      counts: [{ ordinal: 0, value: index }, { ordinal: 1, value: 0 }], total: index === 4999 ? null : index }] }));
  const model = nuclearModel(validateNukesReply(data));
  const top = rankNukes(model, model.tags, 10, 10);
  assert.equal(top.length, 10);
  assert.equal(top[0].tag, 'C04998');
  assert.equal(top.at(-1)!.value, 4989);
});

test('nuclear top ranks omit zero and nonexistent countries while a chosen existing zero remains exact', () => {
  const data = reply();
  data.series = ['zero', 'absent', 'negative', 'unknown'].map(tag => ({ ...data.series[0], tag,
    segments: tag === 'absent' ? [] : [{ firstHour: 10, lastHour: 10 }], runs: [{ ...data.series[0].runs[0],
      firstHour: 10, lastHour: 10, total: tag === 'zero' ? 0 : tag === 'unknown' ? null : -2,
      counts: [{ ordinal: 0, value: tag === 'zero' ? 0 : -2 }] }] }));
  const model = nuclearModel(validateNukesReply(data));
  assert.deepEqual(rankNukes(model, model.tags, 10, 10, true).map(entry => [entry.tag, entry.value]), [['negative', -2]]);
  assert.deepEqual(rankNukes(model, ['zero', 'absent'], 10, 10).map(entry => [entry.tag, entry.value, entry.label]), [['zero', 0, '0']]);
});

test('unsupported nuclear names retain their counted ordinal segment and exact total', () => {
  const data = reply();
  data.types.push({ ordinal: 2, token: null, name: null, supported: false });
  data.series[0].runs[0].counts.push({ ordinal: 2, value: 4 });
  data.series[0].runs[0].total = 9;
  const model = nuclearModel(validateNukesReply(data));
  assert.deepEqual(model.parts.at(-1), { id: '2', label: 'Unknown type 2' });
  const ranked = rankNukes(model, ['AAA'], 10, 10)[0];
  assert.equal(ranked.parts!.reduce((sum, part) => sum + part.value, 0), ranked.value);
  assert.equal(nuclearRuns(model, 'AAA', null, { firstHour: 10, lastHour: 11 }, 2)[0].value, 4);
});

test('nuclear country names and flags follow the hovered hour across a campaign rename', () => {
  const model = nuclearModel(reply());
  const lines = [{ tag: 'AAA', runs: nuclearRuns(model, 'AAA', null, { firstHour: 10, lastHour: 14 }) }];
  const labelAt = (hour: number) => () => ({ name: hour < 14 ? 'Before rename' : 'After rename', flag: hour < 14 ? 'old-flag' : 'new-flag' });
  const at = (hour: number) => linesTooltip(hour, lines, labelAt, () => 'palette', `Hour ${hour}`, 'Stockpile', formatNukes);
  assert.equal(at(10).rows[0].label, 'Before rename');
  assert.equal(at(10).rows[0].countryFlag?.flag, 'old-flag');
  assert.equal(at(14).rows[0].label, 'After rename');
  assert.equal(at(14).rows[0].countryFlag?.flag, 'new-flag');
});

test('nuclear hover retains dated pending and refused flags beside exact stockpiles', () => {
  const model = nuclearModel(reply());
  const lines = [{ tag: 'AAA', runs: nuclearRuns(model, 'AAA', null, { firstHour: 10, lastHour: 14 }) }];
  const pending = { name: 'Before rename', flagPending: true };
  const refused = { name: 'After rename', flagPending: false, flagReason: 'Recorded flag unavailable' };
  const labelAt = (hour: number) => (tag: string) => { assert.equal(tag, 'AAA'); return hour < 14 ? pending : refused; };
  const at = (hour: number) => linesTooltip(hour, lines, labelAt, () => 'palette', `Hour ${hour}`, 'Stockpile', formatNukes).rows[0];
  assert.deepEqual(at(10).countryFlag, pending);
  assert.equal(at(10).value, '5');
  assert.deepEqual(at(14).countryFlag, refused);
  assert.equal(at(14).value, '7');
  assert.equal(at(10).icon, undefined);
  assert.equal(at(14).icon, undefined);
});

test('nuclear defaults and requests reuse race options and top countries', () => {
  assert.equal(OPTIONS.find(field => field.key === 'view')!.default, 'race');
  assert.deepEqual(definition.defaultCountries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
  assert.deepEqual(nukesRequest('1', { tags: ['AAA'], top: 10 }), { branchId: '1', tags: [], top: 10, originHour: null });
  assert.deepEqual(nukesRequest('1', { tags: ['AAA'] }), { branchId: '1', tags: ['AAA'], top: null, originHour: null });
});

test('nuclear reply validator refuses malformed, duplicated and noninteger values', () => {
  const malformed = reply(); malformed.series[0].runs[0].counts.push({ ordinal: 0, value: 1 });
  assert.throws(() => validateNukesReply(malformed), /malformed/);
  const fractional = reply(); fractional.series[0].runs[0].total = 1.5;
  assert.throws(() => validateNukesReply(fractional), /malformed/);
  const inventedEmpty = reply(); inventedEmpty.emptyReadHours = [13];
  assert.throws(() => validateNukesReply(inventedEmpty), /malformed/);
  const sparseSpans = reply(); sparseSpans.series[0].segments = new Array(1);
  assert.throws(() => validateNukesReply(sparseSpans), /malformed/);
});

test('nuclear bar easing stops across an unknown total within an existing country', () => {
  const data = reply();
  data.series[0].runs[2].firstHour = 13;
  data.series[0].runs[2].startsSegment = false;
  const model = nuclearModel(data);
  const items = (hour: number) => countryRaceItems(rankNukes(model, ['AAA'], hour, 1), new Map(), () => 'country', 'missing', false);
  let state = stepRace(null, items(11), 0, 16, 100);
  state = stepRace(state, items(13), 16, 16, 100);
  assert.equal(state.rows[0].widths.get('0')!.solid, 4);
  assert.equal(state.rows[0].item.label, '7');
});

test('nuclear drills join only the native origin aliases and pin that origin through later tag reuse', () => {
  const data = reply();
  data.series = [
    { tag: 'OLD', membership: [{ firstHour: 10, lastHour: 11 }], segments: [{ firstHour: 10, lastHour: 11 }], runs: [
      { firstHour: 10, lastHour: 11, startsSegment: true, total: 2, counts: [{ ordinal: 0, value: 2 }] }] },
    { tag: 'NEW', membership: [{ firstHour: 12, lastHour: 14 }], segments: [{ firstHour: 12, lastHour: 14 }], runs: [
      { firstHour: 12, lastHour: 14, startsSegment: true, total: 3, counts: [{ ordinal: 0, value: 3 }] }] },
  ];
  const model = nuclearIdentityModel(validateNuclearDrillReply(data), 'OLD');
  assert.equal(nuclearDrillAt(model, 'OLD', 14)?.total, 3);
  assert.equal(nukesAt(model, 'OLD', 14)?.stretch, 10);
  assert.equal(nuclearSegmentAt(model, 'OLD', 14), 10);
  const bands = nuclearDrillBands(model, 'OLD', null, { firstHour: 10, lastHour: 14 });
  assert.equal(runAt(bands[0], 12)?.breakBefore, undefined);
  assert.equal(nuclearTooltip(model, 'OLD', 12, 'New alias', new Map(), 'Hour 12')![0].note!.includes(`Read ${formatDateTime(12)}`), true);
  assert.equal(nuclearDrillAliasAt(data, 10), 'OLD');
  assert.equal(nuclearDrillAliasAt(data, 14), 'NEW');
  assert.equal(nuclearDrillAliasAt(data, 15), null);
  const held = reduceDrill(CLOSED, { type: 'open', breakdown: 'OLD', hour: 10, playing: false, playhead: null });
  assert.equal(nuclearDrillAt(model, 'OLD', drillHour(reduceDrill(held, { type: 'follow' }), 14)!)?.total, 3);
  assert.deepEqual(nukesRequest('1', { tags: ['OLD'] }, 10), { branchId: '1', tags: ['OLD'], top: null, originHour: 10 });
  data.series[1].membership![0].firstHour = 11;
  assert.throws(() => validateNuclearDrillReply(data), /conflicting/);
});

test('origin alias joins retain genuine read, identity and unknown-total gaps', () => {
  const data = reply();
  data.series = [
    { tag: 'OLD', membership: [{ firstHour: 10, lastHour: 11 }], segments: [{ firstHour: 10, lastHour: 11 }], runs: [
      { firstHour: 10, lastHour: 11, startsSegment: true, total: 2, counts: [{ ordinal: 0, value: 2 }] }] },
    { tag: 'NEW', membership: [{ firstHour: 12, lastHour: 14 }], segments: [{ firstHour: 12, lastHour: 14 }], runs: [
      { firstHour: 12, lastHour: 14, startsSegment: true, total: 3, counts: [{ ordinal: 0, value: 3 }] }] },
  ];
  data.series[0].runs[0].total = null;
  const unknown = nuclearIdentityModel(validateNuclearDrillReply(data), 'OLD');
  assert.equal(nukesAt(unknown, 'OLD', 12)?.stretch, 12);
  assert.ok(runAt(nuclearDrillBands(unknown, 'OLD', null, { firstHour: 10, lastHour: 14 })[0], 12)?.breakBefore);
  data.series[0].runs[0].total = 2;
  data.series[1].runs[0].firstHour = 13;
  const unread = nuclearIdentityModel(validateNuclearDrillReply(data), 'OLD');
  assert.equal(nukesAt(unread, 'OLD', 12), null);
  assert.equal(nukesAt(unread, 'OLD', 13)?.stretch, 13);
  assert.ok(runAt(nuclearDrillBands(unread, 'OLD', null, { firstHour: 10, lastHour: 14 })[0], 13)?.breakBefore);
  data.series[1].membership![0].firstHour = 13;
  data.series[1].segments[0].firstHour = 13;
  const absent = nuclearIdentityModel(validateNuclearDrillReply(data), 'OLD');
  assert.equal(nuclearSegmentAt(absent, 'OLD', 12), null);
  assert.equal(nuclearSegmentAt(absent, 'OLD', 13), 13);
});

test('content pending and renamed type labels preserve the same native pick owner', () => {
  const data = reply();
  const held = new NumericReply<CountryNukesReply>();
  const owner = held.update('source/branch/origin', data, nuclearNumbers);
  assert.equal(held.update('source/branch/origin', undefined, nuclearNumbers), owner);
  const relabelled = { ...data, types: data.types.map(type => ({ ...type, name: 'A changed content name' })), warnings: ['Content changed'] };
  assert.equal(held.update('source/branch/origin', relabelled, nuclearNumbers), owner);
  assert.equal(nuclearModel(owner!).series.get('AAA')!.runs[0].total, 5);
  assert.equal(held.update('different native selection', undefined, nuclearNumbers), undefined);
});

test('a pinned nuclear drill cannot restamp its old actor when countries, track or source change before effects', () => {
  const oldKey = JSON.stringify(['source', 'revision', 'track', ['OLD']]);
  const initial = { key: null, drill: CLOSED };
  const opened = reduceNuclearDrill(initial, { key: oldKey,
    action: { type: 'open', breakdown: 'OLD', hour: 10, playing: false, playhead: null } });
  const old = reduceNuclearDrill(opened, { key: oldKey, action: { type: 'follow' } });
  const oldOrigin = { key: oldKey, tag: 'OLD', hour: 10 };
  assert.equal(nuclearDrillSelection(old, oldKey, oldOrigin, true, 'OLD', 20).tag, 'OLD');
  for (const key of [
    JSON.stringify(['source', 'revision', 'track', ['NEW']]),
    JSON.stringify(['source', 'revision', 'other-track', ['NEW']]),
    JSON.stringify(['other-source', 'revision', 'track', ['NEW']]),
  ]) {
    // This is the render before the close effect. It must already request NEW.
    const changed = nuclearDrillSelection(old, key, oldOrigin, true, 'NEW', 20);
    assert.equal(changed.drill.breakdown, null);
    assert.equal(changed.origin, null);
    assert.deepEqual(nukesRequest('branch', { tags: [changed.tag!] }, changed.hour),
      { branchId: 'branch', tags: ['NEW'], top: null, originHour: 20 });
    const frozen = { key, tag: changed.tag!, hour: changed.hour };
    // Freezing that origin cannot grant the still-stale reducer a new owner.
    assert.equal(nuclearDrillSelection(old, key, frozen, true, 'NEW', 21).tag, 'NEW');
    const closed = reduceNuclearDrill(old, { key, action: { type: 'close' } });
    assert.equal(nuclearDrillSelection(closed, key, frozen, true, 'NEW', 21).hour, 20);
    const empty = nuclearDrillSelection(old, key, oldOrigin, true, null, 20);
    assert.equal(empty.tag, null);
    assert.equal(nuclearDrillSelection(old, key, oldOrigin, false, 'NEW', 20).tag, null);
    assert.equal(reduceNuclearDrill(old, { key, action: { type: 'follow' } }).drill.breakdown, null);
    const newActor = reduceNuclearDrill(old, { key,
      action: { type: 'open', breakdown: 'NEW', hour: 20, playing: false, playhead: null } });
    assert.equal(nuclearDrillSelection(newActor, key, frozen, true, 'NEW', 20).drill.breakdown, 'NEW');
  }
});
