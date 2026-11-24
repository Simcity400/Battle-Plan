import assert from 'node:assert/strict';
import test from 'node:test';
import type { LogCell, LogPageRow } from '../../generated/dashboard';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import type { FamilySources } from '../contract.ts';
import { knownValue, notRecorded, zeroValue } from '../value.ts';
import {
  buildLogLayout,
  LOG_MIN_TAIL_LIMIT,
  LOG_ROW_HEIGHT,
  logicalScrollFromPhysical,
  nominalRowHeight,
  physicalScrollFromLogical,
  resolveLogColumns,
  tailLimitForViewport,
  visibleItemRange,
} from './layout.ts';
import { logLookAt } from './declaration.ts';
import { LOG_FAMILY_OPTIONS, logFamilySettings } from './options.ts';
import {
  canonicalLogRowAudit,
  deriveLogAudit,
  deriveLogAuditRows,
  deriveLogMirrorItems,
  deriveLogTooltip,
} from './model.ts';
import { createLogPicture, drawLog, type LogTheme } from './painter.ts';
import { familyLogLaneView, LOG_FADE_DURATION_MS, type FamilyLogBinding, type LogMetaField, type LogPhasesField } from './laneView.ts';
import { reconcileFrozenView, verifyLogAuditSnapshot, type FrozenViewState, type LogAuditSnapshot } from './frozenView.ts';
import './registerAll.ts';
import {
  assembleFrozenPageWindow,
  mergeFrozenPageIntoTail,
  newestFirstWireRows,
  pageReplyToTail,
  scrollPageOrdinalRange,
  type LogTailReplyWire,
} from './tailReply.ts';
import { ledgerFacets, ledgerSelection, ledgerTitle, type LedgerItem } from '../../dashboard/marketLedger.ts';
import { OPTIONS as MARKET_OPTIONS } from '../../widgets/market-ledger/definition.ts';
import { marketLedgerProjection, type MarketLedgerProjectionOptions } from '../../widgets/market-ledger/marketLedgerProjection.ts';

const THEME: LogTheme = {
  surface: '#111',
  surfaceAlt: '#1a1a1a',
  hover: '#222',
  selected: '#333',
  text: '#eee',
  muted: '#888',
  line: '#444',
  lineSoft: '#333',
  accent: '#4c9aff',
  positive: '#34d399',
  negative: '#f87171',
  warning: '#f59e0b',
  hatch: '#555',
  hatchEdge: '#666',
  font: 'sans-serif',
  fontSize: { small: 11, body: 13 },
  hatchSpacing: 4,
  categories: ['#4c9aff', '#34d399', '#f59e0b', '#f87171'],
};

const MARKET_COLUMNS = [
  'id', 'occurrence', 'kind', 'mode', 'contract', 'seller', 'buyer', 'variant',
  'equipment', 'equipmentMissing', 'amountRaw', 'auxiliaryRaw', 'totalRaw', 'lostTotalRaw', 'reason',
];

const numCell = (value: string | number): LogCell => ({ kind: 'number', value: String(value) });
const txtCell = (value: string): LogCell => ({ kind: 'text', value });

function makeMarketRow(ordinal: number, hour: number, overrides: Partial<Record<string, LogCell>> = {}): LogPageRow {
  const map: Record<string, LogCell> = {
    id: txtCell(String(ordinal)),
    occurrence: numCell(ordinal),
    kind: numCell(20),
    mode: numCell(20),
    contract: txtCell('0000000000000012'),
    seller: txtCell('GER'),
    buyer: txtCell('FRA'),
    variant: txtCell('7'),
    equipment: txtCell('Infantry Equipment I'),
    equipmentMissing: { kind: 'missing' },
    amountRaw: numCell('100000'),
    auxiliaryRaw: { kind: 'missing' },
    totalRaw: numCell('100000'),
    lostTotalRaw: { kind: 'missing' },
    reason: { kind: 'missing' },
    ...overrides,
  };
  return {
    hour,
    key: `mkt:${ordinal}`,
    ordinal: String(ordinal),
    cells: MARKET_COLUMNS.map(col => map[col] ?? { kind: 'missing' }),
  };
}

function makeTailSources(reply: LogTailReplyWire, offset = 0): FamilySources {
  const panels = new Map<string, unknown>([['tail', reply]]);
  if (offset !== 0) panels.set('__offset', offset);
  return { answers: new Map(), panels, live: new Map() };
}

function countingCanvasContext() {
  const counts = { fillRect: 0, fillText: 0, measureText: 0, stroke: 0, drawImage: 0 };
  const texts: { readonly text: string; readonly x: number; readonly y: number; readonly fillStyle: string }[] = [];
  const ctx = {
    font: '',
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    globalAlpha: 1,
    textBaseline: 'middle',
    textAlign: 'left',
    save() {},
    restore() {},
    setTransform() {},
    clearRect() {},
    beginPath() {},
    rect() {},
    roundRect() {},
    arc() {},
    moveTo() {},
    lineTo() {},
    clip() {},
    fill() {},
    stroke() { counts.stroke++; },
    strokeRect() { counts.stroke++; },
    fillRect() { counts.fillRect++; },
    fillText(text: string, x: number, y: number) {
      counts.fillText++;
      texts.push({ text, x, y, fillStyle: String(ctx.fillStyle) });
    },
    drawImage() { counts.drawImage++; },
    measureText(text: string) {
      counts.measureText++;
      return { width: text.length * 7 };
    },
  };
  return { ctx: ctx as never, counts, texts };
}

