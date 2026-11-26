import assert from 'node:assert/strict';
import test from 'node:test';
import { ellipsise } from './labels.ts';
import {
  TextMeasureScope,
  canvasMeasureText,
  clearTextWidthCache,
  ellipsiseCanvasText,
  fitCanvasLine,
  fitMeasuredLine,
  measureCanvasText,
  textWidthCacheStats,
} from './textWidth.ts';
import { fitTooltip } from './tooltip.ts';
import { createLaneView, type LaneBinding, type LaneFamily } from '../families/laneRuntime.ts';
import type { FamilyTheme } from '../families/contract.ts';
import { registerFamilyProjection } from '../families/registry.ts';
import { drawMeterModel, layoutMeterModel } from '../families/meter/painter.ts';
import { createTablePicture, disposeTablePicture, drawFamilyTable, NO_GLIDE } from '../families/table/painter.ts';
import { buildColumnLayouts, computeRowLayout, layoutFamilyTable } from '../families/table/layout.ts';
import { resolveTableRows, type TableColumnSpec } from '../families/table/model.ts';
import { tableSettingsOf } from '../families/table/settings.ts';
import { layoutCards } from '../families/card/layout.ts';
import type { CardModel } from '../families/card/model.ts';
import { drawCards } from '../families/card/painter.ts';
import type { TimeChartModel } from '../families/time-chart/model.ts';
import { layoutChartRows } from '../families/time-chart/rows.ts';

const TEST_THEME: FamilyTheme = {
  surface: '#16181d',
  raised: '#1e2128',
  text: '#eef1f6',
  muted: '#8c95a6',
  line: '#2e3440',
  lineSoft: '#222630',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#8c95a6',
  hatch: '#6e7681',
  hatchEdge: '#484f58',
  font: 'Inter',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
};

test('TextMeasureScope bounds shared text measurements to what is on screen without a made-up cap', () => {
  clearTextWidthCache();
  let calls = 0;
  const ctx = {
    font: '400 12px Inter, sans-serif',
    measureText(text: string) {
      calls++;
      return { width: text.length * 7 } as TextMetrics;
    },
  };
  const widgetA = new TextMeasureScope();
  const widgetB = new TextMeasureScope();
  const hot = ['Germany', 'Soviet Union', 'United Kingdom', 'United States'];

  widgetA.run(() => {
    for (const label of hot) measureCanvasText(ctx, label, '500 12px Inter, sans-serif');
    for (let i = 0; i < 4500; i++) {
      measureCanvasText(ctx, `val-${i}`, `400 ${10 + (i % 80)}px Font${i % 80}, sans-serif`);
    }
  });
  assert.equal(textWidthCacheStats().entries, 4504);
  assert.equal(textWidthCacheStats().fonts, 81);

  widgetB.run(() => {
    measureCanvasText(ctx, 'Germany', '500 12px Inter, sans-serif');
    measureCanvasText(ctx, 'Japan', '500 12px Inter, sans-serif');
  });
  assert.equal(textWidthCacheStats().entries, 4505);

  const beforeReuse = calls;
  widgetA.run(() => {
    for (const label of hot) measureCanvasText(ctx, label, '500 12px Inter, sans-serif');
  });
  assert.equal(calls, beforeReuse, 'retained labels stay cached without re-measuring');
  assert.equal(textWidthCacheStats().entries, 5, 'transient strings and fonts drop when shown strings change');
  assert.equal(textWidthCacheStats().fonts, 1);

  widgetA.release();
  assert.equal(textWidthCacheStats().entries, 2, 'shared entry stays cached while another widget still shows it');

  widgetB.begin();
  measureCanvasText(ctx, 'Aborted Label', '500 12px Inter, sans-serif');
  widgetB.abort();
  assert.equal(textWidthCacheStats().entries, 2, 'aborted pass drops new entries and keeps previous on-screen entries');

  widgetB.release();
  assert.equal(textWidthCacheStats().entries, 0, 'releasing the last widget drops all entries');
  assert.equal(textWidthCacheStats().fonts, 0);
});

