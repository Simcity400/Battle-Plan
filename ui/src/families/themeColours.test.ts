import assert from 'node:assert/strict';
import test from 'node:test';
import { colour, MARK_CONTRAST } from '../charts/palette.ts';
import { drawNetBar, drawWinRateCell, resolveKitTokens } from '../draw/kit.ts';
import type { Canvas2D } from '../charts/textWidth.ts';
import { contrast, parseColor, parseColorAlpha } from '../shell/appearance/color.ts';
import { BUILT_IN_THEMES, deriveColors, markColours, themeColors } from '../shell/appearance/themes.ts';
import { drawFamilyBars } from './bars/painter.ts';
import { layoutFamilyBars } from './bars/layout.ts';
import type { BarsModel } from './bars/model.ts';
import type { Breakdown, BreakdownPart as WirePart } from '../generated/dashboard';
import { layoutBreakdown } from './breakdown/layout.ts';
import { breakdownLevel } from './breakdown/model.ts';
import { drawBreakdown } from './breakdown/painter.ts';
import { breakdownParts, breakdownTotal } from './breakdown/product.ts';
import { drawCards } from './card/painter.ts';
import { layoutCards } from './card/layout.ts';
import type { CardModel } from './card/model.ts';
import type { FamilyTheme } from './contract.ts';
import { drawFlow, layoutFlow } from './flow/painter.ts';
import type { FlowModel } from './flow/model.ts';
import { layoutHeatLanes } from './heat/lanesLayout.ts';
import type { HeatLanesModel } from './heat/lanesModel.ts';
import { drawHeatLanes } from './heat/lanesPainter.ts';
import { layoutHeatMatrix } from './heat/matrixLayout.ts';
import type { HeatMatrixModel, MatrixAxisEntity } from './heat/matrixModel.ts';
import { drawHeatMatrix } from './heat/matrixPainter.ts';
import { drawMeterModel, layoutMeterModel } from './meter/painter.ts';
import type { MeterModel } from './meter/model.ts';
import { buildColumnLayouts, layoutFamilyTable } from './table/layout.ts';
import { resolveTableRows, type TableColumnSpec } from './table/model.ts';
import { createTablePicture, drawFamilyTable, NO_GLIDE } from './table/painter.ts';
import { tableSettingsOf } from './table/settings.ts';
import { familyThemeOf } from './theme.fixture.ts';
import { layoutTimeChart } from './time-chart/layout.ts';
import './time-chart/path2d.fixture.ts';
import type { TimeChartModel } from './time-chart/model.ts';
import { drawTimeChart } from './time-chart/painter.ts';
import { frameTimeline, placeTimeline } from './timeline/layout.ts';
import type { TimelineModel } from './timeline/model.ts';
import { drawTimeline } from './timeline/painter.ts';
import { resolutionFor } from './timeline/resolution.ts';
import { NO_ART } from './art/types.ts';
import { knownValue, notRecorded, partialValue, zeroValue } from './value.ts';

const LIGHT = familyThemeOf(deriveColors({ canvas: '#f4efdc', accent: '#8a2be2' }));
const DARK = familyThemeOf(deriveColors({ canvas: '#04191d', accent: '#ff7a00' }));
const DEFAULT_LITERALS = [...new Set([
  ...Object.values(themeColors(BUILT_IN_THEMES[0], 'dark')),
  '#d4d4d4', '#cfcfcf', '#c4c4c4', '#c8c8c8', '#64748b', '#475569', '#ff6b6f', '#e5484d', '#4fbf7a', '#6fd496', '#3fae63',
  '#1e1e1e', '#929292', '#ffffff', 'rgb(15, 23, 42)', 'rgb(17, 17, 17)',
].map(key))];

function key(value: string): string {
  const parsed = parseColorAlpha(value);
  if (!parsed) return value.trim().toLowerCase();
  return `${Math.round(parsed.rgb.r)},${Math.round(parsed.rgb.g)},${Math.round(parsed.rgb.b)}`;
}

