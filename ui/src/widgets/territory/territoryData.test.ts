import assert from 'node:assert/strict';
import { test } from 'node:test';
import { runAt } from '../../charts/series.ts';
import { OPTIONS, definition } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { formatTerritory, territoryCountryLines, territoryLegend, territoryNote, measureChoices, measureOf, shownTerritory, territoryModel, territoryRequest, territoryShareKeys, TERRITORY_PRESENTATION, territorySegmentAt, territoryWarnings, validateTerritoryReply, territoryColumn } from './territoryData.ts';
import type { TerritoryReply } from '../../generated/dashboard';
import { ruleOf, MEASURES } from './territoryData.ts';
import { aggregateGroupRuns, resolveCountries } from '../../dashboard/groups.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import { countryCapitulationBadges } from '../../dashboard/countryCapitulations.ts';

const reply = (): TerritoryReply => ({ branchId: '7', areaUnit: 'map cells', areaReason: null, warnings: [],
  worldTotals: [{ firstHour: 100, lastHour: 105, startsSegment: true, values: [5, 10, 250, 800, 4, 9, 200, 700, null, null, null, null] }],
  series: [{ tag: 'Renamed realm', segments: [{ firstHour: 100, lastHour: 102 }, { firstHour: 103, lastHour: 105 }], membership: null,
    runs: [{ firstHour: 100, lastHour: 100, startsSegment: true, values: [2, 5, 123, 400, 1, 4, 101, 300, null, null, null, null] },
      { firstHour: 101, lastHour: 101, startsSegment: false, values: [2, null, null, null, 1, null, null, null, null, null, null, null] },
      { firstHour: 102, lastHour: 102, startsSegment: false, values: [0, 3, -7, 90, 1, 4, 10, 100, null, null, null, null] },
      { firstHour: 103, lastHour: 105, startsSegment: true, values: [0, 3, -7, 90, 1, 4, 10, 100, null, null, null, null] }] }] });

test('Territory asks the shared race view for eased bars and smooth lines with exact hovered values', () => {
  assert.equal(TERRITORY_PRESENTATION.exact, false);
  assert.equal(TERRITORY_PRESENTATION.held, false);
  assert.equal(TERRITORY_PRESENTATION.raceTooltip, 'hover');
  const model = territoryModel(reply(), 'provinces', 'controlled');
  const runs = model.series.get('Renamed realm')!.runs;
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.value]),
    [[100, 100, 5], [102, 102, 3], [103, 105, 3]]);
  assert.equal(runAt(runs, Math.floor(100.75))?.value, 5);
  assert.equal(runAt(runs, 101), null);
  assert.equal(runAt(runs, Math.floor(102.75))?.value, 3);
  assert.equal(runAt(runs, 103)?.breakBefore, true);
});

test('Territory keeps all 5,000 recorded country choices and a user top count beyond the former 30', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const tags = Array.from({ length: 5_000 }, (_, index) => `Opaque:${index}`);
  const chosen = resolveCountries({ kind: 'tags', tags }, [], undefined, 105, definition.countries.max);
  assert.equal(chosen.tags.length, tags.length);
  assert.equal(chosen.dropped, 0);
  assert.equal(territoryRequest('7', chosen, 'states', 'controlled').tags.length, tags.length);
  const range = { firstHour: 100, lastHour: 105 };
  const data: TerritoryReply = { ...reply(), series: tags.map((tag, index) => ({ tag, membership: null,
    segments: [range], identities: [{ ...range, slot: index, countryId: index + 1 }],
    runs: [{ ...range, startsSegment: true, values: Array(12).fill(index) }] })) };
  const model = territoryModel(data, 'states', 'controlled', tags);
  assert.equal(model.selectedCount, 5_000);
  assert.equal(territoryCountryLines(model, chosen, range).length, 5_000);
  assert.equal(territoryShareKeys(model, chosen, 105).length, 5_000);
  const ranked = resolveCountries({ kind: 'group', group: { type: 'top', count: 5_000 }, membership: 'hour', aggregate: false },
    [], undefined, 105, definition.countries.max);
  assert.equal(ranked.top, 5_000);
});