test('fitCanvasLine reuses width intervals without native font reads or re-measuring and matches uncached fitLine', () => {
  clearTextWidthCache();
  let fontReads = 0;
  let measureCalls = 0;
  let currentFont = '400 12px Inter, sans-serif';
  const weights = (text: string) => {
    let w = 0;
    for (const part of new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text)) {
      w += part.segment === '\u2026' ? 9 : part.segment === ' ' ? 4 : 7.5;
    }
    if (text.includes('AV')) w -= 3;
    return w;
  };
  const ctx = {
    get font() { fontReads++; return currentFont; },
    set font(value: string) { currentFont = value; },
    measureText(text: string) { measureCalls++; return { width: weights(text) } as TextMetrics; },
  };
  const font = '500 11.5px Inter, sans-serif';
  const scope = new TextMeasureScope();
  const samples = [
    'Basic Machine Tools',
    'AVATAR Research Program',
    '大日本帝国陸軍省特別攻撃隊',
    '👨‍👩‍👧‍👦🇺🇸abc',
    'المملكة العربية السعودية',
    'Trailing   spaces',
  ];

  const referenceFit = (text: string, width: number) => fitMeasuredLine(text, width, font, sample => weights(sample));

  for (const sample of samples) {
    for (const width of [-5, 0, 5, 8.9, 9, 15, 32.4, 48.2, 67.5, 95, 140, 400, Infinity, Number.NaN]) {
      const actual = scope.run(() => fitCanvasLine(ctx, sample, width, font));
      const expected = referenceFit(sample, width);
      assert.deepEqual(actual, expected, `fitCanvasLine(${JSON.stringify(sample)}, ${width})`);
      assert.equal(
        scope.run(() => ellipsiseCanvasText(ctx, sample, width, font)),
        ellipsise(sample, width, weights),
      );
    }
  }

  scope.run(() => fitCanvasLine(ctx, 'Basic Machine Tools', 80, font));
  const readsAfterWarm = fontReads;
  const callsAfterWarm = measureCalls;
  for (let w = 79.9; w >= 77.0; w -= 0.3) {
    const fitted = scope.run(() => fitCanvasLine(ctx, 'Basic Machine Tools', w, font));
    assert.deepEqual(fitted, referenceFit('Basic Machine Tools', w));
  }
  assert.equal(fontReads, readsAfterWarm, 'interval hits never read ctx.font');
  assert.equal(measureCalls, callsAfterWarm, 'interval hits never call measureText');

  scope.run(() => {
    const a = fitCanvasLine(ctx, 'Basic Machine Tools', 50, font);
    const b = fitCanvasLine(ctx, 'Basic Machine Tools', 90, font);
    assert.deepEqual(a, referenceFit('Basic Machine Tools', 50));
    assert.deepEqual(b, referenceFit('Basic Machine Tools', 90));
  });
  const readsBeforeTwo = fontReads;
  const callsBeforeTwo = measureCalls;
  scope.run(() => {
    fitCanvasLine(ctx, 'Basic Machine Tools', 50, font);
    fitCanvasLine(ctx, 'Basic Machine Tools', 90, font);
  });
  assert.equal(fontReads, readsBeforeTwo);
  assert.equal(measureCalls, callsBeforeTwo, 'multiple truncated widths in one pass stay cached without ping-pong');

  scope.run(() => {
    fitCanvasLine(ctx, '大日本帝国陸軍省特別攻撃隊', 40, font);
  });
  assert.equal(textWidthCacheStats().graphemes, 1, 'non-ASCII grapheme entry is cached while active');

  scope.release();
  assert.equal(textWidthCacheStats().entries, 0);
  assert.equal(textWidthCacheStats().graphemes, 0, 'grapheme entries drop with the widget scope');
});