function recorder(): { readonly ctx: Canvas2D; readonly colours: Set<string>; readonly unread: Set<string> } {
  const colours = new Set<string>();
  const unread = new Set<string>();
  const note = (value: unknown) => {
    if (typeof value !== 'string') return;
    const parsed = parseColorAlpha(value);
    if (!parsed) unread.add(value);
    else if (parsed.alpha > 0) colours.add(key(value));
  };
  const gradient = { addColorStop: (_at: number, value: string) => note(value) };
  const state = new Map<PropertyKey, unknown>([['globalAlpha', 1], ['lineWidth', 1], ['font', '10px sans-serif']]);
  const ctx = new Proxy({}, {
    get(_target, prop) {
      if (state.has(prop)) return state.get(prop);
      if (prop === 'measureText') return (text: string) => ({ width: String(text).length * 6.5, actualBoundingBoxAscent: 8, actualBoundingBoxDescent: 2 });
      if (prop === 'createLinearGradient' || prop === 'createRadialGradient' || prop === 'createConicGradient') return () => gradient;
      if (prop === 'createPattern') return () => null;
      if (prop === 'getLineDash') return () => [];
      if (prop === 'getTransform') return () => ({ a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 });
      if (prop === 'canvas') return { width: 800, height: 400 };
      return () => undefined;
    },
    set(_target, prop, value) {
      if (prop === 'fillStyle' || prop === 'strokeStyle' || prop === 'shadowColor') note(value);
      state.set(prop, value);
      return true;
    },
  }) as unknown as Canvas2D;
  return { ctx, colours, unread };
}

function dataColours(model: unknown): Set<string> {
  const found = new Set<string>();
  const visit = (value: unknown) => {
    if (typeof value === 'string') { if (parseColorAlpha(value)) found.add(key(value)); return; }
    if (Array.isArray(value)) { for (const item of value) visit(item); return; }
    if (value && typeof value === 'object') for (const item of Object.values(value)) visit(item);
  };
  visit(model);
  return found;
}

function assertThemed(label: string, paint: (ctx: Canvas2D, theme: FamilyTheme) => unknown): void {
  const drawn = [LIGHT, DARK].map(theme => {
    const { ctx, colours, unread } = recorder();
    const model = paint(ctx, theme);
    assert.deepEqual([...unread], [], `${label}: every painted colour is a hex or rgb colour this test can compare`);
    assert.ok(colours.has(key(theme.text)) || colours.has(key(theme.muted)), `${label}: the theme's own text is drawn`);
    return { colours, data: dataColours(model) };
  });
  const [light, dark] = drawn as [typeof drawn[0], typeof drawn[0]];
  const fixed = [...light.colours].filter(colourKey => dark.colours.has(colourKey) && !(light.data.has(colourKey) && dark.data.has(colourKey)));
  assert.deepEqual(fixed, [], `${label}: a colour drawn the same under two unrelated themes was not derived from either`);
  for (const { colours, data } of drawn) {
    const stale = DEFAULT_LITERALS.filter(literal => colours.has(literal) && !data.has(literal));
    assert.deepEqual(stale, [], `${label}: a colour of the default dark theme is drawn under a custom theme`);
  }
}

test('the two custom themes are far from the default and from each other', () => {
  assert.notEqual(key(LIGHT.surface), key(DARK.surface));
  assert.ok(!DEFAULT_LITERALS.includes(key(LIGHT.text)) && !DEFAULT_LITERALS.includes(key(DARK.text)));
  assert.equal(LIGHT.dark, false);
  assert.equal(DARK.dark, true);
});

test('bars draw every colour from the theme or the palette for its surface', () => {
  assertThemed('bars', (ctx, theme) => {
    const model: BarsModel = {
      form: 'ordered', hour: 100, scaleMax: 300, scaleUnit: 'Factories', showAxis: true,
      rows: [
        { kind: 'data', key: 'A', label: 'Alpha', media: { kind: 'none', art: NO_ART }, color: colour(theme.surface, 0), value: { value: knownValue(290), format: 'integer' }, numeric: 290 },
        { kind: 'data', key: 'B', label: 'Beta', media: { kind: 'none', art: NO_ART }, color: colour(theme.surface, 1), value: { value: partialValue(120, 'A state unread'), format: 'integer' }, numeric: 120 },
        { kind: 'data', key: 'C', label: 'Gamma', media: { kind: 'none', art: NO_ART }, color: colour(theme.surface, 2), value: { value: zeroValue(0), format: 'integer' }, numeric: 0 },
        { kind: 'data', key: 'D', label: 'Delta', media: { kind: 'none', art: NO_ART }, color: colour(theme.surface, 3), value: { value: notRecorded('Country unread') }, numeric: null },
      ],
    };
    drawFamilyBars(ctx, { model, layout: layoutFamilyBars(model, 540, 260, theme), theme, scrollTop: 0, now: 0, glideDuration: 0 });
    return model;
  });
});