test('tailLimitForViewport enforces floor 64 and scales with tall viewports using main row heights', () => {
  assert.equal(LOG_ROW_HEIGHT.comfortable, 38);
  assert.equal(LOG_ROW_HEIGHT.compact, 28);
  assert.equal(nominalRowHeight('comfortable', true), 38);
  assert.equal(nominalRowHeight('compact', true), 28);
  assert.equal(tailLimitForViewport(200, 'comfortable', true), LOG_MIN_TAIL_LIMIT);
  assert.equal(tailLimitForViewport(600, 'comfortable', true), LOG_MIN_TAIL_LIMIT);
  assert.ok(tailLimitForViewport(2400, 'compact', true) > LOG_MIN_TAIL_LIMIT);
  assert.ok(tailLimitForViewport(2400, 'compact', true) >= Math.ceil(2400 / 28) + 24);
});

test('per-frame projection, layout, and paint work is independent of total log length (1,000 vs 1,000,000 rows)', () => {
  const baseHour = HOUR_EPOCH + 1939 * 365 * 24;
  const makeFixture = (total: number) => {
    const k = 64;
    const rows: LogPageRow[] = [];
    for (let i = 0; i < k; i++) {
      rows.push(makeMarketRow(total - 1 - i, baseHour - i));
    }
    const reply: LogTailReplyWire = {
      branchId: '1',
      log: 'market-events',
      columns: MARKET_COLUMNS,
      baseOrdinal: '0',
      firstOrdinal: String(total - k),
      offset: '0',
      total: String(total),
      rows,
      facetTotals: [],
      missing: [],
      warnings: [],
    };
    const opts: MarketLedgerProjectionOptions = {
      looks: { GER: [{ from: 0, name: 'Germany' }], FRA: [{ from: 0, name: 'France' }] },
    };
    const model = marketLedgerProjection.project(makeTailSources(reply), baseHour, opts);
    const layout = buildLogLayout(model, 'comfortable', true);
    const { ctx, counts } = countingCanvasContext();
    const result = drawLog(ctx, createLogPicture(), {
      model,
      layout,
      density: 'comfortable',
      showMonths: true,
      width: 640,
      height: 360,
      scrollTop: 0,
      theme: THEME,
      inspectedKey: null,
      focusedKey: null,
      selectedKey: null,
      icons: new Map(),
      dpr: 1,
    });
    return { model, layout, result, counts };
  };

  const small = makeFixture(1_000);
  const million = makeFixture(1_000_000);

  assert.equal(small.model.rows.length, 64);
  assert.equal(million.model.rows.length, 64);
  assert.equal(million.layout.items.length, small.layout.items.length);
  assert.equal(million.result.paintedRows, small.result.paintedRows);
  assert.equal(million.result.auditRows.length, small.result.auditRows.length);
  assert.equal(million.counts.fillRect, small.counts.fillRect);
  assert.equal(million.counts.fillText, small.counts.fillText);
  assert.equal(million.counts.stroke, small.counts.stroke);
  assert.equal(million.counts.drawImage, small.counts.drawImage);
  assert.equal(small.layout.compressed, false);
  assert.equal(million.layout.compressed, true);
  assert.equal(million.layout.physicalHeight, 1_000_000);
  const midLogical = logicalScrollFromPhysical(million.layout, 360, 500_000);
  const roundTrip = physicalScrollFromLogical(million.layout, 360, midLogical);
  assert.ok(Math.abs(roundTrip - 500_000) < 1);
});

test('market-ledger projection preserves notRecorded cells, distinguishes zero from notApplicable, carries flagReason in tooltip, and builds canonical ordered audit', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '3',
    rows: [
      makeMarketRow(2, hour, {
        kind: { kind: 'notRecorded' },
        contract: { kind: 'notRecorded' },
        variant: { kind: 'notRecorded' },
        amountRaw: { kind: 'notRecorded' },
      }),
      makeMarketRow(1, hour - 1, {
        kind: numCell(20),
        amountRaw: numCell('0'),
      }),
      makeMarketRow(0, hour - 2, {
        kind: numCell(12),
        variant: txtCell('0'),
        amountRaw: { kind: 'missing' },
      }),
    ],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const opts: MarketLedgerProjectionOptions = {
    looks: {
      GER: [{ from: 0, name: 'Germany', flagReason: 'Flag atlas missing for GER' }],
      FRA: [{ from: 0, name: 'France' }],
    },
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), hour, opts);
  assert.equal(model.rows[0]!.rawMagnitudes?.kind?.kind, 'notRecorded');
  assert.equal(model.rows[0]!.rawMagnitudes?.contract?.kind, 'notRecorded');
  assert.equal(model.rows[0]!.rawMagnitudes?.amount?.kind, 'notRecorded');
  assert.ok(canonicalLogRowAudit(model.rows[0]!).includes('Equipment design not recorded'));

  const tip1 = deriveLogTooltip(model.rows[1]!);
  const flagRow = tip1?.rows.find(r => r.label === 'Seller flag');
  assert.ok(flagRow?.value?.includes('Flag atlas missing for GER'), 'F11: recorded flag failure reason carried into tooltip');

  assert.deepEqual(model.rows[1]!.rawMagnitudes?.amount, zeroValue(0n));
  assert.equal(model.rows[2]!.rawMagnitudes?.amount?.kind, 'notApplicable');
  assert.equal(model.rows[2]!.slots[1]?.kind === 'value' ? model.rows[2]!.slots[1].value.kind : null, 'notApplicable');

  const auditA = deriveLogAudit(model);
  assert.equal(auditA.ordered, true);
  assert.deepEqual([...auditA.values.keys()], ['__summary__', 'mkt:2', 'mkt:1', 'mkt:0']);
  assert.notEqual(
    canonicalLogRowAudit(model.rows[0]!),
    canonicalLogRowAudit(model.rows[1]!),
    'notRecorded and zero produce distinct canonical audit strings',
  );
  assert.equal(
    canonicalLogRowAudit(model.rows[2]!).includes('notApplicable'),
    true,
    'notApplicable is distinguished in canonical audit',
  );
});

