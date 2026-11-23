import assert from 'node:assert/strict';
import test from 'node:test';
import type { LogCell, LogPageRow } from '../../generated/dashboard';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import type { FamilySources } from '../contract.ts';
import { knownValue, notApplicable, notRecorded, zeroValue } from '../value.ts';
import {
  buildLogLayout,
  LOG_MIN_TAIL_LIMIT,
  logicalScrollFromPhysical,
  physicalScrollFromLogical,
  tailLimitForViewport,
  visibleItemRange,
} from './layout.ts';
import { canonicalLogRowAudit, deriveLogAudit, deriveLogTooltip } from './model.ts';
import { createLogPicture, drawLog, type LogTheme } from './painter.ts';
import { pageReplyToTail, scrollPageOrdinalRange, type LogTailReplyWire } from './tailReply.ts';
import { divisionLogProjection, type DivisionLogProjectionOptions } from '../../widgets/division-log/divisionLogProjection.ts';
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

const DIV_COLUMNS = [
  'division', 'kind', 'status', 'hour', 'by', 'afterHour', 'tag', 'name', 'nameMissing',
  'icon', 'iconMissing', 'presentationHour', 'template', 'templateName', 'templateMissing',
  'province', 'placeHour', 'placeAtEvent', 'killerTag', 'killerDivision', 'matchedDeaths',
  'readHour', 'placeName', 'placeMissing',
];

const MARKET_COLUMNS = [
  'id', 'occurrence', 'kind', 'mode', 'contract', 'seller', 'buyer', 'variant',
  'equipment', 'equipmentMissing', 'amountRaw', 'auxiliaryRaw', 'totalRaw', 'lostTotalRaw', 'reason',
];

const numCell = (value: string | number): LogCell => ({ kind: 'number', value: String(value) });
const txtCell = (value: string): LogCell => ({ kind: 'text', value });

function makeDivisionRow(ordinal: number, hour: number, overrides: Partial<Record<string, LogCell>> = {}): LogPageRow {
  const map: Record<string, LogCell> = {
    division: txtCell(String(ordinal)),
    kind: numCell(0),
    status: txtCell('first_observed'),
    hour: numCell(hour),
    by: numCell(0),
    afterHour: { kind: 'missing' },
    tag: txtCell('GER'),
    name: txtCell(`Inf. Div. ${ordinal}`),
    nameMissing: { kind: 'missing' },
    icon: { kind: 'missing' },
    iconMissing: { kind: 'missing' },
    presentationHour: numCell(hour),
    template: txtCell('31'),
    templateName: txtCell('Infanterie-Division'),
    templateMissing: { kind: 'missing' },
    province: numCell(42),
    placeHour: numCell(hour),
    placeAtEvent: numCell(1),
    killerTag: { kind: 'missing' },
    killerDivision: { kind: 'missing' },
    matchedDeaths: { kind: 'missing' },
    readHour: numCell(hour),
    placeName: txtCell('Berlin'),
    placeMissing: { kind: 'missing' },
    ...overrides,
  };
  return {
    hour,
    key: `div:${ordinal}`,
    ordinal: String(ordinal),
    cells: DIV_COLUMNS.map(col => map[col] ?? { kind: 'missing' }),
  };
}

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
  const counts = { fillRect: 0, fillText: 0, measureText: 0, stroke: 0 };
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
    fillText() { counts.fillText++; },
    drawImage() {},
    measureText(text: string) {
      counts.measureText++;
      return { width: text.length * 7 };
    },
  };
  return { ctx: ctx as never, counts };
}

test('tailLimitForViewport enforces floor 64 and scales with tall viewports', () => {
  assert.equal(tailLimitForViewport(200, 'comfortable', false), LOG_MIN_TAIL_LIMIT);
  assert.equal(tailLimitForViewport(600, 'comfortable', false), LOG_MIN_TAIL_LIMIT);
  assert.ok(tailLimitForViewport(2400, 'compact', false) > LOG_MIN_TAIL_LIMIT);
  assert.ok(tailLimitForViewport(2400, 'compact', false) >= Math.ceil(2400 / 22) + 24);
});

