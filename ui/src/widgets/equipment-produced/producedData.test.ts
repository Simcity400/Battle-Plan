import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { createWidget } from '../../dashboard/model.ts';
import { rankedCandidates, resolveCountries } from '../../dashboard/groups.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { DATE_OVERLAY_OPTION } from '../../charts/race/options.ts';
import { mixCountry } from '../../charts/race/options.ts';
import { countryRaceItems } from '../../charts/race/countryRace.ts';
import type { EquipmentProducedReply, ProducedKey, ProducedSeries } from '../../generated/dashboard';
import '../definitions.ts';
import { definition, OPTIONS } from './definition.ts';
import { bandValue, column, emptyRace, equipmentBandValue, equipmentGroupValue, keyBands, keyLabel, mixBands, producedAt, producedModel, producedRequest, producedTooltips, raceEntries, rankProduced,
  seriesBands, storedValues, totalRuns, typeLabel, validateProducedReply, valueAt } from './producedData.ts';

/** A coded value: 0 before `from`, the running sum of `deltas`, unknown in `unknown`'s ranges. */
const c = (from: number, deltas: number[], unknown: number[] = []) => ({ from, deltas, unknown });
const key = (item: Partial<ProducedKey> & { key: string }): ProducedKey => ({ name: null, typeKey: null, domain: null, names: [], ...item });

/** Germany's and Italy's infantry equipment and fighters over four daily reads (hours
 * 8, 32, 56, 80, each carried 23 hours). Germany's fighters are unknown from the third
 * read (a tick not counted), so its total is too; Italy has no read on the second day. */
const REPLY: EquipmentProducedReply = {
  branchId: '1', measure: 'total', count: 'units', decimals: 0, grouping: 'type',
  types: [{ key: 'infantry_equipment', name: 'Infantry Equipment', domain: 'land' }, { key: 'fighter_equipment', name: 'Fighter', domain: 'air' }],
  keys: [key({ key: 'infantry_equipment', name: 'Infantry Equipment', domain: 'land' }), key({ key: 'fighter_equipment', name: 'Fighter', domain: 'air' })],
  series: [
    { tag: 'GER', firstHours: [8, 32, 56, 80], lastHours: [31, 55, 79, 103], breaks: [0], membership: null,
      values: [{ key: 0, values: c(1, [10, 5]) }, { key: 1, values: c(1, [3], [2, 4]) }], total: c(1, [13], [2, 4]) },
    { tag: 'ITA', firstHours: [8, 56, 80], lastHours: [31, 79, 103], breaks: [0, 1], membership: null,
      values: [{ key: 0, values: c(1, [12, 8]) }], total: c(1, [12, 8]) },
  ],
  known: [{ firstHour: 8, lastHour: 55 }], reads: [8, 32, 56, 80], observations: [8], warnings: [],
};

const read = (values: ReturnType<typeof column> | null, length: number) =>
  Array.from({ length }, (_, index) => valueAt(values, index)).map(value => Number.isNaN(value) ? null : value);

test('independent equipment subtotals are readable without plotting or ranking an unknown combined amount', () => {
  const reply: EquipmentProducedReply = { ...REPLY, series: [{ ...REPLY.series[0],
    values: [{ key: 0, values: c(0, [], [0, 4]), equipmentValues: c(0, [-5, 1]) }],
    total: c(0, [], [0, 4]), }], known: [] };
  assert.equal(validateProducedReply(reply), reply);
  const model = producedModel(reply);
  const series = model.series.get('GER')!;
  assert.equal(equipmentGroupValue(series, 0, 1), -4);
  assert.equal(equipmentBandValue(series, { id: '0', keys: [0], color: '#123456', other: false }, 1), -4);
  assert.equal(producedAt(model, 'GER', 40)?.total, null);
  assert.deepEqual(rankProduced(model, ['GER'], 40, 10), []);
  assert.deepEqual(totalRuns(model, 'GER', null, { firstHour: 8, lastHour: 103 }), []);
  const tooltip = [...producedTooltips(model, 'GER', 'Germany', producedAt(model, 'GER', 40)!, 40, 'Recorded day', [])][0];
  assert.deepEqual(tooltip.rows.map(row => [row.label, row.value]), [
    ['Infantry Equipment', 'Not known'], ['Infantry Equipment — Equipment lines', '−4'], ['Total', 'Not known'],
  ]);
  assert.match(tooltip.note!, /Total unavailable; equipment subtotal shown/);
  assert.throws(() => validateProducedReply({ ...reply, series: [{ ...reply.series[0],
    values: [{ ...reply.series[0].values[0], equipmentValues: c(0, [Number.MAX_SAFE_INTEGER + 1]) }] }] }));
});