test('F10: rowCache never reuses another recording row with identical id, ordinal, hour, and cellCount but different amount or contract', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const emptyLooks = {};
  const replyA: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1',
    rows: [makeMarketRow(42, hour, { amountRaw: numCell('100000'), contract: txtCell('00000000000000aa') })],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const replyB: LogTailReplyWire = {
    ...replyA,
    rows: [makeMarketRow(42, hour, { amountRaw: numCell('999000'), contract: txtCell('00000000000000bb') })],
  };
  const modelA = marketLedgerProjection.project(makeTailSources(replyA), hour, { looks: emptyLooks });
  const modelB = marketLedgerProjection.project(makeTailSources(replyB), hour, { looks: emptyLooks });
  const metaA = modelA.rows[0]!.meta as LedgerItem;
  const metaB = modelB.rows[0]!.meta as LedgerItem;
  assert.equal(metaA.amountRaw, '100000');
  assert.equal(metaA.contract, '00000000000000aa');
  assert.equal(metaB.amountRaw, '999000');
  assert.equal(metaB.contract, '00000000000000bb');
  assert.notEqual(canonicalLogRowAudit(modelA.rows[0]!), canonicalLogRowAudit(modelB.rows[0]!));
});

test('F17: visibleItemRange uses binary search and reads O(log N + visible) items on a 1,000-row layout', () => {
  const baseHour = HOUR_EPOCH + 1939 * 365 * 24 + 2000;
  const rows = Array.from({ length: 1_000 }, (_, i) => makeMarketRow(999 - i, baseHour - i));
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1000',
    rows,
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), baseHour, { looks: {} });
  const layout = buildLogLayout(model, 'comfortable', false);
  let reads = 0;
  const proxied = new Proxy(layout.items, {
    get(target, prop, receiver) {
      if (typeof prop === 'string' && /^\d+$/.test(prop)) reads++;
      return Reflect.get(target, prop, receiver);
    },
  });
  const [start, end] = visibleItemRange(proxied, 500 * 38, 152);
  assert.equal(start, 500);
  assert.equal(end, 504);
  assert.ok(reads <= 30, `binary search + 4 visible items should read <= 30 elements out of 1,000, read ${reads}`);
});

test('F9 & F18: narrow widths (340 px & 300 px) keep figure column, subline, and whole-line ellipsis; compact draws side-by-side at 28 px', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1',
    rows: [
      makeMarketRow(1, hour, {
        kind: numCell(21),
        mode: numCell(21),
        seller: txtCell('GER'),
        buyer: txtCell('FRA'),
        equipment: txtCell('Cavalry 1937 A Heavy Artillery Equipment'),
        amountRaw: numCell('12500'),
      }),
    ],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const opts: MarketLedgerProjectionOptions = {
    looks: { GER: [{ from: 0, name: 'Germany' }], FRA: [{ from: 0, name: 'France' }] },
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), hour, opts);

  for (const width of [340, 300]) {
    const cols = resolveLogColumns(model.columns, width, 12, 10);
    assert.ok(cols.some(c => c.id === 'figure'), `width ${width} must retain the figure column`);
    const layout = buildLogLayout(model, 'comfortable', false);
    assert.equal(layout.rowHeight, 38, 'comfortable market row height matches main (38 px)');
    const { ctx, texts } = countingCanvasContext();
    drawLog(ctx, createLogPicture(), {
      model,
      layout,
      density: 'comfortable',
      showMonths: false,
      width,
      height: 120,
      scrollTop: 0,
      theme: THEME,
      inspectedKey: null,
      focusedKey: null,
      selectedKey: null,
      icons: new Map(),
      dpr: 1,
    });
    assert.ok(texts.some(t => t.text.includes('CIC')), `width ${width} emits the figure value: ${texts.map(t => t.text).join(' | ')}`);
    const subTexts = texts.filter(t => t.y > 20 && t.x > 80);
    assert.ok(subTexts.length > 0, `width ${width} emits the counterparty/equipment subline`);
    const subJoined = subTexts.map(t => t.text).join('');
    assert.ok(subJoined.endsWith('…'), `overflowing rich subline at width ${width} must end with ellipsis, got ${JSON.stringify(subJoined)}`);
  }

  const compactLayout = buildLogLayout(model, 'compact', false);
  assert.equal(compactLayout.rowHeight, 28, 'compact market row height matches main (28 px)');
  const compactDraw = countingCanvasContext();
  drawLog(compactDraw.ctx, createLogPicture(), {
    model,
    layout: compactLayout,
    density: 'compact',
    showMonths: false,
    width: 600,
    height: 120,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
  });
  const bodyTexts = compactDraw.texts.filter(t => t.x >= 90 && !t.text.includes('CIC'));
  assert.ok(bodyTexts.length >= 2, 'compact renders both title and detail');
  assert.ok(bodyTexts.every(t => t.y === bodyTexts[0]!.y), 'compact draws title and detail on the exact same horizontal line');
  assert.ok(bodyTexts[1]!.x > bodyTexts[0]!.x, 'compact detail sits to the right of title');
});