test('Territory names the shared line legend by entity kind', () => {
  assert.equal(territoryLegend(false).label, 'Countries');
  assert.equal(territoryLegend(true).label, 'Factions');
});

test('Territory defaults to controlled states, the top 10, race, and no date overlay; Share is offered', () => {
  const options = resolveOptions(OPTIONS);
  assert.equal(definition.countries.aggregate, true);
  assert.equal(options.view, 'race'); assert.equal(measureOf(options.measure), 'states'); assert.equal(options.rule, 'controlled');
  assert.equal(options.dateOverlay, false);
  const view = OPTIONS.find(option => option.key === 'view')!;
  assert.deepEqual(view.kind === 'choice' && view.choices.map(choice => choice.value), ['race', 'lines', 'share']);
  assert.deepEqual(definition.defaultCountries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
});

test('validate totals and dated spans; malformed columns, unsafe counts, duplicates and overlap are rejected', () => {
  assert.equal(validateTerritoryReply(reply()).series[0].tag, 'Renamed realm');
  for (const corrupt of [
    (data: TerritoryReply) => { data.series[0].runs[0].values.pop(); },
    (data: TerritoryReply) => { data.series[0].runs[0].values[0] = -1; },
    (data: TerritoryReply) => { data.series[0].runs[0].values[2] = 0.5; },
    (data: TerritoryReply) => { data.series[0].runs[0].values[3] = 2 ** 53; },
    (data: TerritoryReply) => { data.series.push(data.series[0]); },
    (data: TerritoryReply) => { data.series[0].runs[1].firstHour = 100; },
    (data: TerritoryReply) => { data.series[0].runs[0].startsSegment = undefined as unknown as boolean; },
    (data: TerritoryReply) => { data.series[0].identities = [{ firstHour: 100, lastHour: 101, slot: -1, countryId: 1 }]; },
    (data: TerritoryReply) => { data.series[0].identities = [{ firstHour: 100, lastHour: 101, slot: 1, countryId: 1 }, { firstHour: 101, lastHour: 102, slot: 1, countryId: 1 }]; },
  ]) { const data = reply(); corrupt(data); assert.throws(() => validateTerritoryReply(data), /malformed/); }
  assert.throws(() => validateTerritoryReply(null), /malformed/);
});

test('world totals require ordered recorded runs and exact measure columns before Share reads them', () => {
  const good = { ...reply(), worldTotals: [{ firstHour: 100, lastHour: 105, startsSegment: true, values: [2, 3, -7, 90, 1, 4, -3, null, null, null, null, null] }] };
  assert.equal(validateTerritoryReply(good), good);
  for (const worldTotals of [undefined, null,
    [{ ...good.worldTotals[0], values: [1] }],
    [{ ...good.worldTotals[0], startsSegment: null }],
    [{ ...good.worldTotals[0], firstHour: 106 }],
    [{ ...good.worldTotals[0], values: [2 ** 53, 3, -7, 90, 1, 4, -3, null, null, null, null, null] }],
    [{ ...good.worldTotals[0], values: [-1, 3, -7, 90, 1, 4, -3, null, null, null, null, null] }],
    [good.worldTotals[0], { ...good.worldTotals[0], firstHour: 105, lastHour: 106 }],
  ]) assert.throws(() => validateTerritoryReply({ ...good, worldTotals }), /malformed/);
});

test('a missing province read leaves state counts intact, keeps zero, exact signed VP, and identity breaks', () => {
  const states = territoryModel(reply(), 'states', 'controlled');
  assert.deepEqual(states.series.get('Renamed realm')!.runs, [
    { firstHour: 100, lastHour: 101, value: 2, breakBefore: true },
    { firstHour: 102, lastHour: 102, value: 0 }, { firstHour: 103, lastHour: 105, value: 0, breakBefore: true },
  ]);
  const provinces = territoryModel(reply(), 'provinces', 'controlled');
  assert.deepEqual(provinces.series.get('Renamed realm')!.runs.map(run => run.firstHour), [100, 102, 103]);
  assert.equal(territoryModel(reply(), 'victoryPoints', 'controlled').series.get('Renamed realm')!.runs[1].value, -7);
  assert.equal(territoryModel(reply(), 'area', 'owned').series.get('Renamed realm')!.runs[0].value, 300);
  assert.equal(territorySegmentAt(provinces, 'Renamed realm', 101), 100);
  assert.equal(territorySegmentAt(provinces, 'Renamed realm', 103), 103);
  assert.equal(territorySegmentAt(provinces, 'Renamed realm', 99), null);
});

test('requests preserve opaque tags and rank the selected rule and measure; clipping never draws beyond membership/range', () => {
  assert.deepEqual(territoryRequest('7', { tags: ['realm,"new'] }, 'provinces', 'owned'), { branchId: '7', tags: ['realm,"new'], top: null });
  assert.deepEqual(territoryRequest('7', { tags: [], top: 10 }, 'victoryPoints', 'controlled'),
    { branchId: '7', tags: [], top: { count: 10, measure: 'victoryPoints', rule: 'controlled' } });
  const runs = territoryModel(reply(), 'states', 'controlled').series.get('Renamed realm')!.runs;
  assert.deepEqual(shownTerritory(runs, [{ firstHour: 101, lastHour: 102 }], { firstHour: 102, lastHour: 104 }),
    [{ firstHour: 102, lastHour: 102, value: 0 }]);
  assert.equal(formatTerritory(27_917_287_411), '27,917,287,411');
});

test('area is hidden when refused by name; other measures stay available without content', () => {
  const data = { ...reply(), areaUnit: null, areaReason: 'Map area unavailable: custom-territory.bmp uses an unsupported bitmap form' };
  assert.equal(validateTerritoryReply(data).areaReason, data.areaReason);
  assert.deepEqual(measureChoices(data).map(choice => choice.value), ['states', 'provinces', 'victoryPoints']);
  assert.deepEqual(measureChoices(reply()).map(choice => choice.value), ['states', 'provinces', 'victoryPoints', 'area']);
});

test('Map area is labelled as map cells and names the projection distortion', () => {
  assert.equal(measureChoices(reply()).find(choice => choice.value === 'area')?.label, 'Map area');
  assert.equal(reply().areaUnit, 'map cells');
  assert.match(territoryNote('area')!, /Map area exaggerates territory near the poles/);
  assert.equal(territoryNote('states'), undefined);
});

test('an area refusal warns only a chosen Map area widget and leaves other diagnostics intact', () => {
  const areaReason = 'Map area unavailable: game and user folders are not configured';
  const data = { ...reply(), areaReason, areaUnit: null, warnings: ['Incomplete state read'] };
  for (const measure of ['states', 'provinces', 'victoryPoints'] as const) {
    assert.deepEqual(territoryWarnings(data, measure), ['Incomplete state read']);
  }
  assert.deepEqual(territoryWarnings(data, 'area'), ['Incomplete state read', areaReason]);
  assert.deepEqual(territoryWarnings(undefined, 'area'), []);
  assert.deepEqual(territoryWarnings(reply(), 'area'), []);
});

test('stale territory rules fall back to controlled', () => {
  assert.equal(ruleOf('owned'), 'owned');
  for (const stale of ['former', 'CONTROLLED', undefined, null, 4]) assert.equal(ruleOf(stale), 'controlled');
});

test('wire measure columns stay correct when settings label order changes', () => {
  const original = [...MEASURES];
  const choices = MEASURES as unknown as typeof original;
  try {
    choices.reverse();
    for (const [measure, column] of [['states', 0], ['provinces', 1], ['victoryPoints', 2], ['area', 3]] as const) {
      for (const rule of ['controlled', 'owned'] as const) assert.equal(territoryModel(reply(), measure, rule).series.get('Renamed realm')!.runs[0].value,
        reply().series[0].runs[0].values[column + (rule === 'owned' ? 4 : 0)]);
    }
  } finally { choices.splice(0, choices.length, ...original); }
});

test('non-faction group sums keep annexed members at recorded zero while incomplete reads stay gaps', () => {
  const roster: CountryGroupReply = { branchId: '7', humans: [{ tag: 'survivor', firstHour: 100, lastHour: 105 }, { tag: 'annexed', firstHour: 100, lastHour: 102 }],
    countryIdentities: [{ tag: 'survivor', identities: [{ firstHour: 100, lastHour: 105, slot: 1, countryId: 10 }] },
      { tag: 'annexed', identities: [{ firstHour: 100, lastHour: 102, slot: 2, countryId: 20 }] }],
    wars: [], warStarts: [], capitulations: [], identityChanges: [], unobserved: { humans: [], factions: [], wars: [] }, warnings: [], factions: [] };
  const range = { firstHour: 100, lastHour: 105 };
  const data: TerritoryReply = { ...reply(), series: [
    { tag: 'survivor', membership: null, segments: [range], runs: [{ ...range, startsSegment: true, values: Array(12).fill(5) }] },
    { tag: 'annexed', membership: null, segments: [{ firstHour: 100, lastHour: 102 }], runs: [
      { firstHour: 100, lastHour: 102, startsSegment: true, values: Array(12).fill(2) },
      { firstHour: 103, lastHour: 103, startsSegment: true, values: Array(12).fill(0) },
      { firstHour: 105, lastHour: 105, startsSegment: true, values: Array(12).fill(0) }] },
  ] };
  for (const membership of ['hour', 'end', 'ever'] as const) {
    const resolved = resolveCountries({ kind: 'group', group: { type: 'humans' }, membership, aggregate: true }, [], roster, 105, Infinity);
    const model = territoryModel(data, 'states', 'controlled');
    const lines = territoryCountryLines(model, resolved, range, true);
    const runs = aggregateGroupRuns(lines, resolved, range);
    const at = (hour: number) => runs.find(run => run.firstHour <= hour && hour <= run.lastHour)?.value;
    assert.equal(at(100), membership === 'end' ? 5 : 7);
    assert.equal(at(103), 5);
    assert.equal(at(104), membership === 'ever' ? undefined : 5);
    assert.equal(at(105), 5);
  }
});

test('manual Territory picks follow every selected native instance through aliases without admitting unrelated alias holders', () => {
  const range = { firstHour: 100, lastHour: 105 };
  const identity = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ firstHour, lastHour, slot, countryId });
  const value = (firstHour: number, lastHour: number, amount: number, startsSegment = true) => ({ firstHour, lastHour, startsSegment, values: Array(12).fill(amount) });
  const data: TerritoryReply = { ...reply(), series: [
    { tag: 'picked', membership: null, segments: [identity(100, 101, 1, 7), identity(104, 105, 2, 8)],
      identities: [identity(100, 101, 1, 7), identity(104, 105, 2, 8)],
      runs: [value(100, 101, 4), value(102, 103, 0), value(104, 105, 9)] },
    { tag: 'alias', membership: null, segments: [identity(102, 102, 1, 7), identity(104, 104, 1, 7), identity(105, 105, 3, 99)],
      identities: [identity(102, 102, 1, 7), identity(104, 104, 1, 7), identity(105, 105, 3, 99)],
      runs: [value(100, 101, 0), value(102, 102, 5), value(104, 104, 6), value(105, 105, 99)] },
    { tag: 'not released', membership: null, segments: [], identities: [], runs: [value(100, 105, 0)] },
  ] };
  const resolved = resolveCountries({ kind: 'tags', tags: ['picked', 'not released'] }, [], undefined, 105, Infinity);
  const model = territoryModel(validateTerritoryReply(data), 'states', 'controlled', resolved.tags);
  assert.equal(model.selectedCount, 2, 'two selected native instances stay independently drawable');
  const lines = territoryCountryLines(model, resolved, range);
  const at = (tag: string, hour: number) => runAt(lines.find(line => line.tag === tag)!.runs, hour)?.value;
  assert.equal(at('picked', 102), undefined, 'absence is not an individual zero line');
  assert.equal(at('alias', 102), 5, 'the original picked holder continues under its alias');
  assert.equal(at('alias', 103), undefined, 'a missing native-identity hour stays a gap');
  assert.equal(at('alias', 104), 6);
  assert.equal(at('alias', 105), undefined, 'an unrelated holder of an alias tag is not selected');
  assert.equal(at('picked', 105), 9, 'a later native instance of the explicitly picked literal tag remains a separate selection');
  assert.equal(at('not released', 100), undefined);
  assert.deepEqual(territoryShareKeys(model, resolved, 102), ['alias']);
  assert.deepEqual(territoryShareKeys(model, resolved, 103), []);
  assert.deepEqual(territoryShareKeys(model, resolved, 105), ['picked']);
  assert.equal(territorySegmentAt(model, 'alias', 105), null);
  const badges = countryCapitulationBadges({ capitulations: [
    { tag: 'alias', hour: 102 }, { tag: 'alias', hour: 103 }, { tag: 'alias', hour: 105 },
  ] }, [...model.series].map(([tag, series]) => ({ tag, ...series })), [range.firstHour, range.lastHour]);
  assert.deepEqual(badges.get('alias')?.map(badge => badge.hour), [102], 'markers use the same selected native coverage as the lines');
  data.series[1].runs[1].values[0] = null;
  const missing = territoryModel(data, 'states', 'controlled', resolved.tags);
  assert.deepEqual(territoryShareKeys(missing, resolved, 102), ['alias'], 'a present holder with missing totals stays a required Share candidate');
});