test('cards draw every colour from the theme', () => {
  assertThemed('card', (ctx, theme) => {
    const model: CardModel = {
      hour: 10,
      rows: [
        { kind: 'item', key: 'a', title: { kind: 'known', value: 'First' }, date: { hour: 5 }, place: 'Somewhere' },
        { kind: 'item', key: 'b', title: notRecorded('Name not recorded'), date: { hour: null, reason: 'Date not recorded' }, place: 'Elsewhere' },
      ],
      columns: { date: true, flag: true, place: true, figure: 170, state: 120 },
      rowHeight: 36,
    };
    drawCards(ctx, { model, layout: layoutCards(model, 640, 300), theme, scrollTop: 0, inspectedKey: null, selectedKey: null, focusedKey: null });
    return model;
  });
});

test('a flow draws its parts from the palette for its surface and its chrome from the theme', () => {
  assertThemed('flow', (ctx, theme) => {
    const node = (key: string, side: 'left' | 'centre' | 'right', numeric: number | null) => ({ key, side, label: key, numeric, figure: numeric === null ? 'Not recorded' : String(numeric), spoken: key, audit: key });
    const model: FlowModel = {
      hour: 10, mode: 'centred',
      left: [node('left:a', 'left', 26), node('left:b', 'left', null)],
      centre: node('centre:c', 'centre', 26),
      right: [node('right:d', 'right', 40)],
      ribbons: [
        { key: 'r1', from: 'left:a', to: 'centre:c', bands: [{ part: 0, numeric: 20 }, { part: 1, numeric: 6 }], numeric: 26, audit: 'r1' },
        { key: 'r2', from: 'centre:c', to: 'right:d', bands: [{ part: 0, numeric: 40 }], numeric: 40, audit: 'r2' },
      ],
      parts: [{ key: 'oil', label: 'Oil', ordinal: 0 }, { key: 'steel', label: 'Steel', ordinal: 1 }],
      notRecordedKey: true,
    };
    drawFlow(ctx, { model, layout: layoutFlow(model, 846, 360), theme, inspected: 'r1', focused: null });
    return model;
  });
});

test('a heat matrix draws every colour from the theme or its model', () => {
  assertThemed('heat matrix', (ctx, theme) => {
    const rows: MatrixAxisEntity[] = [{ id: 'ENG', label: 'United Kingdom', tag: 'ENG' }, { id: 'GER', label: 'German Reich', tag: 'GER' }];
    const columns: MatrixAxisEntity[] = [
      { id: 'ENG', label: 'United Kingdom', tag: 'ENG', group: { id: 'f:1', label: 'Allies', color: colour(theme.surface, 0) } },
      { id: 'GER', label: 'German Reich', tag: 'GER', group: { id: 'f:2', label: 'Axis', color: colour(theme.surface, 1) } },
    ];
    const model: HeatMatrixModel = {
      hour: 240, rows, columns,
      cells: { present: new Map([[1, { kind: 'value', color: colour(theme.surface, 2), text: '+50', value: 50 }], [2, { kind: 'missing', reason: 'Opinions not recorded' }]]), absent: { kind: 'empty' }, self: { kind: 'diagonal' } },
      legend: { entries: [{ id: 'missing', label: 'Not recorded', color: colour(theme.surface, 3), hatched: true }] },
    };
    const layout = layoutHeatMatrix({ width: 520, height: 260, scrollLeft: 0, scrollTop: 0, rows, columns, legend: model.legend, widestRowName: 96 });
    drawHeatMatrix(ctx, { model, layout, theme, scrollLeft: 0, scrollTop: 0, hoveredCell: { row: 0, column: 1 }, focusedCell: null });
    return model;
  });
});

test('meters draw every colour from the theme or the palette for its surface', () => {
  assertThemed('meter', (ctx, theme) => {
    const model: MeterModel = {
      hour: 10, layout: 'rows',
      sections: [{ key: 's', title: 'Subjects', count: '3', rows: [
        { key: 'a', name: 'Alpha', bar: { kind: 'track', fraction: 0.6, color: colour(theme.surface, 0) }, figure: '60%', spoken: 'a', audit: 'a', sparkline: [1, 2, null, 4] },
        { key: 'b', name: 'Beta', bar: { kind: 'threshold', fraction: 0.3, ticks: [{ at: 0.5, reached: false }] }, figure: '30%', spoken: 'b', audit: 'b' },
        { key: 'c', name: 'Gamma', muted: true, bar: { kind: 'track', fraction: null }, figureMissing: true, spoken: 'c', audit: 'c' },
      ] }],
    };
    drawMeterModel(ctx, { layout: layoutMeterModel(ctx, model, 600, theme), width: 600, height: 300, scrollLeft: 0, scrollTop: 0, theme, inspectedKey: 'a', selectedKey: null, focusedKey: null });
    return model;
  });
});