test('F9: rich line whose penultimate part lands within 4 px of edge appends ellipsis instead of silently dropping final part', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1',
    rows: [makeMarketRow(1, hour, { seller: txtCell('GER'), buyer: txtCell('FRA'), equipment: txtCell('Rifle') })],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), hour, {
    looks: { GER: [{ from: 0, name: 'Germany' }], FRA: [{ from: 0, name: 'France' }] },
  });
  const layout = buildLogLayout(model, 'comfortable', false);
  const { ctx, texts } = countingCanvasContext();
  drawLog(ctx, createLogPicture(), {
    model,
    layout,
    density: 'comfortable',
    showMonths: false,
    width: 344,
    height: 80,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
  });
  const subTexts = texts.filter(t => t.y > 20 && t.x > 80).map(t => t.text).join('');
  assert.ok(subTexts.endsWith('…'), `truncated rich subline near edge must end with ellipsis, got ${JSON.stringify(subTexts)}`);
});

test('F5 & F7: per-index frozen page store resists late A success/failure over B and preserves full-interval coverage on interior pages', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 200;
  const fullGap = { firstHour: hour - 190, lastHour: hour - 180, reason: 'Unread early stretch' };
  const pageGap = { firstHour: hour - 50, lastHour: hour - 45, reason: 'Unread interior stretch' };
  const makePage = (offset: number, count: number, missing = [pageGap]): LogTailReplyWire => ({
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: String(1000 - offset - 1),
    offset: String(offset),
    total: '1000',
    rows: Array.from({ length: count }, (_, i) => makeMarketRow(1000 - offset - 1 - i, hour - offset - i)),
    facetTotals: offset === 0 ? [{ key: 'all', total: '1000' }] : [],
    missing,
    warnings: [],
  });
  const frozenTail = makePage(0, 64, [fullGap]);
  const pageA = makePage(128, 128, [pageGap]);
  const pageB = makePage(640, 128, [pageGap]);

  const merged = mergeFrozenPageIntoTail(frozenTail, pageB);
  assert.deepEqual(merged.missing, [fullGap], 'F7: full-interval coverage stays on the merged tail');
  assert.ok(merged.inlineMissing?.some(g => g.reason === pageGap.reason), 'F7: page-local gap stays available for inline gap rows');
  assert.deepEqual(merged.facetTotals, frozenTail.facetTotals);

  const frozen: FrozenViewState = {
    gen: 1,
    queryKey: 'q1',
    hour,
    liveSeq: 1,
    baseOrdinal: 0,
    total: 1000,
    tailLoadedCount: 64,
    pageSize: 128,
    pages: new Map(),
    inFlight: new Map(),
    errors: new Map(),
    visibleStart: 645,
    visibleEnd: 660,
    offset: 0,
    loadedPages: [],
    panels: null,
  };

  frozen.pages.set(5, pageB);
  const afterB = reconcileFrozenView(frozen);
  assert.equal(afterB.activeError, null);
  assert.equal(frozen.offset, 640);
  assert.equal((frozen.panels?.tail as LogTailReplyWire).rows[0]!.key, pageB.rows[0]!.key);

  frozen.pages.set(1, pageA);
  const afterLateASuccess = reconcileFrozenView(frozen);
  assert.equal(afterLateASuccess.activeError, null);
  assert.equal(frozen.offset, 640, 'F5 order 1: late A success must NOT overwrite visible page B');
  assert.equal((frozen.panels?.tail as LogTailReplyWire).rows[0]!.key, pageB.rows[0]!.key);

  frozen.pages.delete(1);
  frozen.errors.set(1, new Error('Page A failed'));
  const afterLateAFailure = reconcileFrozenView(frozen);
  assert.equal(afterLateAFailure.activeError, null, 'F5 order 2: late A failure must NOT cover visible page B with Retry');
  assert.equal(frozen.offset, 640);

  frozen.visibleStart = 130;
  frozen.visibleEnd = 145;
  const afterScrollBackToFailedA = reconcileFrozenView(frozen);
  assert.equal(afterScrollBackToFailedA.activeError?.pageIndex, 1, 'scrolling back to failed page A surfaces its error');
});