test('values read from their first nonzero sample, unknown only inside their ranges', () => {
  assert.deepEqual(read(column(c(1, [10, 5])), 4), [0, 10, 15, 15]);
  // Known values after an unknown stretch survive, coded against the last known one.
  assert.deepEqual(read(column(c(0, [3, -2, 0, 4], [2, 3])), 5), [3, 1, null, 5, 5]);
  assert.deepEqual(read(null, 2), [0, 0]);
  const model = producedModel(REPLY);
  const germany = model.series.get('GER')!;
  // The country's total is the product's, exact however its groups are known.
  assert.deepEqual(read(germany.total, 4), [0, 13, null, null]);
  assert.deepEqual(germany.stretch, [8, 8, 8, 8]);
  // Italy's missing read breaks its stretch: a bar never glides across it.
  assert.deepEqual(model.series.get('ITA')!.stretch, [8, 56, 56]);
});

test('the reply is checked for samples in order, known keys and safe numbers', () => {
  assert.equal(validateProducedReply(REPLY), REPLY);
  const broken = (series: Partial<ProducedSeries>) => ({ ...REPLY, series: [{ ...REPLY.series[0], ...series }] });
  assert.throws(() => validateProducedReply(broken({ lastHours: [31, 55, 79] })));
  assert.throws(() => validateProducedReply(broken({ firstHours: [8, 30, 56, 80] })));
  assert.throws(() => validateProducedReply(broken({ breaks: [4] })));
  assert.throws(() => validateProducedReply(broken({ values: [{ key: 2, values: c(0, []) }] })));
  assert.throws(() => validateProducedReply(broken({ values: [{ key: 0, values: c(0, [0.5]) }] })));
  assert.throws(() => validateProducedReply(broken({ values: [{ key: 0, values: c(0, [1], [1]) }] })));
  assert.throws(() => validateProducedReply(broken({ total: c(0, [1], [3, 2]) })));
  assert.throws(() => validateProducedReply({ ...REPLY, measure: 'year' }));
  assert.throws(() => validateProducedReply({ ...REPLY, known: [{ firstHour: 8, lastHour: 7 }] }));
  assert.throws(() => validateProducedReply({ ...REPLY, known: [{ firstHour: 8, lastHour: 31 }, { firstHour: 31, lastHour: 55 }] }));
});

test('countries rank by the value shown; an unknown or unrecorded one is left out, never zero', () => {
  const model = producedModel(REPLY);
  const ranked = (hour: number) => rankProduced(model, ['GER', 'ITA', 'FRA'], hour, 10).map(entry => [entry.tag, entry.at.total]);
  assert.deepEqual(ranked(8), [['GER', 0], ['ITA', 0]]);
  // Italy has no read covering hour 40.
  assert.deepEqual(ranked(40), [['GER', 13]]);
  // Germany's fighters are unknown from hour 56: it drops out rather than ranking on infantry alone.
  assert.deepEqual(ranked(60), [['ITA', 12]]);
  assert.equal(producedAt(model, 'GER', 60)!.total, null);
  assert.deepEqual(rankProduced(model, ['GER', 'ITA'], 8, 1).map(entry => entry.tag), ['GER']);
});