test('a table draws every colour from the theme', () => {
  assertThemed('table', (ctx, theme) => {
    const columns: TableColumnSpec[] = [
      { key: 'name', label: 'Country', as: 'plain', offered: ['plain'], priority: 0, text: true, widest: ['Country 999'] },
      { key: 'v1', label: 'Score', as: 'faint', tone: 'blue', offered: ['plain', 'faint'], priority: 1, numeric: true, format: 'integer', widest: ['999,999'] },
      { key: 'v2', label: 'Rate', as: 'track', tone: 'grey', offered: ['plain', 'track'], priority: 2, numeric: true, format: 'decimal1', widest: ['999.9'] },
    ];
    const settings = tableSettingsOf(undefined, columns, { density: 'comfortable', sort: { column: 'v1', direction: 'descending' } });
    const layout = layoutFamilyTable(buildColumnLayouts(columns, settings), 480, { rankShown: true, rankWidth: 24 });
    const rows = resolveTableRows({
      columns, hour: 1,
      rows: [0, 1, 2].map(index => ({
        key: `C${index}`, name: `Country ${index}`,
        cells: {
          name: { value: knownValue(`Country ${index}`) },
          v1: { value: knownValue(100 - index * 10), format: 'integer' },
          v2: { value: index === 2 ? notRecorded('No data') : knownValue(index + 0.5), format: 'decimal1', bar: { fraction: knownValue(0.4) } },
        },
      })),
    }, settings);
    drawFamilyTable(ctx, { layout, rows, settings, hour: 1, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: true, rankWidth: 24, theme, now: 1, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: 'C1', selectedKey: 'C0', focusedKey: null, dpr: 1 }, createTablePicture());
    return rows;
  });
});

test('a time chart draws every colour from the theme or the palette for its surface', () => {
  assertThemed('time chart', (ctx, theme) => {
    const points = (scale: number) => Array.from({ length: 12 }, (_, at) => ({ hour: at * 24, value: at === 6 ? null : (at - 4) * scale }));
    const model: TimeChartModel = {
      kind: 'theme-test', measureTitle: 'Balance', mode: 'lines', hour: 11 * 24, extent: [0, 11 * 24], yAxis: { format: 'power' },
      series: [
        { id: 'a', key: 'a', name: 'Alpha', color: colour(theme.surface, 0), role: 'primary', negRed: true, points: points(3), headValue: 21, headHour: 11 * 24, endLabel: { name: 'Alpha', fullName: 'Alpha', value: '21' } },
        { id: 'b', key: 'b', name: 'Beta', color: colour(theme.surface, 1), role: 'secondary', points: points(-2), headValue: -14, headHour: 11 * 24 },
      ],
    };
    const layout = layoutTimeChart(model, 600, 220, theme.font, text => text.length * 6.5);
    drawTimeChart(ctx, model, layout, theme, `plot:${5 * 24}`, 1);
    return model;
  });
});

test('a timeline draws every colour from the theme or the palette for its surface', () => {
  assertThemed('timeline', (ctx, theme) => {
    const model: TimelineModel = {
      hour: 2000 as TimelineModel['hour'], firstHour: 0, unobserved: [{ firstHour: 600, lastHour: 700 }], iconsOnBars: false, labelWidth: 120, warnings: [], nextChangeAt: Infinity,
      lanes: [
        { key: 'g', kind: 'group', label: 'Axis', expandable: true, defaultOpen: true, spans: [], auditValue: 'Axis' },
        { key: 'm', kind: 'item', label: 'Italy', parentKey: 'g', depth: 1, auditValue: 'Italy', spans: [
          { id: 's', firstHour: 100, lastHour: 900, color: colour(theme.surface, 0), open: false, label: 'Held' },
          { id: 't', firstHour: 1200, lastHour: 2000, color: colour(theme.surface, 1), open: true, heldBefore: false },
        ] },
      ],
    };
    const layout = frameTimeline(placeTimeline(model, 640, 240, {}), 2000);
    drawTimeline(ctx, { model, layout, theme, scrollTop: 0, expanded: {}, inspectedKey: null, selectedKey: null, focusedKey: null, resolution: resolutionFor(null, model, layout) });
    return model;
  });
});