test('F14: verifyLogAuditSnapshot fetches expanded offset-0 page when loadedPages includes page 0', async () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 50;
  const rows100 = Array.from({ length: 100 }, (_, i) => makeMarketRow(i, hour));
  const tail64: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '100',
    rows: rows100.slice(0, 64),
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const page100: LogTailReplyWire = {
    ...tail64,
    rows: rows100,
  };
  let pageCalls = 0;
  const snap: LogAuditSnapshot<MarketLedgerProjectionOptions> = {
    queryKey: 'country:GER',
    projection: marketLedgerProjection,
    options: { looks: {} },
    density: 'comfortable',
    months: false,
    scrollTop: 70 * 38,
    height: 152,
    limit: 64,
    isStatic: true,
    exactSources: async () => makeTailSources(tail64),
    onLoadScrollPage: async (_h, offset, pageSize) => {
      pageCalls++;
      assert.equal(offset, 0);
      assert.equal(pageSize, 128);
      return page100;
    },
    frozen: {
      hour,
      baseOrdinal: 0,
      total: 100,
      pageSize: 128,
      offset: 0,
      visibleStart: 70,
      visibleEnd: 74,
      loadedPages: [{ pageIndex: 0, offset: 0, limit: 128 }],
    },
  };
  const verified = await verifyLogAuditSnapshot(hour, snap);
  assert.equal(pageCalls, 1, 'F14: offset-0 expanded page must still be fetched by the verifier');
  assert.ok(verified?.values.has('mkt:70'), 'verified audit includes row 70 from the expanded offset-0 page');
});

test('F13: mirror derives items from actual visibleItemRange past month headers and gaps, with distinct non-colliding gap indices', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 200;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '3',
    rows: [
      makeMarketRow(2, hour - 10),
      makeMarketRow(1, hour - 40),
      makeMarketRow(0, hour - 80),
    ],
    facetTotals: [],
    missing: [{ firstHour: hour - 5, lastHour: hour - 1, reason: 'Leading gap' }],
    warnings: [],
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), hour, { looks: {} });
  const layout = buildLogLayout(model, 'comfortable', false);
  const [visStart] = visibleItemRange(layout.items, 0, 200);
  const mirror = deriveLogMirrorItems(layout.items, visStart, 10, null);
  const gapEntry = mirror.find(m => m.key.startsWith('gap:'));
  const row0Entry = mirror.find(m => m.key === 'mkt:2');
  assert.ok(gapEntry && row0Entry);
  assert.notEqual(gapEntry.index, row0Entry.index, 'leading gap and first row must have distinct navigation ordinals');
  assert.equal(row0Entry.index, 0);
  assert.ok((gapEntry.index ?? 0) < 0);

  const row1Top = layout.rowTops[1]!;
  const [visAtRow1] = visibleItemRange(layout.items, row1Top, 80);
  const mirrorAtRow1 = deriveLogMirrorItems(layout.items, visAtRow1, 2, null);
  assert.equal(mirrorAtRow1[0]!.key, 'mkt:1', 'mirror starts at the exact visible row even after leading gaps');
});

test('F16: paint invalidation skips backCanvas copy on quiet frames, reacts to negative/category theme color changes, and reports honest scannedItems vs paintedItems', () => {
  const baseHour = HOUR_EPOCH + 1939 * 365 * 24 + 200;
  const opts: MarketLedgerProjectionOptions = {
    looks: { GER: [{ from: 0, name: 'Germany' }], FRA: [{ from: 0, name: 'France' }] },
  };
  const reply1: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '30',
    rows: Array.from({ length: 30 }, (_, i) => makeMarketRow(29 - i, baseHour - i, { kind: numCell(21), amountRaw: numCell('-5000') })),
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const picture = createLogPicture();
  const fakePrev = {} as CanvasImageSource;
  let prevCopies = 0;
  const preparePrevCanvas = () => { prevCopies++; return fakePrev; };

  const model1 = marketLedgerProjection.project(makeTailSources(reply1), baseHour, opts);
  const layout1 = buildLogLayout(model1, 'comfortable', false);
  const full = countingCanvasContext();
  const res1 = drawLog(full.ctx, picture, {
    model: model1,
    layout: layout1,
    density: 'comfortable',
    showMonths: false,
    width: 640,
    height: 360,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    preparePrevCanvas,
  });
  assert.equal(res1.drew, true);
  assert.equal(res1.incremental, false);
  assert.equal(prevCopies, 0, 'full repaint does not wastefully copy backCanvas');
  assert.ok(res1.paintedRows >= 9);
  assert.equal(res1.paintedItems, res1.scannedItems);

  const quiet = countingCanvasContext();
  const resQuiet = drawLog(quiet.ctx, picture, {
    model: model1,
    layout: layout1,
    density: 'comfortable',
    showMonths: false,
    width: 640,
    height: 360,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    preparePrevCanvas,
  });
  assert.equal(resQuiet.drew, false);
  assert.equal(prevCopies, 0, 'quiet frame does not copy backCanvas');
  assert.equal(resQuiet.paintedItems, 0);
  assert.ok(resQuiet.scannedItems >= 9, 'quiet frame honestly reports scannedItems');

  const rethemed = countingCanvasContext();
  const resTheme = drawLog(rethemed.ctx, picture, {
    model: model1,
    layout: layout1,
    density: 'comfortable',
    showMonths: false,
    width: 640,
    height: 360,
    scrollTop: 0,
    theme: { ...THEME, negative: '#ff0000' },
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    preparePrevCanvas,
  });
  assert.equal(resTheme.drew, true, 'changing negative theme color invalidates paused frame');

  const reply2: LogTailReplyWire = {
    ...reply1,
    total: '31',
    rows: [makeMarketRow(30, baseHour + 2, { kind: numCell(21), amountRaw: numCell('-5000') }), ...reply1.rows.slice(0, 29)],
  };
  const model2 = marketLedgerProjection.project(makeTailSources(reply2), baseHour + 2, opts);
  const layout2 = buildLogLayout(model2, 'comfortable', false);
  const inc = countingCanvasContext();
  const res2 = drawLog(inc.ctx, picture, {
    model: model2,
    layout: layout2,
    density: 'comfortable',
    showMonths: false,
    width: 640,
    height: 360,
    scrollTop: 0,
    theme: { ...THEME, negative: '#ff0000' },
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    preparePrevCanvas,
  });
  assert.equal(res2.drew, true);
  assert.equal(res2.incremental, true);
  assert.equal(prevCopies, 1, 'incremental blit copies backCanvas lazily exactly once');
  assert.equal(res2.paintedRows, 1);
  assert.equal(inc.counts.drawImage, 1);
});