test('createLaneView drops cached text entries when a widget changes its shown strings or unmounts', () => {
  clearTextWidthCache();
  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    private readonly ctx = {
      font: '400 12px Inter',
      save() {},
      restore() {},
      setTransform() {},
      measureText(text: string) { return { width: text.length * 6 }; },
    };
    constructor(width: number, height: number) {
      this.width = width;
      this.height = height;
    }
    getContext() { return this.ctx; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    interface ScopeOptions { readonly labels: readonly string[]; readonly skipPaint?: boolean }
    const kind = 'text-scope-test-kind';
    registerFamilyProjection({
      family: 'card',
      kind,
      project: (_view: unknown, options: unknown) => options as ScopeOptions,
    });
    const family: LaneFamily<LaneBinding<ScopeOptions>, ScopeOptions, { readonly labels: readonly string[] }> = {
      family: 'card',
      boundary: () => Infinity,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout(model, _binding, _surface, theme, measure) {
        const font = `400 12px ${theme.font}`;
        for (const label of model.labels) measureCanvasText(measure, `layout:${label}`, font);
        return { labels: model.labels };
      },
      paint(ctx, frame) {
        if (frame.model.skipPaint) return { drew: false, next: false };
        const font = `500 11.5px ${frame.theme.font}`;
        for (const label of frame.layout.labels) fitCanvasLine(ctx, label, 48, font);
        return { drew: true, next: false };
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };
    const view = createLaneView(family, () => {});
    const state = new Map<string, unknown>();
    let serial = 0;
    const paint = (options: ScopeOptions, rev: number) => view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: {
        family: 'card',
        kind,
        options,
        sources: {},
        scrollTop: 0,
        theme: TEST_THEME,
        revisions: { options: rev, sources: 1, theme: 1 },
      },
      hour: rev,
      seek: 0,
      speed: 0,
      binding: { generation: 1, surfaces: [{ width: 320, height: 180, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: rev * 100,
    }, state, () => `c:${++serial}`);

    paint({ labels: ['Alpha Research', 'Beta Research'] }, 1);
    const afterFirst = textWidthCacheStats().entries;
    assert.ok(afterFirst >= 4);

    paint({ labels: ['Alpha Research', 'Beta Research'], skipPaint: true }, 2);
    assert.equal(textWidthCacheStats().entries, afterFirst, 'skipped paint preserves previously drawn strings');

    paint({ labels: ['Gamma Research'] }, 3);
    assert.ok(textWidthCacheStats().entries < afterFirst, 'removed strings drop after repaint');

    view.unmount?.(1, state);
    assert.equal(textWidthCacheStats().entries, 0, 'unmount releases all widget text entries');
    assert.equal(textWidthCacheStats().fonts, 0);

    paint({ labels: ['Still Held'] }, 4);
    assert.ok(textWidthCacheStats().entries > 0);
    const refused = view.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: {
        family: 'card',
        kind: 'nonexistent-unknown-kind',
        options: { labels: [] },
        sources: {},
        scrollTop: 0,
        theme: TEST_THEME,
        revisions: { options: 5, sources: 1, theme: 1 },
      },
      hour: 5,
      seek: 0,
      speed: 0,
      binding: { generation: 2, surfaces: [{ width: 320, height: 180, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 500,
    }, state, () => `c:${++serial}`);
    assert.equal(refused.mode, 'refused');
    assert.equal(textWidthCacheStats().entries, 0, 'unknown-projection refusal releases prior picture entries');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }
});

test('unscoped misses do not accumulate, nested re-entry keeps ownership, and multi-widget fit intervals do not ping-pong', () => {
  clearTextWidthCache();
  let measureCalls = 0;
  const ctx = {
    font: '400 12px Inter, sans-serif',
    measureText(text: string) {
      measureCalls++;
      return { width: [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(text)].length * 7 } as TextMetrics;
    },
  };
  const font = '500 12px Inter, sans-serif';

  for (let i = 0; i < 200; i++) measureCanvasText(ctx, `unscoped-${i}`, font);
  assert.equal(textWidthCacheStats().entries, 0, 'unscoped misses never insert permanently');

  const scopeA = new TextMeasureScope();
  const scopeB = new TextMeasureScope();
  scopeA.run(() => {
    scopeB.run(() => {
      scopeA.run(() => {
        measureCanvasText(ctx, 'Nested Owned By A', font);
      });
    });
  });
  scopeB.release();
  assert.equal(textWidthCacheStats().entries, 1, 'inner re-entry keeps entry owned by outer scope A');
  const beforeUnscopedHit = measureCalls;
  assert.equal(measureCanvasText(ctx, 'Nested Owned By A', font), 17 * 7);
  assert.equal(measureCalls, beforeUnscopedHit, 'unscoped lookup still reads shared cache hits');

  scopeA.run(() => fitCanvasLine(ctx, 'United Kingdom', 39, font));
  scopeB.run(() => fitCanvasLine(ctx, 'United Kingdom', 68, font));
  const afterWarm = measureCalls;
  for (let i = 0; i < 4; i++) {
    assert.equal(scopeA.run(() => fitCanvasLine(ctx, 'United Kingdom', 39, font)).text, 'Unit\u2026');
    assert.equal(scopeB.run(() => fitCanvasLine(ctx, 'United Kingdom', 68, font)).text, 'United K\u2026');
  }
  assert.equal(measureCalls, afterWarm, 'two widgets fitting the same string at different widths never ping-pong');

  scopeA.run(() => fitCanvasLine(ctx, '大日本帝国陸軍省特別攻撃隊', 44, font));
  assert.equal(textWidthCacheStats().graphemes, 1);
  scopeA.run(() => fitCanvasLine(ctx, '大日本帝国陸軍省特別攻撃隊', 43, font));
  assert.equal(textWidthCacheStats().graphemes, 1, 'non-ASCII grapheme entry stays cached across interval hits');

  scopeA.release();
  scopeB.release();
  assert.equal(textWidthCacheStats().entries, 0);
  assert.equal(textWidthCacheStats().graphemes, 0);

  const tooltipMeasure = canvasMeasureText(ctx);
  const rows = Array.from({ length: 200 }, (_, i) => ({ label: `Row ${i}`, value: `${i * 10}` }));
  fitTooltip([{ title: 'Large Tooltip', rows }], 160, {
    metrics: { measure: tooltipMeasure, family: 'Inter, sans-serif', sizes: { xs: 11, sm: 12, md: 13 } },
  });
  assert.equal(textWidthCacheStats().entries, 0, 'fitTooltip releases all candidate measurements on return');
});

test('counted meter headers preserve 400 11.5px title font, meter layout drops off-screen figures, and table partial paint preserves unchanged cells', () => {
  clearTextWidthCache();
  const drawnHeaders: { text: string; font: string; x: number }[] = [];
  const makeFontCtx = () => {
    const stack: { font: string }[] = [];
    const raw = {
      font: '400 12px Inter',
      fillStyle: '',
      strokeStyle: '',
      lineWidth: 1,
      globalAlpha: 1,
      textAlign: 'left' as CanvasTextAlign,
      textBaseline: 'middle' as CanvasTextBaseline,
      save() { stack.push({ font: raw.font }); },
      restore() { const prev = stack.pop(); if (prev) raw.font = prev.font; },
      beginPath() {},
      closePath() {},
      clip() {},
      rect() {},
      translate() {},
      moveTo() {},
      lineTo() {},
      stroke() {},
      fill() {},
      fillRect() {},
      clearRect() {},
      roundRect() {},
      createLinearGradient() { return { addColorStop() {} }; },
      fillText(text: string, x: number) { drawnHeaders.push({ text, font: raw.font, x }); },
      measureText(text: string) {
        const px = Number.parseFloat(/\b(\d+(?:\.\d+)?)px/.exec(raw.font)?.[1] ?? '12') || 12;
        const bold = /\b600\b/.test(raw.font) ? 1.15 : 1;
        return { width: text.length * px * 0.55 * bold };
      },
    };
    return raw as unknown as CanvasRenderingContext2D;
  };
  const meterCtx = makeFontCtx();
  const layoutScope = new TextMeasureScope();
  const largeModel = {
    layout: 'columns' as const,
    hour: 100,
    sections: [{
      key: 's1',
      title: 'Header label',
      count: '3',
      rows: Array.from({ length: 120 }, (_, i) => ({
        key: `r${i}`,
        name: `Item ${i}`,
        figure: `${i}.5%`,
        secondary: `+${i}/d`,
        bar: { kind: 'none' as const },
        spoken: '',
        audit: '',
      })),
    }],
  };
  const meterLayout = layoutScope.run(() => layoutMeterModel(meterCtx, largeModel, 124, TEST_THEME));
  assert.equal(textWidthCacheStats().entries, 0, 'meter layout scan drops off-screen figure widths');
  layoutScope.release();

  const paintScope = new TextMeasureScope();
  paintScope.run(() => drawMeterModel(meterCtx, {
    layout: meterLayout,
    width: 124,
    height: 30,
    scrollLeft: 0,
    scrollTop: 0,
    theme: TEST_THEME,
    inspectedKey: null,
    selectedKey: null,
    focusedKey: null,
  }));
  const titleDraw = drawnHeaders.find(d => d.text.startsWith('Header'));
  const countDraw = drawnHeaders.find(d => d.text === '3');
  assert.ok(titleDraw && countDraw);
  assert.equal(titleDraw.font, `400 11.5px ${TEST_THEME.font}`, 'counted header preserves effective 400 11.5px title font');
  assert.equal(titleDraw.text, 'Header label');
  paintScope.release();

  clearTextWidthCache();
  const columns: TableColumnSpec[] = [
    { key: 'name', label: 'Country', as: 'plain', offered: ['plain'], priority: 0, text: true, widest: ['Country 9'] },
    { key: 'val', label: 'Score', as: 'plain', offered: ['plain'], priority: 1, numeric: true, format: 'integer', widest: ['999'] },
  ];
  const settings = tableSettingsOf(undefined, columns, { density: 'comfortable', sort: { column: 'val', direction: 'descending' } });
  const baseCols = buildColumnLayouts(columns, settings);
  const tableCtx = makeFontCtx();
  const tableLayout = layoutFamilyTable(baseCols, 320, { rankShown: false, rankWidth: 0 });
  const picture = createTablePicture();
  const makeTableRows = (v2: number) => resolveTableRows({
    columns,
    hour: 1,
    rows: [
      { key: 'a', name: 'Alpha', cells: { name: { value: { kind: 'known', value: 'Alpha' } }, val: { value: { kind: 'known', value: 100 }, format: 'integer' } } },
      { key: 'b', name: 'Beta', cells: { name: { value: { kind: 'known', value: 'Beta' } }, val: { value: { kind: 'known', value: v2 }, format: 'integer' } } },
    ],
  }, settings);
  const paintTable = (v2: number, hour: number, width = 320) => drawFamilyTable(tableCtx, {
    layout: tableLayout,
    rows: makeTableRows(v2),
    settings,
    hour,
    width,
    height: 160,
    scrollLeft: 0,
    scrollTop: 0,
    rankShown: false,
    rankWidth: 0,
    theme: TEST_THEME,
    now: hour,
    glideDuration: 0,
    glide: NO_GLIDE,
    markers: new Map(),
    inspectedKey: null,
    selectedKey: null,
    focusedKey: null,
    dpr: 1,
  }, picture);

  paintTable(50, 1);
  const afterFull = textWidthCacheStats().entries;
  assert.ok(afterFull >= 4);
  const partial = paintTable(75, 2);
  assert.equal(partial.partial, true);
  assert.equal(partial.paintedCells, 1);
  assert.equal(textWidthCacheStats().entries, afterFull, 'partial table paint retains unchanged visible cells and drops replaced cell value');
  disposeTablePicture(picture);
  assert.equal(textWidthCacheStats().entries, 0, 'disposing table picture releases all cell scopes');

  const subPixelScope = new TextMeasureScope();
  const exactCtx = {
    font: '400 12px Inter',
    measureText(text: string) { return { width: text.length * 7.1 } as TextMetrics; },
  };
  const wide = subPixelScope.run(() => ellipsiseCanvasText(exactCtx, 'abcdef', 28.5, '400 12px Inter'));
  const narrow = subPixelScope.run(() => ellipsiseCanvasText(exactCtx, 'abcdef', 28.25, '400 12px Inter'));
  assert.equal(wide, 'abc\u2026');
  assert.equal(narrow, 'ab\u2026', 'exact sub-pixel width budgets never reuse a stale floored fit');
  subPixelScope.release();
});

test('off-screen content, scrolling, pinned overlays, otherFits, and nested release/abort stay bounded', () => {
  const makeTrackedCtx = () => {
    let calls = 0;
    const ctx = {
      font: '400 12px Inter',
      fillStyle: '',
      strokeStyle: '',
      lineWidth: 1,
      globalAlpha: 1,
      textAlign: 'left' as CanvasTextAlign,
      textBaseline: 'alphabetic' as CanvasTextBaseline,
      save() {},
      restore() {},
      beginPath() {},
      rect() {},
      clip() {},
      clearRect() {},
      fillRect() {},
      strokeRect() {},
      moveTo() {},
      lineTo() {},
      stroke() {},
      fill() {},
      roundRect() {},
      translate() {},
      fillText() {},
      createLinearGradient() {
        return { addColorStop() {} };
      },
      measureText(text: string) {
        calls++;
        return { width: text.length * 7 } as TextMetrics;
      },
    } as unknown as CanvasRenderingContext2D;
    return { ctx, calls: () => calls };
  };

  clearTextWidthCache();
  const wideCols: TableColumnSpec[] = Array.from({ length: 80 }, (_, i) => ({
    key: `c${i}`,
    label: `Col ${i}`,
    as: 'plain' as const,
    offered: ['plain' as const],
    priority: i,
    text: true,
    minWidth: 60,
    widest: [`Cell-${i}`],
  }));
  const wideSettings = tableSettingsOf(undefined, wideCols, { density: 'comfortable', sort: { column: 'c0', direction: 'ascending' } });
  const wideLayout = layoutFamilyTable(buildColumnLayouts(wideCols, wideSettings), 260, { rankShown: false, rankWidth: 0 });
  const wideRows = resolveTableRows({
    columns: wideCols,
    hour: 1,
    rows: [{
      key: 'row-0',
      name: 'Row 0',
      cells: Object.fromEntries(wideCols.map((c, i) => [c.key, { value: { kind: 'known' as const, value: `Cell-val-${i}` } }])),
    }],
  }, wideSettings);
  const widePic = createTablePicture();
  const { ctx: wideCtx } = makeTrackedCtx();
  drawFamilyTable(wideCtx, {
    layout: wideLayout,
    rows: wideRows,
    settings: wideSettings,
    hour: 1,
    width: 260,
    height: 80,
    scrollLeft: 0,
    scrollTop: 0,
    rankShown: false,
    rankWidth: 0,
    theme: TEST_THEME,
    now: 1,
    glideDuration: 0,
    glide: NO_GLIDE,
    markers: new Map(),
    inspectedKey: null,
    selectedKey: null,
    focusedKey: null,
    dpr: 1,
  }, widePic);
  assert.ok(textWidthCacheStats().entries <= 12, `260px table with 80 columns retains only visible columns, got ${textWidthCacheStats().entries}`);
  disposeTablePicture(widePic);
  assert.equal(textWidthCacheStats().entries, 0);

  clearTextWidthCache();
  const detailCols: TableColumnSpec[] = [
    { key: 'title', label: 'Title', as: 'plain', offered: ['plain'], priority: 0, text: true, minWidth: 160 },
    { key: 'detail', label: 'Detail', as: 'plain', offered: ['plain'], priority: 1, text: true, minWidth: 160 },
  ];
  const detailSettings = tableSettingsOf(undefined, detailCols, { density: 'comfortable', sort: { column: 'title', direction: 'ascending' } });
  const detailBase = buildColumnLayouts(detailCols, detailSettings);
  const detailFlat = resolveTableRows({
    columns: detailCols,
    hour: 1,
    rows: Array.from({ length: 300 }, (_, i) => ({
      key: `d-${i}`,
      name: `Detail ${i}`,
      detailColumn: 'detail',
      cells: {
        title: { value: { kind: 'known' as const, value: `Detail ${i}` } },
        detail: { value: { kind: 'known' as const, value: `Long detail text line number ${i} with unique content` } },
      },
    })),
  }, detailSettings);
  const detailLayoutScope = new TextMeasureScope();
  const { ctx: detailCtx } = makeTrackedCtx();
  detailLayoutScope.run(() => computeRowLayout(detailFlat, 'comfortable', 'standard', {
    width: 260,
    columns: detailBase,
    theme: TEST_THEME,
    measure: canvasMeasureText(detailCtx),
  }));
  assert.equal(textWidthCacheStats().entries, 0, 'table detail-row layout scan drops off-screen entries');
  detailLayoutScope.release();

  clearTextWidthCache();
  const chartModel: TimeChartModel = {
    kind: 'test-rows',
    measureTitle: 'Balances',
    mode: 'rows',
    hour: 48,
    extent: [0, 48],
    yAxis: { format: 'number' },
    series: [],
    rows: Array.from({ length: 1500 }, (_, i) => ({
      id: `r-${i}`,
      name: `Resource Balance Name ${i}`,
      color: TEST_THEME.accent,
      valueText: String(i),
      months: [{ firstHour: 0, lastHour: 47, value: i, valueText: String(i) }],
    })),
  };
  const rowsScope = new TextMeasureScope();
  const { ctx: rowsCtx } = makeTrackedCtx();
  const rowsFrame = rowsScope.run(() => layoutChartRows(chartModel, 480, 100, false, canvasMeasureText(rowsCtx), 'Inter'));
  assert.equal(rowsFrame.rows.rows.length, 1500);
  assert.ok(textWidthCacheStats().entries <= 8, `100px time-chart retains only intersecting rows, got ${textWidthCacheStats().entries}`);
  rowsScope.release();
  assert.equal(textWidthCacheStats().entries, 0);

  clearTextWidthCache();
  const cardModel: CardModel = {
    hour: 1,
    rowHeight: 32,
    columns: {},
    rows: [{
      kind: 'list',
      key: 'mods',
      label: 'Items',
      name: 'Items',
      items: Array.from({ length: 1500 }, (_, i) => `CandidateItem-${i}`),
      none: 'None',
    }],
  };
  const cardLayout = layoutCards(cardModel, 320, 80);
  const cardScope = new TextMeasureScope();
  const { ctx: cardCtx } = makeTrackedCtx();
  cardScope.run(() => drawCards(cardCtx, { model: cardModel, layout: cardLayout, theme: TEST_THEME, scrollTop: 0 }));
  assert.ok(textWidthCacheStats().entries <= 4, `card list retains only drawn string, not 1499 candidates, got ${textWidthCacheStats().entries}`);
  cardScope.release();
  assert.equal(textWidthCacheStats().entries, 0);

  clearTextWidthCache();
  const scrollCols: TableColumnSpec[] = [
    { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, minWidth: 120 },
    { key: 'val', label: 'Val', as: 'plain', offered: ['plain'], priority: 1, numeric: true, format: 'integer', minWidth: 80 },
  ];
  const scrollSettings = tableSettingsOf(undefined, scrollCols, { density: 'comfortable', sort: { column: 'val', direction: 'descending' } });
  const scrollLayout = layoutFamilyTable(buildColumnLayouts(scrollCols, scrollSettings), 260, { rankShown: false, rankWidth: 0 });
  const scrollRows = resolveTableRows({
    columns: scrollCols,
    hour: 1,
    rows: Array.from({ length: 20 }, (_, i) => ({
      key: `s-${i}`,
      name: `Country ${i}`,
      cells: {
        name: { value: { kind: 'known' as const, value: `Country ${i}` } },
        val: { value: { kind: 'known' as const, value: 1000 - i }, format: 'integer' as const },
      },
    })),
  }, scrollSettings);
  const scrollPic = createTablePicture();
  const scrollTracked = makeTrackedCtx();
  const paintScroll = (scrollTop: number) => drawFamilyTable(scrollTracked.ctx, {
    layout: scrollLayout,
    rows: scrollRows,
    settings: scrollSettings,
    hour: 1,
    width: 260,
    height: 140,
    scrollLeft: 0,
    scrollTop,
    rankShown: false,
    rankWidth: 0,
    theme: TEST_THEME,
    now: 1,
    glideDuration: 0,
    glide: NO_GLIDE,
    markers: new Map(),
    inspectedKey: null,
    selectedKey: null,
    focusedKey: null,
    dpr: 1,
  }, scrollPic);
  paintScroll(0);
  const callsAfterFirst = scrollTracked.calls();
  paintScroll(1);
  paintScroll(2);
  paintScroll(3);
  assert.equal(scrollTracked.calls(), callsAfterFirst, 'full table paint during scroll reuses surviving visible cells without re-measuring');
  disposeTablePicture(scrollPic);
  assert.equal(textWidthCacheStats().entries, 0);

  clearTextWidthCache();
  const pinCols: TableColumnSpec[] = [
    { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, minWidth: 100 },
    { key: 'val', label: 'Val', as: 'plain', offered: ['plain'], priority: 1, numeric: true, format: 'integer', minWidth: 100 },
    { key: 'extra', label: 'Extra', as: 'plain', offered: ['plain'], priority: 2, text: true, minWidth: 180 },
  ];
  const pinSettings = tableSettingsOf(undefined, pinCols, { density: 'comfortable', sort: { column: 'val', direction: 'descending' } });
  const pinLayout = layoutFamilyTable(buildColumnLayouts(pinCols, pinSettings, { name: 100, val: 100, extra: 180 }), 220, { rankShown: false, rankWidth: 0 });
  assert.ok(pinLayout.scrollable && pinLayout.pinnedWidth > 0, 'test table has scrollable pinned column');
  const pinPic = createTablePicture();
  const outerPaintScope = new TextMeasureScope();
  const { ctx: pinCtx } = makeTrackedCtx();
  const paintPinned = (label: string, hour: number) => outerPaintScope.run(() => drawFamilyTable(pinCtx, {
    layout: pinLayout,
    rows: resolveTableRows({
      columns: pinCols,
      hour,
      rows: [{
        key: 'only',
        name: label,
        cells: {
          name: { value: { kind: 'known' as const, value: label } },
          val: { value: { kind: 'known' as const, value: 42 }, format: 'integer' as const },
          extra: { value: { kind: 'known' as const, value: 'Right' } },
        },
      }],
    }, pinSettings),
    settings: pinSettings,
    hour,
    width: 220,
    height: 80,
    scrollLeft: 0,
    scrollTop: 0,
    rankShown: false,
    rankWidth: 0,
    theme: TEST_THEME,
    now: hour,
    glideDuration: 0,
    glide: NO_GLIDE,
    markers: new Map(),
    inspectedKey: null,
    selectedKey: null,
    focusedKey: null,
    dpr: 1,
  }, pinPic));
  paintPinned('Alpha', 1);
  const pinnedBaseline = textWidthCacheStats().entries;
  const pinPartial1 = paintPinned('Beta', 2);
  assert.equal(pinPartial1.partial, true);
  assert.equal(textWidthCacheStats().entries, pinnedBaseline, 'partial pinned repaint drops replaced pinned value from outer paint scope');
  const pinPartial2 = paintPinned('Gamma', 3);
  assert.equal(pinPartial2.partial, true);
  assert.equal(textWidthCacheStats().entries, pinnedBaseline);
  disposeTablePicture(pinPic);
  outerPaintScope.release();
  assert.equal(textWidthCacheStats().entries, 0);

  clearTextWidthCache();
  const fitTracked = makeTrackedCtx();
  const font = '400 12px Inter';
  const sharedLabel = 'Very long shared country label for fit slots';
  const holder = new TextMeasureScope();
  holder.run(() => measureCanvasText(fitTracked.ctx, sharedLabel, font));
  for (let i = 0; i < 120; i++) {
    const scopeA = new TextMeasureScope();
    const scopeB = new TextMeasureScope();
    scopeA.run(() => fitCanvasLine(fitTracked.ctx, sharedLabel, 40 + (i % 5) * 7, font));
    scopeB.run(() => fitCanvasLine(fitTracked.ctx, sharedLabel, 80 + (i % 5) * 7, font));
    scopeA.release();
    scopeB.run(() => fitCanvasLine(fitTracked.ctx, sharedLabel, 120 + (i % 5) * 7, font));
    scopeB.release();
  }
  const probeCallsBefore = fitTracked.calls();
  const probe = new TextMeasureScope();
  probe.run(() => fitCanvasLine(fitTracked.ctx, sharedLabel, 80, font));
  assert.ok(fitTracked.calls() > probeCallsBefore, 'released scopes leave no orphan fit intervals in otherFits');
  probe.release();
  holder.release();
  assert.equal(textWidthCacheStats().entries, 0);

  clearTextWidthCache();
  const nestTracked = makeTrackedCtx();
  const outer = new TextMeasureScope();
  const inner = new TextMeasureScope();
  outer.run(() => {
    measureCanvasText(nestTracked.ctx, 'outer-only', font);
    inner.run(() => {
      outer.release();
      measureCanvasText(nestTracked.ctx, 'inner-only', font);
    });
  });
  assert.equal(textWidthCacheStats().entries, 1, 'releasing outer scope inside inner run preserves inner scope ownership');
  inner.release();
  assert.equal(textWidthCacheStats().entries, 0, 'releasing inner scope cleans up entries measured after outer release');

  clearTextWidthCache();
  const abortTracked = makeTrackedCtx();
  const abortScope = new TextMeasureScope();
  abortScope.run(() => fitCanvasLine(abortTracked.ctx, 'Long label for abort check', 50, font));
  const afterCommitCalls = abortTracked.calls();
  assert.throws(() => {
    abortScope.run(() => {
      fitCanvasLine(abortTracked.ctx, 'Long label for abort check', 50, font);
      throw new Error('aborted pass');
    });
  }, /aborted pass/);
  abortScope.run(() => fitCanvasLine(abortTracked.ctx, 'Long label for abort check', 50, font));
  assert.equal(abortTracked.calls(), afterCommitCalls, 'aborting a pass that hit a committed fit preserves that fit interval');
  abortScope.release();
  assert.equal(textWidthCacheStats().entries, 0);
});