test('per-frame projection, layout, and paint work is independent of total log length (1,000 vs 1,000,000 rows)', () => {
  const baseHour = HOUR_EPOCH + 1939 * 365 * 24;
  const makeFixture = (total: number) => {
    const k = 64;
    const rows: LogPageRow[] = [];
    for (let i = 0; i < k; i++) {
      rows.push(makeDivisionRow(total - 1 - i, baseHour - i));
    }
    const reply: LogTailReplyWire = {
      branchId: '1',
      log: 'division-events',
      columns: DIV_COLUMNS,
      baseOrdinal: '0',
      firstOrdinal: String(total - k),
      total: String(total),
      rows,
      facetTotals: [
        { key: 'created', total: String(total) },
        { key: 'vanished', total: '0' },
      ],
      missing: [],
      warnings: [],
    };
    const opts: DivisionLogProjectionOptions = {
      events: 'both',
      showTotals: true,
      lossesAvailable: true,
      looks: { GER: [{ from: 0, name: 'Germany' }] },
      manifestWarnings: [],
      manifestGaps: [],
    };
    const model = divisionLogProjection.project(makeTailSources(reply), baseHour, opts);
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
  assert.deepEqual(million.counts, small.counts);
  assert.equal(small.layout.compressed, false);
  assert.equal(million.layout.compressed, true);
  assert.equal(million.layout.physicalHeight, 1_000_000);
  const midLogical = logicalScrollFromPhysical(million.layout, 360, 500_000);
  const roundTrip = physicalScrollFromLogical(million.layout, 360, midLogical);
  assert.ok(Math.abs(roundTrip - 500_000) < 1);
});

test('division-log projection keeps missing kind as notRecorded, uses inline place names, and reports exact vanished count including partial bin', () => {
  const hour = HOUR_EPOCH + 1940 * 365 * 24 + 3;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '2',
    rows: [
      makeDivisionRow(1, hour, {
        kind: { kind: 'notRecorded' },
        status: { kind: 'notRecorded' },
        placeName: { kind: 'missing' },
        placeMissing: txtCell('Province names unavailable'),
      }),
      makeDivisionRow(0, hour - 1, {
        kind: numCell(1),
        status: txtCell('division_destroyed'),
        killerTag: txtCell('SOV'),
        placeName: txtCell('Stalingrad'),
      }),
    ],
    facetTotals: [
      { key: 'created', total: '0' },
      { key: 'lost', total: '1' },
      { key: 'transfer', total: '0' },
      { key: 'vanished', total: '3' },
    ],
    missing: [{ firstHour: hour - 2, lastHour: hour - 2, reason: 'Log gap' }],
    warnings: [],
  };
  const opts: DivisionLogProjectionOptions = {
    events: 'both',
    showTotals: true,
    lossesAvailable: true,
    looks: {
      GER: [{ from: 0, name: 'Germany' }],
      SOV: [{ from: 0, name: 'Soviet Union' }],
    },
    manifestWarnings: [],
    manifestGaps: [],
  };
  const model = divisionLogProjection.project(makeTailSources(reply), hour, opts);
  assert.equal(model.rows.length, 2);
  assert.equal(model.rows[0]!.auditCells.kind.kind, 'notRecorded');
  assert.equal(model.rows[0]!.auditCells.status.kind, 'notRecorded');
  assert.equal(model.rows[0]!.auditCells.place.kind, 'notRecorded');
  assert.deepEqual(model.rows[1]!.auditCells.place, knownValue('Stalingrad'));
  assert.deepEqual(model.rows[1]!.auditCells.killer, knownValue('Soviet Union'));
  assert.ok(model.warnings?.some(w => w.includes('(3)')), 'exact vanished count from facetTotals surfaces in warnings');

  const layout = buildLogLayout(model, 'comfortable', false);
  assert.ok(layout.items.some(item => item.kind === 'gap' && item.label.startsWith('Not recorded · ')), 'gap row rendered inline');

  const tip = deriveLogTooltip(model.rows[1]!);
  assert.ok(tip?.subtitle && tip.subtitle.includes('1940'), `tooltip carries date line: ${tip?.subtitle}`);
});

test('market-ledger projection preserves notRecorded cells, distinguishes zero from notApplicable, and builds canonical ordered audit', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
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
      GER: [{ from: 0, name: 'Germany' }],
      FRA: [{ from: 0, name: 'France' }],
    },
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), hour, opts);
  assert.equal(model.rows[0]!.auditCells.kind.kind, 'notRecorded');
  assert.equal(model.rows[0]!.auditCells.contract.kind, 'notRecorded');
  assert.equal(model.rows[0]!.auditCells.equipment.kind, 'notRecorded');
  assert.equal(model.rows[0]!.auditCells.amount.kind, 'notRecorded');

  assert.deepEqual(model.rows[1]!.auditCells.amount, zeroValue(0n));
  assert.equal(model.rows[2]!.auditCells.amount.kind, 'notApplicable');
  assert.equal(model.rows[2]!.auditCells.equipment.kind, 'notApplicable');

  const auditA = deriveLogAudit(model);
  assert.equal(auditA.ordered, true);
  assert.deepEqual([...auditA.values.keys()], ['mkt:2', 'mkt:1', 'mkt:0']);
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

test('scroll-back ordinal range and pageReplyToTail map newest-first scroll offsets to exact ascending cursors', () => {
  const range = scrollPageOrdinalRange(50, 200, 64, 64);
  assert.deepEqual(range, { cursor: String(50 + 72), pageSize: 64 });

  const tailEnd = scrollPageOrdinalRange(50, 200, 180, 64);
  assert.deepEqual(tailEnd, { cursor: '50', pageSize: 20 });

  assert.equal(scrollPageOrdinalRange(50, 200, 200, 64), null);

  const reversed = pageReplyToTail(
    {
      branchId: '1',
      log: 'division-events',
      columns: DIV_COLUMNS,
      firstOrdinal: '122',
      total: '200',
      cursor: '186',
      rows: [makeDivisionRow(122, 100), makeDivisionRow(123, 101)],
      missing: [],
      warnings: [],
    },
    50,
    200,
  );
  assert.deepEqual(reversed.rows.map(r => r.key), ['div:123', 'div:122']);
  assert.equal(reversed.baseOrdinal, '50');
  assert.equal(reversed.total, '200');
});

test('visibleItemRange returns only the viewport slice in O(log N + visible)', () => {
  const baseHour = HOUR_EPOCH + 1939 * 365 * 24;
  const rows = Array.from({ length: 64 }, (_, i) => makeDivisionRow(63 - i, baseHour - i));
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '64',
    rows,
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const model = divisionLogProjection.project(makeTailSources(reply), baseHour, {
    events: 'both',
    showTotals: false,
    lossesAvailable: true,
    looks: {},
    manifestWarnings: [],
    manifestGaps: [],
  });
  const layout = buildLogLayout(model, 'comfortable', false);
  const [start, end] = visibleItemRange(layout.items, 0, 140);
  assert.equal(start, 0);
  assert.equal(end, 5);
  assert.ok(notRecorded('missing').kind === 'notRecorded' && notApplicable('none').kind === 'notApplicable');
});