test('F6: worker freezes the acknowledged liveSeq tail rather than a newer painted-but-unacknowledged hour', () => {
  if (typeof OffscreenCanvas === 'undefined') return;
  const hour0 = HOUR_EPOCH + 1940 * 365 * 24 + 100;
  const opts: MarketLedgerProjectionOptions = { looks: {} };
  const reply100: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '2',
    rows: [makeMarketRow(1, hour0), makeMarketRow(0, hour0 - 1)],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const reply101: LogTailReplyWire = {
    ...reply100,
    total: '3',
    rows: [makeMarketRow(2, hour0 + 1), makeMarketRow(1, hour0), makeMarketRow(0, hour0 - 1)],
  };
  const laneView = familyLogLaneView(() => {});
  const state = new Map<string, unknown>();
  let seq = 0;
  const paint = (hour: number, reply: LogTailReplyWire, overrides: Partial<FamilyLogBinding<MarketLedgerProjectionOptions>> = {}) =>
    laneView.paint({
      widget: 1,
      hour,
      now: 1000,
      speed: 24,
      binding: { surfaces: [{ width: 480, height: 300, dpr: 1 }] },
      model: {
        family: 'log',
        kind: 'market-ledger',
        options: opts,
        density: 'comfortable',
        months: false,
        sources: { tail: 1 },
        width: 480,
        height: 300,
        scrollTop: 0,
        theme: THEME,
        ...overrides,
      },
      subs: [{ id: 1, spec: {} as never }],
      pictures: [{ hour, through: hour, parts: [{ kind: 'panel', value: { kind: 'panel', data: reply, refused: null } }] }],
      inspect: null,
    } as never, state, () => `c${++seq}`);

  const p100 = paint(hour0, reply100);
  const meta100 = p100.fields.meta!.value as LogMetaField;
  assert.equal(meta100.total, 2);

  paint(hour0 + 1, reply101);

  const pFrozen = paint(hour0 + 1, reply101, {
    frozenGen: 1,
    frozenHour: hour0,
    frozenLiveSeq: meta100.liveSeq,
    frozenTotal: meta100.total,
    scrollTop: 40,
  });
  const metaFrozen = pFrozen.fields.meta!.value as LogMetaField;
  assert.equal(metaFrozen.hour, hour0);
  assert.equal(metaFrozen.total, 2, 'F6: frozen history holds H100 (2 rows), never leaking H101');
  assert.equal(pFrozen.fields.liveNewCount?.value, 1, 'F6: H101 counts once in the new badge without appearing in H100 history');
});