test('individual group and legacy country lines end at disappearance while fixed aggregate zeros survive', () => {
  const range = { firstHour: 100, lastHour: 105 };
  const data: TerritoryReply = { ...reply(), series: [{ tag: 'annexed', membership: null,
    segments: [{ firstHour: 100, lastHour: 102 }], runs: [
      { firstHour: 100, lastHour: 102, startsSegment: true, values: Array(12).fill(2) },
      { firstHour: 103, lastHour: 105, startsSegment: true, values: Array(12).fill(0) },
    ] }] };
  const resolved = resolveCountries({ kind: 'tags', tags: ['annexed'] }, [], undefined, 105, Infinity);
  const model = territoryModel(data, 'states', 'controlled', resolved.tags);
  assert.equal(runAt(territoryCountryLines(model, resolved, range)[0].runs, 103), null);
  assert.equal(runAt(territoryCountryLines(model, resolved, range, true)[0].runs, 103)?.value, 0);
  assert.deepEqual(territoryShareKeys(model, resolved, 103), []);
});

test('occupied columns preserve signed VP, exact zero and independent unknowns without reusing owned totals', () => {
  const data = reply();
  const occupied = [[2, 4, -9, 27], [2, null, null, null], [0, 3, 0, 0], [0, 3, 5, 50]];
  data.series[0].runs.forEach((run, index) => run.values.splice(8, 4, ...occupied[index]));
  assert.equal(validateTerritoryReply(data), data);
  assert.equal(ruleOf('occupied'), 'occupied');
  for (const [measure, column] of [['states', 8], ['provinces', 9], ['victoryPoints', 10], ['area', 11]] as const) {
    assert.equal(territoryColumn(measure, 'occupied'), column);
    const runs = territoryModel(data, measure, 'occupied').series.get('Renamed realm')!.runs;
    assert.equal(runs[0].value, occupied[0][column - 8]);
  }
  assert.equal(territoryModel(data, 'victoryPoints', 'occupied').series.get('Renamed realm')!.runs[1].value, 0);
  assert.deepEqual(territoryRequest('7', {tags: [], top: 5_000}, 'victoryPoints', 'occupied').top,
    {count: 5_000, measure: 'victoryPoints', rule: 'occupied'});
  assert.deepEqual(measureChoices(data).map(choice => choice.value), ['states', 'provinces', 'victoryPoints', 'area']);
  assert.deepEqual(territoryRequest('7', {tags: [], top: 5_000}, 'area', 'occupied').top,
    {count: 5_000, measure: 'area', rule: 'occupied'});
  assert.equal(territoryModel(data, 'area', 'occupied').series.get('Renamed realm')!.runs[1].value, 0);
  for (const column of [8, 9, 11]) {
    const invalid = structuredClone(data); invalid.series[0].runs[0].values[column] = -1;
    assert.throws(() => validateTerritoryReply(invalid), /malformed/);
  }
  for (const column of [10, 11]) {
    const invalid = structuredClone(data); invalid.series[0].runs[0].values[column] = 2 ** 53;
    assert.throws(() => validateTerritoryReply(invalid), /malformed/);
  }
});
