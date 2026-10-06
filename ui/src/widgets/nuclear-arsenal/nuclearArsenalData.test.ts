import assert from 'node:assert/strict';
import test from 'node:test';
import { linesTooltip } from '../../charts/race/lines.ts';
import { runAt } from '../../charts/series.ts';
import { countryRaceItems } from '../../charts/race/countryRace.ts';
import { stepRace } from '../../charts/race/race.ts';
import type { CountryNukesReply } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveCountries } from '../../dashboard/groups.ts';
import { OPTIONS, definition } from './definition.ts';
import { formatNukes, nuclearModel, nuclearRuns, nuclearSegmentAt, nuclearTooltip, nukesRequest, rankNukes, validateNukesReply } from './nuclearArsenalData.ts';

const reply = (): CountryNukesReply => ({ branchId: '1', readHours: [10, 11, 12, 14], emptyReadHours: [], warnings: [], types: [
  { ordinal: 0, token: 'mod_singularity_stockpile', name: 'Singularity bombs', supported: true },
  { ordinal: 1, token: 'mod_stellar_stockpile', name: 'Stellar bombs', supported: true },
], series: [{ tag: 'AAA', segments: [{ firstHour: 10, lastHour: 14 }], membership: null, runs: [
  { firstHour: 10, lastHour: 11, startsSegment: true, counts: [{ ordinal: 0, value: 2 }, { ordinal: 1, value: 3 }], total: 5 },
  { firstHour: 12, lastHour: 12, startsSegment: false, counts: [{ ordinal: 1, value: 3 }], total: null },
  { firstHour: 14, lastHour: 14, startsSegment: true, counts: [{ ordinal: 0, value: 4 }, { ordinal: 1, value: 3 }], total: 7 },
] }] });

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
  assert.equal(at(10).rows[0].icon, 'old-flag');
  assert.equal(at(14).rows[0].label, 'After rename');
  assert.equal(at(14).rows[0].icon, 'new-flag');
});

test('nuclear defaults and requests reuse race options and top countries', () => {
  assert.equal(OPTIONS.find(field => field.key === 'view')!.default, 'race');
  assert.deepEqual(definition.defaultCountries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
  assert.deepEqual(nukesRequest('1', { tags: ['AAA'], top: 10 }), { branchId: '1', tags: [], top: 10 });
  assert.deepEqual(nukesRequest('1', { tags: ['AAA'] }), { branchId: '1', tags: ['AAA'], top: null });
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