test('S5, S8, S11, S29: ledgerFacets membership, logLookAt before appearance, kind-34 drill guard, and removed rowAnimation', () => {
  const scope = {
    kind: 'membership' as const,
    membership: [
      { country: 'GER', firstHour: 100, lastHour: 250 },
      { country: 'ITA', firstHour: 180, lastHour: 300 },
    ],
    unobserved: [{ firstHour: 120, lastHour: 140 }],
  };
  const mktFacets = ledgerFacets([20, 34], scope);
  const partyFacet = mktFacets.find(f => f.name === 'country|receiver');
  assert.ok(partyFacet);
  assert.deepEqual(partyFacet.countries, []);
  assert.equal(partyFacet.membership?.length, 2);
  assert.deepEqual(partyFacet.unobserved, [{ firstHour: 120, lastHour: 140 }]);

  const looks = {
    GER: [{ from: 200, name: 'Germany' }],
    FRA: [{ from: 100, name: null, status: 'Country name unavailable' }],
  };
  assert.equal(logLookAt(looks, 'GER', 150), null);
  assert.equal(logLookAt(looks, 'GER', 200)?.name, 'Germany');

  const mktReply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1',
    rows: [makeMarketRow(0, 150, { seller: txtCell('GER'), buyer: txtCell('FRA') })],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const mktModel = marketLedgerProjection.project(makeTailSources(mktReply), 150, { looks });
  assert.deepEqual(mktModel.rows[0]!.rawMagnitudes?.sellerTag, knownValue('GER'));
  assert.deepEqual(mktModel.rows[0]!.rawMagnitudes?.buyerTag, knownValue('FRA'));

  const unknownMode: LedgerItem = {
    id: '1', kind: 34, mode: null, contract: '0000000000000012',
    seller: 'GER', sellerState: 'tag', buyer: 'ITA', buyerState: 'tag',
    variant: '6', equipment: 'Rifle', equipmentMissing: null,
    amountRaw: null, amountState: 'missing', auxiliaryRaw: null, auxiliaryState: 'missing',
    totalRaw: null, reason: 'Delivery amount unavailable',
  };
  assert.ok(!ledgerTitle(unknownMode, t => t ?? '').toLowerCase().includes('lend-lease'));
  assert.equal(ledgerSelection(unknownMode), null);
  assert.equal(ledgerSelection({ ...unknownMode, mode: 20 }), 'contract:false:0000000000000012');
  assert.equal(ledgerSelection({ ...unknownMode, mode: 3 }), 'agreement:0000000000000012');

  assert.ok(!LOG_FAMILY_OPTIONS.some(o => o.key === 'rowAnimation'));
  assert.ok(!MARKET_OPTIONS.some(o => o.key === 'rowAnimation'));
  assert.deepEqual(logFamilySettings({ density: 'compact', months: true, chips: false }), {
    density: 'compact',
    months: true,
    chips: false,
  });
});

test('fade-in runs only at <=48 h/s with motion enabled, completes cleanly with zero idle frames, and skips >48 h/s or reducedMotion', () => {
  const hour0 = HOUR_EPOCH + 1940 * 365 * 24 + 10;
  const reply0: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '5',
    rows: [4, 3, 2, 1, 0].map(i => makeMarketRow(i, hour0 - (4 - i))),
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const reply1: LogTailReplyWire = {
    ...reply0,
    total: '6',
    rows: [5, 4, 3, 2, 1, 0].map(i => makeMarketRow(i, hour0 + 1 - (5 - i))),
  };
  const opts: MarketLedgerProjectionOptions = { looks: { GER: [{ from: 0, name: 'Germany' }], FRA: [{ from: 0, name: 'France' }] } };

  const alphasSeen: number[] = [];
  const { ctx } = countingCanvasContext();
  const rawCtx = ctx as unknown as { fillText(text: string, x: number, y: number): void; globalAlpha: number };
  const baseFillText = rawCtx.fillText.bind(rawCtx);
  rawCtx.fillText = (text: string, x: number, y: number) => {
    if (rawCtx.globalAlpha < 1) alphasSeen.push(rawCtx.globalAlpha);
    baseFillText(text, x, y);
  };
  const pic = createLogPicture();
  const m0 = marketLedgerProjection.project(makeTailSources(reply0), hour0, opts);
  const l0 = buildLogLayout(m0, 'comfortable', false);
  const m1 = marketLedgerProjection.project(makeTailSources(reply1), hour0 + 1, opts);
  const l1 = buildLogLayout(m1, 'comfortable', false);

  drawLog(ctx, pic, {
    model: m0, layout: l0, density: 'comfortable', showMonths: false,
    width: 480, height: 300, scrollTop: 0, theme: THEME,
    inspectedKey: null, focusedKey: null, selectedKey: null, icons: new Map(), dpr: 1, prevCanvas: null,
  });

  const prevCanvas = {} as CanvasImageSource;
  const r1Fade = drawLog(ctx, pic, {
    model: m1, layout: l1, density: 'comfortable', showMonths: false,
    width: 480, height: 300, scrollTop: 0, theme: THEME,
    inspectedKey: null, focusedKey: null, selectedKey: null, icons: new Map(), dpr: 1, prevCanvas,
    rowAlphas: new Map([[m1.rows[0]!.key, 0.5]]),
  });
  assert.equal(r1Fade.drew, true);
  assert.equal(r1Fade.incremental, true);
  assert.equal(r1Fade.paintedItems, 1);
  assert.ok(alphasSeen.some(a => a > 0 && a < 1));

  if (typeof OffscreenCanvas !== 'undefined') {
    let fakeNow = 1000;
    const laneView = familyLogLaneView(() => {}, { clock: () => fakeNow });
    const makeBinding = (overrides: Partial<FamilyLogBinding<MarketLedgerProjectionOptions>> = {}): FamilyLogBinding<MarketLedgerProjectionOptions> => ({
      family: 'log', kind: 'market-ledger', options: opts, density: 'comfortable', months: false,
      sources: { tail: 1 }, width: 480, height: 300, scrollTop: 0, theme: THEME, ...overrides,
    });
    const runPaint = (state: Map<string, unknown>, reply: LogTailReplyWire, hour: number, speed: number, binding: FamilyLogBinding<MarketLedgerProjectionOptions>) => {
      let seq = 0;
      return laneView.paint({
        widget: 1, hour, now: fakeNow, speed,
        binding: { surfaces: [{ width: 480, height: 300, dpr: 1 }] },
        model: binding,
        subs: [{ id: 1, spec: {} as never }],
        pictures: [{ hour, through: hour, parts: [{ kind: 'panel', value: { kind: 'panel', data: reply, refused: null } }] }],
        inspect: null,
      } as never, state, () => `c${++seq}`);
    };

    const st24 = new Map<string, unknown>();
    runPaint(st24, reply0, hour0, 24, makeBinding());
    const pFade = runPaint(st24, reply1, hour0 + 1, 24, makeBinding());
    assert.equal(pFade.motion, true);
    const phasesFade = pFade.fields.phases?.value as LogPhasesField | undefined;
    assert.ok((phasesFade?.fadeActiveFrames ?? 0) >= 1);

    fakeNow += LOG_FADE_DURATION_MS + 20;
    const pDone = runPaint(st24, reply1, hour0 + 1, 24, makeBinding());
    assert.equal(pDone.motion, false);

    const stFast = new Map<string, unknown>();
    runPaint(stFast, reply0, hour0, 1440, makeBinding());
    assert.equal(runPaint(stFast, reply1, hour0 + 1, 1440, makeBinding()).motion, false);

    const stReduced = new Map<string, unknown>();
    runPaint(stReduced, reply0, hour0, 24, makeBinding({ reducedMotion: true }));
    assert.equal(runPaint(stReduced, reply1, hour0 + 1, 24, makeBinding({ reducedMotion: true })).motion, false);

    const stDisabled = new Map<string, unknown>();
    runPaint(stDisabled, reply0, hour0, 24, makeBinding({ fadeEnabled: false }));
    assert.equal(runPaint(stDisabled, reply1, hour0 + 1, 24, makeBinding({ fadeEnabled: false })).motion, false);
  }
});