test('the top N rank a country only once it has produced something, and say why nobody shows', () => {
  const model = producedModel(REPLY);
  // Everyone is at zero on the first read: as the top N rank, nobody shows yet.
  assert.deepEqual(rankProduced(model, ['GER', 'ITA'], 8, 10, true), []);
  assert.equal(emptyRace(model, 8), 'nothing');
  assert.deepEqual(rankProduced(model, ['GER', 'ITA'], 40, 10, true).map(entry => entry.tag), ['GER']);
  // Where nothing is known, it says so instead of claiming nothing was made.
  assert.equal(emptyRace(producedModel({ ...REPLY, known: [] }), 60), 'unknown');
  // Coverage survives the top-N filter; before the first read nothing is known.
  const top = producedModel({ ...REPLY, series: [{ ...REPLY.series[0], membership: [{ firstHour: 32, lastHour: 103 }] },
    { ...REPLY.series[1], membership: [{ firstHour: 56, lastHour: 103 }] }] });
  assert.equal(emptyRace(top, 10), 'nothing');
  assert.equal(emptyRace(top, 4), 'unknown');
});

test('race bars are one total or one segment per group, summing to the label', () => {
  const model = producedModel(REPLY);
  const bands = keyBands(model, ['#a', '#b', '#c'], '#muted');
  const ranked = rankProduced(model, ['GER', 'ITA'], 40, 10);
  const entries = (split: boolean) => countryRaceItems(raceEntries(model, ranked, { split, bands: () => bands, unknown: '#u' }),
    new Map(), () => '#country', '#u', false);
  const [germany] = entries(true);
  assert.equal(germany.label, '13');
  assert.deepEqual(germany.segments.map(segment => [segment.id, segment.value, segment.color]), [['0', 10, '#a'], ['1', 3, '#b']]);
  assert.equal(germany.segments.reduce((sum, segment) => sum + segment.value, 0), germany.value);
  const [whole] = entries(false);
  assert.deepEqual(whole.segments, [{ id: 'total', color: '#country', value: 13 }]);
});