test('a breakdown draws its parts from the palette for its surface and its chrome from the theme', () => {
  const wire = (key: string, ordinal: number, figure: string | null, extra: Partial<WirePart> = {}): WirePart => ({
    key, name: key, nameMissing: null, ordinal, other: false, amount: figure === null ? null : { numerator: figure, denominator: '1' },
    state: figure === null ? 'notRecorded' : 'known', reason: figure === null ? `${key} was not recorded` : null,
    share: null, parentShare: null, count: null, last: null, art: null, children: [], ...extra,
  });
  const breakdown: Breakdown = {
    total: { amount: { numerator: '600', denominator: '1' }, state: 'partial', reason: 'Spending was not recorded' },
    parts: [wire('Daily costs', 3, '300', { share: 0.5 }), wire('Decisions', 0, '200', { share: 0.33 }), wire('Other', 1, '60', { share: 0.1, other: true }), wire('Unread', 2, null)],
    unrecordedHours: 2,
  };
  const words = { unit: 'PP', verb: 'Spent', format: (numerator: bigint, denominator: bigint) => String(numerator / denominator) };
  const model = { hour: 10, rootLabel: 'Spent', total: breakdownTotal(breakdown, words), parts: breakdownParts(breakdown, words) };
  for (const view of ['parts', 'ring'] as const) {
    assertThemed(`breakdown ${view}`, (ctx, theme) => {
      const layout = layoutBreakdown({ view, level: breakdownLevel(model), width: 900, height: 340, theme, namesInRing: true, userExpanded: {}, measure: text => text.length * 6.5 });
      drawBreakdown(ctx, { layout, theme, hour: 10, scrollTop: 0, inspectedKey: 'Decisions', selectedKey: null, focusedKey: null });
      return model;
    });
  }
});

test('heat lanes draw every colour from the theme or the palette for its surface', () => {
  assertThemed('heat lanes', (ctx, theme) => {
    const model: HeatLanesModel = {
      hour: 480, firstHour: 0, lastHour: 480, dayHours: 24, playhead: 480, gaps: [{ firstHour: 200, lastHour: 240, reason: 'Not recorded' }],
      legend: { entries: [{ id: 'a', label: 'Held', color: colour(theme.surface, 0) }, { id: 'm', label: 'Not recorded', color: colour(theme.surface, 1), hatched: true }] },
      rows: [
        { id: 'r1', label: 'Normandy', runs: [
          { firstHour: 0, lastHour: 199, readHour: 0, cell: { kind: 'solid', color: colour(theme.surface, 0), label: 'Held' } },
          { firstHour: 240, lastHour: 480, readHour: 240, cell: { kind: 'missing', reason: 'Not recorded' } },
        ], readout: { kind: 'value', primary: '42%', swatch: colour(theme.surface, 0), tone: 'negative' } },
        { id: 'r2', label: 'Brittany', runs: [{ firstHour: 0, lastHour: 480, readHour: 0, cell: { kind: 'clear', label: 'Quiet' } }], readout: { kind: 'missing', reason: 'Not recorded' } },
      ],
    };
    const layout = layoutHeatLanes({ width: 640, height: 220, scrollTop: 0, firstHour: 0, lastHour: 480, displayedHour: 480, rows: model.rows, legend: model.legend });
    drawHeatLanes(ctx, { model, layout, theme, scrollTop: 0, hoveredRow: 0, hoveredHour: 100, selectedRowId: 'r2', focusedRowId: null });
    return model;
  });
});

test('shortage and good marks are one derived pair: the approved shades on Carbon, readable on light themes, recoloured by every theme', () => {
  const carbon = themeColors(BUILT_IN_THEMES[0], 'dark');
  assert.deepEqual(markColours(carbon.error, carbon.positive), { shortFill: '#e5484d', shortInk: '#ff6b6f', goodFill: '#4fbf7a', goodInk: '#6fd496' });
  const lights = [LIGHT, ...BUILT_IN_THEMES.map(theme => familyThemeOf(themeColors(theme, 'light')))];
  for (const theme of lights) {
    const tokens = resolveKitTokens(theme);
    for (const ink of [tokens.shortage, tokens.goodInk]) assert.ok(contrast(parseColor(ink)!, parseColor(theme.surface)!) >= MARK_CONTRAST, `${ink} is readable on ${theme.surface}`);
  }
  assertThemed('status marks', (ctx, theme) => {
    drawWinRateCell(ctx, 0, 200, 20, 3, 7, theme);
    drawWinRateCell(ctx, 0, 200, 50, 8, 2, theme);
    drawNetBar(ctx, 0, 120, 70, 30, -0.5, theme);
    drawNetBar(ctx, 0, 120, 100, 30, 0.5, theme);
    ctx.fillStyle = theme.text;
    return null;
  });
});