test('F17: direct seller/buyer state assertions, audit excludes presentation fields, and same-hour tie ordering', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '2',
    rows: [
      makeMarketRow(1, hour, {
        kind: numCell(34),
        mode: { kind: 'notRecorded' },
        seller: { kind: 'missing' },
        buyer: { kind: 'notRecorded' },
        amountRaw: { kind: 'notRecorded' },
        variant: txtCell('0'),
        equipment: { kind: 'missing' },
      }),
      makeMarketRow(0, hour - 1, {
        kind: numCell(20),
        mode: numCell(20),
        amountRaw: numCell('0'),
        equipment: { kind: 'missing' },
        equipmentMissing: txtCell('Design missing from recording'),
      }),
    ],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const opts: MarketLedgerProjectionOptions = {
    looks: { GER: [{ from: 0, name: 'Germany' }], FRA: [{ from: 0, name: 'France' }] },
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), hour, opts);
  const row0 = model.rows[0]!;
  const row1 = model.rows[1]!;
  assert.deepEqual(row0.rawMagnitudes?.sellerTag, notRecorded('Country name unavailable'), 'F17: missing seller cell is notRecorded with unavailable reason');
  assert.deepEqual(row0.rawMagnitudes?.buyerTag, notRecorded('Country not recorded'), 'F17: notRecorded buyer cell preserves not-recorded reason');
  assert.equal(row0.rawMagnitudes?.mode?.kind, 'notRecorded');
  assert.equal(row0.rawMagnitudes?.amount?.kind, 'notRecorded');
  assert.equal(row0.rawMagnitudes?.auxiliary?.kind, 'notApplicable');
  assert.ok(canonicalLogRowAudit(row1).includes('Design missing from recording'));
  assert.equal(row0.meta && ledgerSelection(row0.meta as never), null);
  assert.notEqual(row1.meta && ledgerSelection(row1.meta as never), null);

  const layout = buildLogLayout(model, 'comfortable', false);
  const auditWithBadge = deriveLogAuditRows(model, layout.items, 0, layout.items.length, 0, 0, 300, 7);
  const auditNoBadge = deriveLogAuditRows(model, layout.items, 0, layout.items.length, 0, 0, 300, 0);
  assert.notDeepEqual(auditWithBadge, auditNoBadge);

  const restyledRow = {
    ...row1,
    art: row1.art ? { ...row1.art, color: '#ff00ff' } : undefined,
    slots: row1.slots.map(slot => (
      slot.kind === 'text' ? { ...slot, tone: 'accent' as const, weight: 'bold' as const, maxLines: 2 } : slot
    )),
  };
  assert.equal(canonicalLogRowAudit(restyledRow), canonicalLogRowAudit(row1));

  const sameHourRows = [
    makeMarketRow(12, hour),
    makeMarketRow(11, hour),
    makeMarketRow(10, hour - 1),
  ];
  assert.deepEqual(newestFirstWireRows(sameHourRows).map(r => r.ordinal), ['11', '12', '10']);

  const range = scrollPageOrdinalRange(50, 200, 64, 64);
  assert.deepEqual(range, { cursor: String(50 + 72), pageSize: 64 });
  const reversed = pageReplyToTail(
    { branchId: '1', log: 'market-events', columns: MARKET_COLUMNS, firstOrdinal: '122', total: '200', cursor: '186', rows: [makeMarketRow(122, 100), makeMarketRow(123, 101)], missing: [], warnings: [] },
    50,
    200,
  );
  assert.deepEqual(reversed.rows.map(r => r.key), ['mkt:123', 'mkt:122']);
  assert.ok(assembleFrozenPageWindow(reply, new Map(), 128, 0, 2)?.tail.rows.length === 2);
});