test('production tooltips name the hovered hour and lines name their own drawn ends', async () => {
  // Render the widget and shared view; only transport, artwork and canvas leaves are replaced.
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(REPLY)} });`,
    '/dashboard/playback.ts': `export const usePlaybackHour = () => {};
      export const usePlaybackSpeed = clock => clock.getSnapshot().hoursPerSecond;`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = (_client, _track, _tags, moment, _active, _segment, hours) => ({
      name: (tag, hour) => tag + ' at ' + (hour ?? hours?.get(tag) ?? moment), color: () => [50, 100, 150], flag: () => undefined, revision: 0 });`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: hour => tag => ({ name: tag + ' hovered ' + hour }), revision: 0 });`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123', '#456'], muted: '#888', surface: '#000', fontSize: { body: 12 } });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/draw.ts': `export const dayLabel = String; export const drawBands = () => {}; export const drawLine = () => {};
      export const placeEndLabels = (ctx, _frame, _tokens, labels) => { ctx.labels = labels; };`,
    '/charts/race/RaceChart.tsx': `import { createElement } from 'react';
      export default props => createElement('p', null, props.items.map(item => props.tooltip(item.key)?.title).join('|'));`,
    '/charts/TimeChart.tsx': `import { createElement } from 'react';
      export default props => { const ctx = {}; props.overlay(ctx, { x: props.extent, toY: value => value }, {});
        return createElement('p', null, ctx.labels.map(label => label.text).join('|')); }`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null },
    plugins: [{ name: 'production-chart-leaves', enforce: 'pre', transform: (_source, id) => {
      const replacement = Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix));
      return replacement?.[1];
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/equipment-produced/EquipmentProducedWidget.tsx');
    const render = (view: string, lastHour: number) => {
      const widget = createWidget('equipment-produced', 'track-1', 'w1');
      widget.countries = { kind: 'tags', tags: ['GER', 'ITA'] };
      widget.options = { view };
      const track = { id: 'track-1', source: { id: 's', revision: 1, metadata: { countries: [] } },
        branch: { id: '1', firstHour: 8, lastHour, gaps: [] } };
      const runtime = { tracks: [track], clock: { getHour: () => lastHour, getSnapshot: () => ({ hoursPerSecond: 24 }) } };
      return renderToStaticMarkup(createElement(Widget, { widget, runtime, active: true, onDiagnostics() {} }));
    };
    assert.match(render('race', 40), /GER hovered 40/);
    const lines = render('lines', 90);
    assert.match(lines, /GER at 55 13/);
    assert.match(lines, /ITA at 90 20/);
  } finally { await vite.close(); }
});

test('empty top-N weeks and months distinguish complete zero windows from unknown ones', () => {
  const start = 8;
  for (const measure of ['week', 'month'] as const) {
    const days = measure === 'week' ? 10 : 40;
    const firstWindow = measure === 'week' ? 7 : 31;
    const firstHours = Array.from({ length: days - firstWindow }, (_, day) => start + (firstWindow + day) * 24);
    const lastHours = firstHours.map(hour => hour + 23);
    const reply: EquipmentProducedReply = { ...REPLY, measure,
      reads: Array.from({ length: days }, (_, day) => start + day * 24), observations: [start],
      known: [{ firstHour: firstHours[0], lastHour: lastHours.at(-1)! }], series: [] };
    const model = producedModel(reply);
    const dayTwo = start + 2 * 24;
    assert.deepEqual(rankProduced(model, ['AAA'], dayTwo, 10, true), []);
    assert.equal(emptyRace(model, dayTwo), 'unknown', measure);
    // Nobody ever ranks in a zero-production branch, even after a complete window.
    assert.equal(emptyRace(model, firstHours[0]), 'nothing', measure);
    assert.equal(emptyRace(model, firstHours[0] + 23), 'nothing', measure);
    assert.equal(emptyRace(model, lastHours.at(-1)! + 1), 'unknown', measure);
    // Missing or inexact counts never become zeros merely because daily reads exist.
    assert.equal(emptyRace(producedModel({ ...reply, known: [] }), firstHours[0]), 'unknown', measure);
  }
});

test('empty races require all present countries to be known, including before top-N filtering', () => {
  const first = 8 + 7 * 24;
  const reply: EquipmentProducedReply = { ...REPLY, measure: 'week', keys: [], types: [],
    series: [
      { tag: 'opaque-a', firstHours: [first, first + 24, first + 48], lastHours: [first + 23, first + 47, first + 71],
        breaks: [0], membership: null, values: [], total: null },
      { tag: 'opaque-b', firstHours: [first, first + 24], lastHours: [first + 23, first + 47],
        breaks: [0], membership: null, values: [], total: c(2, [], [1, 2]) },
    ],
    known: [{ firstHour: first, lastHour: first + 23 }, { firstHour: first + 48, lastHour: first + 71 }] };
  const explicit = producedModel(validateProducedReply(reply));
  assert.equal(producedAt(explicit, 'opaque-a', first + 24)!.total, 0);
  assert.equal(producedAt(explicit, 'opaque-b', first + 24)!.total, null);
  assert.equal(producedAt(explicit, 'opaque-b', first + 48), null);
  for (const model of [explicit, producedModel({ ...reply, series: [] })]) {
    assert.deepEqual(rankProduced(model, ['opaque-a', 'opaque-b'], first + 24, 10, true), []);
    assert.equal(emptyRace(model, first + 24), 'unknown');
    assert.equal(emptyRace(model, first + 23), 'nothing');
    assert.equal(emptyRace(model, first + 48), 'nothing');
    assert.equal(emptyRace(model, first - 1), 'unknown');
  }
});

test('a type an unresolved variant could belong to is unknown; the bar keeps the exact total, its unknown part hatched', () => {
  // Infantry is unknown on the second read (an unresolved variant held something); the
  // unresolved group and the total are exact.
  const reply: EquipmentProducedReply = { ...REPLY, keys: [key({ key: 'infantry_equipment', name: 'Infantry Equipment' }), key({ key: ':mod_gun' })],
    series: [{ tag: 'GER', firstHours: [8, 32], lastHours: [31, 55], breaks: [0], membership: null,
      values: [{ key: 0, values: c(0, [4, 6], [1, 2]) }, { key: 1, values: c(1, [2]) }], total: c(0, [4, 8]) }] };
  const model = producedModel(validateProducedReply(reply));
  const bands = keyBands(model, ['#a', '#b'], '#muted');
  const ranked = rankProduced(model, ['GER'], 40, 10);
  assert.equal(ranked[0].at.total, 12);
  const [bar] = raceEntries(model, ranked, { split: true, bands: () => bands, unknown: '#u' });
  assert.equal(bar.value, 12);
  assert.equal(bar.label, '12');
  assert.deepEqual(bar.parts, [{ id: '1', color: '#b', value: 2 }, { id: 'unknown', color: '#u', value: 0, hatched: 10 }]);
  const [tooltip] = [...producedTooltips(model, 'GER', 'Germany', producedAt(model, 'GER', 40)!, 40, '2 Jan', bands)];
  assert.deepEqual(tooltip.rows.map(row => [row.label, row.value]), [['Equipment type name unavailable', '2'], ['Infantry Equipment', 'Not known'], ['Total', '12']]);
});

test('by exact variant each country keeps its own largest variants in colour', () => {
  const keys = ['1', '2', '3', '4'].map(id => key({ key: id }));
  const reply: EquipmentProducedReply = { ...REPLY, grouping: 'variant', keys, series: [
    { tag: 'GER', firstHours: [8], lastHours: [31], breaks: [0], membership: null, total: c(0, [16]),
      values: [{ key: 0, values: c(0, [9]) }, { key: 1, values: c(0, [5]) }, { key: 2, values: c(0, [2]) }] },
    { tag: 'SOV', firstHours: [8], lastHours: [31], breaks: [0], membership: null, total: c(0, [8]),
      values: [{ key: 3, values: c(0, [7]) }, { key: 2, values: c(0, [1]) }] },
  ] };
  const model = producedModel(validateProducedReply(reply));
  const bandsOf = (tag: string) => seriesBands(model, tag, ['#1', '#2'], '#o').map(band => [band.id, band.keys, band.color]);
  // Germany's own largest (variant 1) keeps a colour; the rest are Other.
  assert.deepEqual(bandsOf('GER'), [['0', [0], '#1'], ['other', [1, 2], '#o']]);
  // The Soviet Union has two variants, which fit.
  assert.deepEqual(bandsOf('SOV'), [['2', [2], '#1'], ['3', [3], '#2']]);
});

test('compact columns stay bounded by the reply, not by samples times groups', () => {
  // 30 countries, 3,650 daily samples and 1,000 constant variant columns each (Codex
  // round 1: about 877 MB of typed arrays had every column been expanded).
  const samples = 3650;
  const firstHours = Array.from({ length: samples }, (_, index) => index * 24);
  const lastHours = firstHours.map(hour => hour + 23);
  const keys = Array.from({ length: 1000 }, (_, index) => key({ key: String(index + 1) }));
  const series = Array.from({ length: 30 }, (_, country) => ({ tag: `C${country}`, firstHours, lastHours, breaks: [0], membership: null,
    values: keys.map((_, index) => ({ key: index, values: c(0, [1]) })), total: c(0, [1000]) }));
  const model = producedModel({ ...REPLY, grouping: 'variant', keys, series });
  assert.ok(storedValues(model) <= 30 * 1001, `${storedValues(model)} numbers held`);
  const germany = model.series.get('C0')!;
  assert.equal(valueAt(germany.values.get(999)!, samples - 1), 1);
  assert.equal(valueAt(germany.total, samples - 1), 1000);
});

test('beyond the palette the largest groups keep their colours and the rest fold into Other', () => {
  const keys = ['a', 'b', 'c', 'd'].map(name => key({ key: name, name }));
  const reply: EquipmentProducedReply = { ...REPLY, keys, series: [{ tag: 'GER', firstHours: [8], lastHours: [31], breaks: [0], membership: null,
    total: c(0, [17]), values: [{ key: 0, values: c(0, [1]) }, { key: 1, values: c(0, [9]) }, { key: 2, values: c(0, [5]) }, { key: 3, values: c(0, [2]) }] }] };
  const model = producedModel(reply);
  const bands = keyBands(model, ['#1', '#2', '#3'], '#muted');
  // b and c are the largest; they keep the reply's order, and a and d share Other.
  assert.deepEqual(bands.map(band => [band.id, band.keys, band.color]), [['1', [1], '#1'], ['2', [2], '#2'], ['other', [0, 3], '#muted']]);
  assert.equal(bandValue(model.series.get('GER')!, bands[2], 0), 3);
});

const ALL = { firstHour: 0, lastHour: 1000 };

test('lines and the mix break where a value is unknown or the recording breaks, and follow a group and the range', () => {
  const model = producedModel(REPLY);
  // A date range cuts every line to the hours shown.
  assert.deepEqual(totalRuns(model, 'ITA', null, { firstHour: 20, lastHour: 70 }).map(run => [run.firstHour, run.lastHour, run.value]), [[20, 31, 0], [56, 70, 12]]);
  assert.deepEqual(totalRuns(model, 'GER', null, ALL), [{ firstHour: 8, lastHour: 31, value: 0, breakBefore: true }, { firstHour: 32, lastHour: 55, value: 13 }]);
  assert.deepEqual(totalRuns(model, 'ITA', [{ firstHour: 60, lastHour: 90 }], ALL).map(run => [run.firstHour, run.lastHour, run.value]), [[60, 79, 12], [80, 90, 20]]);
  const bands = keyBands(model, ['#a', '#b'], '#muted');
  const [infantry, fighters] = mixBands(model, 'GER', bands, null, ALL);
  assert.deepEqual(infantry.map(run => [run.firstHour, run.lastHour, run.value]), [[8, 31, 0], [32, 55, 10], [56, 103, 15]]);
  assert.deepEqual(fighters.map(run => [run.firstHour, run.lastHour, run.value]), [[8, 31, 0], [32, 55, 3]]);
});

test('a variant is named only from the read that recorded its name, and not where its names were not known', () => {
  const reply: EquipmentProducedReply = { ...REPLY, grouping: 'variant', keys: [key({ key: '20', typeKey: 'infantry_equipment',
    names: [{ firstHour: 8, name: null, known: true, design: null }, { firstHour: 56, name: 'Gewehr 43', known: true, design: null },
      { firstHour: 70, name: null, known: false, design: null }, { firstHour: 80, name: 'Gewehr 43', known: true, design: null }] })], series: [] };
  const model = producedModel(validateProducedReply(reply));
  assert.equal(keyLabel(model, 0, 8), 'Variant 20');
  assert.equal(keyLabel(model, 0, 60), 'Gewehr 43');
  // A gap or a due read that did not land: the old name is not carried over it.
  assert.equal(keyLabel(model, 0, 75), 'Variant 20');
  assert.equal(keyLabel(model, 0, 85), 'Gewehr 43');
  assert.equal(keyLabel(producedModel({ ...reply, keys: [key({ key: '20', names: [{ firstHour: 56, name: 'Gewehr 43', known: true, design: null }] })] }), 0, 20), 'Variant 20');
  const domains = producedModel({ ...REPLY, grouping: 'domain', keys: [key({ key: 'air' }), key({ key: 'unresolved' })], series: [] });
  assert.equal(keyLabel(domains, 0, 0), 'Air');
  assert.equal(keyLabel(domains, 1, 0), 'Type not resolved');
  const types = producedModel({ ...REPLY, keys: [key({ key: ':mod_gun' })], series: [] });
  assert.equal(keyLabel(types, 0, 0), 'Equipment type name unavailable');
});

test('the tooltip lists every group, most first, unknown last, with the total and when it was read', () => {
  const model = producedModel(REPLY);
  const bands = keyBands(model, ['#a', '#b'], '#muted');
  const [full, compact] = [...producedTooltips(model, 'GER', 'Germany', producedAt(model, 'GER', 40)!, 40, '2 Jan', bands)];
  assert.deepEqual(full.rows.map(row => [row.label, row.value]), [['Infantry Equipment', '10'], ['Fighter', '3'], ['Total', '13']]);
  assert.equal(full.subtitle, '2 Jan · Running total');
  assert.match(full.note!, /^Counted since [^.]*\.$/);
  assert.equal(compact.rows.length, 3);
  const [unknown] = [...producedTooltips(model, 'GER', 'Germany', producedAt(model, 'GER', 60)!, 60, '3 Jan', bands)];
  assert.deepEqual(unknown.rows.map(row => [row.label, row.value]), [['Infantry Equipment', '15'], ['Fighter', 'Not known'], ['Total', 'Not known']]);
  const [nothing] = [...producedTooltips(model, 'ITA', 'Italy', producedAt(model, 'ITA', 8)!, 8, '1 Jan', bands)];
  assert.deepEqual(nothing.rows.map(row => row.label), ['Nothing delivered', 'Total']);
});

test('tags are opaque and types are listed as the recorded content names them, mods included', () => {
  // Any non-NUL bytes: a space, a dynamic country's tag.
  const reply: EquipmentProducedReply = { ...REPLY, series: [{ ...REPLY.series[0], tag: 'MOD TAG' }, { ...REPLY.series[1], tag: 'D01' }] };
  const model = producedModel(validateProducedReply(reply));
  assert.deepEqual(rankProduced(model, ['MOD TAG', 'D01'], 60, 10).map(entry => entry.tag), ['D01']);
  assert.deepEqual(rankProduced(model, ['MOD TAG', 'D01'], 40, 10).map(entry => entry.tag), ['MOD TAG']);
  assert.deepEqual(producedRequest('1', { tags: ['MOD TAG', 'D01'] }, 'total', 'type', [], false).tags, ['MOD TAG', 'D01']);
  // A mod's type by its own name; a type content does not resolve by its recorded token.
  assert.equal(typeLabel({ key: 'mod_small_arms', name: 'Mod Rifles' }), 'Mod Rifles');
  assert.equal(typeLabel({ key: 'mod_small_arms', name: null }), 'Equipment type name unavailable');
  assert.equal(typeLabel({ key: ':destroyer_1', name: null }), 'Equipment type name unavailable');
  assert.equal(typeLabel({ key: ':77', name: null }), 'Equipment type name unavailable');
});

test('the request asks for the top N or the chosen countries, with the measure, grouping and types', () => {
  assert.deepEqual(producedRequest('1', { tags: [], top: 10 }, 'week', 'variant', ['infantry_equipment'], false),
    { branchId: '1', measure: 'week', count: 'units', grouping: 'variant', included: ['infantry_equipment'], refits: false, tags: [], top: 10 });
  assert.deepEqual(producedRequest('1', { tags: ['GER'] }, 'total', 'type', [], true),
    { branchId: '1', measure: 'total', count: 'units', grouping: 'type', included: [], refits: true, tags: ['GER'], top: null });
});

test('production selection and ranking accept every recorded country', () => {
  assert.equal(definition.countries.max, Number.MAX_SAFE_INTEGER);
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD${index}`);
  const named = resolveCountries({ kind: 'tags', tags }, [], undefined, 40, definition.countries.max);
  assert.equal(named.dropped, 0);
  assert.deepEqual(producedRequest('1', named, 'total', 'type', [], false).tags, tags);
  const top = resolveCountries({ kind: 'group', group: { type: 'top', count: tags.length }, membership: 'hour', aggregate: false },
    [], undefined, 40, definition.countries.max);
  assert.equal(producedRequest('1', top, 'total', 'type', [], false).top, tags.length);
  const model = producedModel({ ...REPLY, keys: [], series: tags.map(tag => ({ tag, firstHours: [8], lastHours: [103],
    breaks: [0], membership: null, values: [], total: c(0, [1]) })) });
  assert.equal(rankProduced(model, tags, 40, tags.length).length, tags.length);
});

test('a new widget races the top 10 of all countries by running total, date overlay off', () => {
  const widget = createWidget('equipment-produced', 'track-1', 'w1');
  assert.deepEqual(widget.countries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
  const options = resolveOptions(OPTIONS, widget.options);
  assert.equal(options.view, 'race');
  assert.equal(options.measure, 'total');
  assert.equal(options.grouping, 'type');
  assert.equal(options.refits, false);
  assert.equal(options[DATE_OVERLAY_OPTION.key], false);
  const resolved = resolveCountries(widget.countries, [], undefined, 100, 30);
  assert.equal(resolved.top, 10);
  const model = producedModel(REPLY);
  const ranked = rankProduced(model, rankedCandidates(resolved, 40, model.tags), 40, resolved.top!);
  assert.deepEqual(ranked.map(entry => entry.tag), ['GER']);
  assert.equal(mixCountry(widget.countries, null, ranked), 'GER');
});
