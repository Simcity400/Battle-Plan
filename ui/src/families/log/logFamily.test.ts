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
  logicalYForRowIndex,
  physicalScrollFromLogical,
  rowIndexAtLogicalY,
  scrollGeometryOf,
  tailLimitForViewport,
  visibleItemRange,
} from './layout.ts';
import { logLookAt } from './declaration.ts';
import { LOG_FAMILY_OPTIONS, logFamilySettings } from './options.ts';
import { canonicalLogRowAudit, deriveLogAudit, deriveLogAuditRows, deriveLogTooltip } from './model.ts';
import { createLogPicture, drawLog, type LogTheme } from './painter.ts';
import { familyLogLaneView, LOG_FADE_DURATION_MS, type FamilyLogBinding, type LogPhasesField } from './laneView.ts';
import './registerAll.ts';
import { mergeFrozenPageIntoTail, pageReplyToTail, scrollPageOrdinalRange, type LogTailReplyWire } from './tailReply.ts';
import { ledgerFacets, ledgerSelection, ledgerTitle, type LedgerItem } from '../../dashboard/marketLedger.ts';
import { divisionFacets } from '../../widgets/division-log/divisionLogData.ts';
import { OPTIONS as DIVISION_OPTIONS } from '../../widgets/division-log/definition.ts';
import { OPTIONS as MARKET_OPTIONS } from '../../widgets/market-ledger/definition.ts';
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
  const counts = { fillRect: 0, fillText: 0, measureText: 0, stroke: 0, drawImage: 0 };
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
    drawImage() { counts.drawImage++; },
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
  const row0 = model.rows[0]!;
  const row1 = model.rows[1]!;
  assert.equal(row0.rawMagnitudes?.kind?.kind, 'notRecorded');
  assert.equal(row0.slots[1]?.kind === 'pill' ? row0.slots[1].label.kind : null, 'notRecorded');
  assert.ok(canonicalLogRowAudit(row0).includes('Province names unavailable'));
  assert.ok(canonicalLogRowAudit(row1).includes('Stalingrad'));
  assert.ok(canonicalLogRowAudit(row1).includes('Soviet Union'));
  assert.ok(model.warnings?.some(w => w.includes('(3)')), 'exact vanished count from facetTotals surfaces in warnings');

  const layout = buildLogLayout(model, 'comfortable', false);
  assert.ok(layout.items.some(item => item.kind === 'gap' && item.label.startsWith('Not recorded · ')), 'gap row rendered inline');

  const tip = deriveLogTooltip(row1);
  assert.equal(tip?.title, 'Inf. Div. 0');
  assert.equal(tip?.subtitle, undefined, 'tooltip omits static date line so display controller formats date outside audited model');
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
  assert.equal(model.rows[0]!.rawMagnitudes?.kind?.kind, 'notRecorded');
  assert.equal(model.rows[0]!.rawMagnitudes?.contract?.kind, 'notRecorded');
  assert.equal(model.rows[0]!.rawMagnitudes?.amount?.kind, 'notRecorded');
  assert.ok(canonicalLogRowAudit(model.rows[0]!).includes('Equipment design not recorded'));

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

test('entering one new row at the top shifts via drawImage and paints only the new row; quiet hours skip repaint', () => {
  const baseHour = HOUR_EPOCH + 1939 * 365 * 24 + 200;
  const opts: DivisionLogProjectionOptions = {
    events: 'both',
    showTotals: false,
    lossesAvailable: true,
    looks: { GER: [{ from: 0, name: 'Germany' }] },
    manifestWarnings: [],
    manifestGaps: [],
  };
  const reply1: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '30',
    rows: Array.from({ length: 30 }, (_, i) => makeDivisionRow(29 - i, baseHour - i)),
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const picture = createLogPicture();
  const fakePrev = {} as CanvasImageSource;

  const model1 = divisionLogProjection.project(makeTailSources(reply1), baseHour, opts);
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
    prevCanvas: fakePrev,
  });
  assert.equal(res1.drew, true);
  assert.equal(res1.incremental, false);
  assert.ok(res1.paintedRows >= 12);

  const quiet = countingCanvasContext();
  const modelQuiet = divisionLogProjection.project(makeTailSources(reply1), baseHour + 1, opts);
  const layoutQuiet = buildLogLayout(modelQuiet, 'comfortable', false);
  const resQuiet = drawLog(quiet.ctx, picture, {
    model: modelQuiet,
    layout: layoutQuiet,
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
    prevCanvas: fakePrev,
  });
  assert.equal(resQuiet.drew, false);
  assert.equal(quiet.counts.fillText, 0);
  assert.equal(quiet.counts.drawImage, 0);

  const reply2: LogTailReplyWire = {
    ...reply1,
    total: '31',
    rows: [makeDivisionRow(30, baseHour + 2), ...reply1.rows.slice(0, 29)],
  };
  const model2 = divisionLogProjection.project(makeTailSources(reply2), baseHour + 2, opts);
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
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    prevCanvas: fakePrev,
  });
  assert.equal(res2.drew, true);
  assert.equal(res2.incremental, true);
  assert.equal(res2.paintedRows, 1);
  assert.equal(inc.counts.drawImage, 1);
  assert.ok(inc.counts.fillText * 5 < full.counts.fillText, `incremental fillText=${inc.counts.fillText} vs full=${full.counts.fillText}`);

  const monthPic = createLogPicture();
  const m1 = buildLogLayout(model1, 'comfortable', true);
  const fullMonth = countingCanvasContext();
  drawLog(fullMonth.ctx, monthPic, {
    model: model1,
    layout: m1,
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
    prevCanvas: fakePrev,
  });
  const m2 = buildLogLayout(model2, 'comfortable', true);
  const incMonth = countingCanvasContext();
  const resMonth = drawLog(incMonth.ctx, monthPic, {
    model: model2,
    layout: m2,
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
    prevCanvas: fakePrev,
  });
  assert.equal(resMonth.incremental, true, 'same-month entering row shifts via drawImage under unchanged month header');
  assert.equal(resMonth.paintedRows, 1);
  assert.equal(incMonth.counts.drawImage, 1);
});

test('S5: divisionFacets and ledgerFacets preserve dated membership runs and unobserved spans', () => {
  const scope = {
    kind: 'membership' as const,
    membership: [
      { tag: 'GER', firstHour: 100, lastHour: 250 },
      { tag: 'ITA', firstHour: 180, lastHour: 300 },
    ],
    unobserved: [{ firstHour: 120, lastHour: 140 }],
  };
  const divFacets = divisionFacets(['created', 'lost'], null, scope);
  const slotFacet = divFacets.find(f => f.name === 'slot');
  assert.ok(slotFacet);
  assert.deepEqual(slotFacet.countries, []);
  assert.equal(slotFacet.membership?.length, 2);
  assert.equal(slotFacet.membership?.[0]?.firstHour, 100);
  assert.deepEqual(slotFacet.unobserved, [{ firstHour: 120, lastHour: 140 }]);

  const mktFacets = ledgerFacets([20, 34], scope);
  const partyFacet = mktFacets.find(f => f.name === 'country|receiver');
  assert.ok(partyFacet);
  assert.deepEqual(partyFacet.countries, []);
  assert.equal(partyFacet.membership?.length, 2);
  assert.deepEqual(partyFacet.unobserved, [{ firstHour: 120, lastHour: 140 }]);
});

test('S8: logLookAt returns null before first dated appearance and unresolved country names stay notRecorded with tag kept separately', () => {
  const looks = {
    GER: [{ from: 200, name: 'Germany' }],
    FRA: [{ from: 100, name: null, status: 'Country name unavailable' }],
  };
  assert.equal(logLookAt(looks, 'GER', 150), null, 'before first appearance returns null');
  assert.equal(logLookAt(looks, 'GER', 200)?.name, 'Germany');

  const divReply: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '1',
    rows: [makeDivisionRow(0, 150, { tag: txtCell('GER') })],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const divModel = divisionLogProjection.project(makeTailSources(divReply), 150, {
    events: 'both',
    showTotals: false,
    lossesAvailable: true,
    looks,
    manifestWarnings: [],
    manifestGaps: [],
  });
  const divBody = divModel.rows[0]!.slots[0];
  assert.equal(divBody?.kind === 'rich' ? divBody.parts[0]?.text.kind : null, 'notRecorded');
  assert.deepEqual(divModel.rows[0]!.rawMagnitudes?.countryTag, knownValue('GER'));

  const mktReply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '1',
    rows: [makeMarketRow(0, 150, { seller: txtCell('GER'), buyer: txtCell('FRA') })],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const mktModel = marketLedgerProjection.project(makeTailSources(mktReply), 150, { looks });
  const mktSub = mktModel.rows[0]!.subline;
  assert.equal(mktSub?.kind === 'rich' ? mktSub.parts[0]?.text.kind : null, 'notRecorded');
  assert.deepEqual(mktModel.rows[0]!.rawMagnitudes?.sellerTag, knownValue('GER'));
  assert.equal(mktSub?.kind === 'rich' ? mktSub.parts[2]?.text.kind : null, 'notRecorded');
  assert.deepEqual(mktModel.rows[0]!.rawMagnitudes?.buyerTag, knownValue('FRA'));
});

test('S9: fallback state place name with missing reason is partial across row, tooltip, mirror, and audit', () => {
  const hour = HOUR_EPOCH + 1940 * 365 * 24;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '1',
    rows: [makeDivisionRow(0, hour, {
      placeName: txtCell('Brandenburg'),
      placeMissing: txtCell('Province rename table not recorded'),
    })],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const model = divisionLogProjection.project(makeTailSources(reply), hour, {
    events: 'both',
    showTotals: false,
    lossesAvailable: true,
    looks: { GER: [{ from: 0, name: 'Germany' }] },
    manifestWarnings: [],
    manifestGaps: [],
  });
  const row = model.rows[0]!;
  const placePart = row.slots[0]?.kind === 'rich' ? row.slots[0].parts[6]?.text : null;
  assert.equal(placePart?.kind, 'partial');
  if (placePart?.kind === 'partial') {
    assert.equal(placePart.value, 'Brandenburg');
    assert.equal(placePart.reason, 'Province rename table not recorded');
  }
  assert.ok(canonicalLogRowAudit(row).includes('Province rename table not recorded'));
  assert.ok(row.spoken.includes('Brandenburg') && row.spoken.includes('Province rename table not recorded'));
  const tip = deriveLogTooltip(row);
  const locRow = tip?.rows.find(r => r.label === 'Location');
  assert.ok(locRow?.value?.includes('Brandenburg') && locRow.value.includes('Province rename table not recorded'));
});

test('S10: division-log facet pills derive notRecorded or partial from interval coverage', () => {
  const fromHour = 100;
  const hour = 200;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '0',
    rows: [],
    facetTotals: [
      { key: 'created', total: '5' },
      { key: 'lost', total: '0' },
      { key: 'transfer', total: '0' },
    ],
    missing: [],
    warnings: [],
  };
  const noRemovals = divisionLogProjection.project(makeTailSources(reply), hour, {
    events: 'both',
    showTotals: true,
    lossesAvailable: true,
    fromHour,
    looks: {},
    manifestWarnings: [],
    manifestGaps: [{ firstHour: 100, lastHour: 200, reason: 'division equipment deletions unavailable; removal causes and totals incomplete' }],
  });
  assert.equal(noRemovals.facetPills?.find(p => p.key === 'created')?.count.kind, 'known');
  assert.equal(noRemovals.facetPills?.find(p => p.key === 'lost')?.count.kind, 'notRecorded');
  assert.equal(noRemovals.facetPills?.find(p => p.key === 'transfer')?.count.kind, 'notRecorded');

  const mixedRemovals = divisionLogProjection.project(makeTailSources(reply), hour, {
    events: 'both',
    showTotals: true,
    lossesAvailable: true,
    fromHour,
    looks: {},
    manifestWarnings: [],
    manifestGaps: [{ firstHour: 100, lastHour: 150, reason: 'division equipment deletions unavailable; removal causes and totals incomplete' }],
  });
  const lostMixed = mixedRemovals.facetPills?.find(p => p.key === 'lost')?.count;
  assert.equal(lostMixed?.kind, 'partial');
  if (lostMixed?.kind === 'partial') {
    assert.equal(lostMixed.value, 0);
    assert.equal(lostMixed.atLeast, true);
  }
});

test('S11: kind-34 delivery with unknown mode stays unknown and cannot open a drill target', () => {
  const unknownMode: LedgerItem = {
    id: '1',
    kind: 34,
    mode: null,
    contract: '0000000000000012',
    seller: 'GER',
    sellerState: 'tag',
    buyer: 'ITA',
    buyerState: 'tag',
    variant: '6',
    equipment: 'Rifle',
    equipmentMissing: null,
    amountRaw: null,
    amountState: 'missing',
    auxiliaryRaw: null,
    auxiliaryState: 'missing',
    totalRaw: null,
    reason: 'Delivery amount unavailable',
  };
  assert.ok(!ledgerTitle(unknownMode, t => t ?? '').toLowerCase().includes('lend-lease'));
  assert.equal(ledgerSelection(unknownMode), null);
  assert.equal(ledgerSelection({ ...unknownMode, mode: 20 }), 'contract:false:0000000000000012');
  assert.equal(ledgerSelection({ ...unknownMode, mode: 3 }), 'agreement:0000000000000012');
  assert.equal(ledgerSelection({ ...unknownMode, mode: 20, contract: null }), null);
});

test('S18: mergeFrozenPageIntoTail preserves frozen facetTotals and warnings when paging older slices', () => {
  const frozen: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '100',
    total: '164',
    rows: [],
    facetTotals: [{ key: 'created', total: '120' }, { key: 'vanished', total: '4' }],
    missing: [],
    warnings: ['Frozen warning'],
  };
  const page: LogTailReplyWire = {
    ...frozen,
    firstOrdinal: '0',
    facetTotals: [],
    warnings: ['Page warning'],
  };
  const merged = mergeFrozenPageIntoTail(frozen, page);
  assert.deepEqual(merged.facetTotals, frozen.facetTotals);
  assert.deepEqual(merged.warnings, ['Frozen warning', 'Page warning']);
});

test('S19 & S20: layout clips gaps to both bounds on every page, aligns row/scroll positions across headers and gaps, and nextChange advertises manifest gap boundaries', () => {
  const hour = HOUR_EPOCH + 1940 * 365 * 24 + 200;
  const fromHour = hour - 100;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '200',
    rows: [
      makeDivisionRow(135, hour - 20),
      makeDivisionRow(134, hour - 80),
    ],
    facetTotals: [],
    missing: [
      { firstHour: fromHour - 50, lastHour: fromHour - 10, reason: 'Before range' },
      { firstHour: hour - 50, lastHour: hour - 40, reason: 'Inside page 2' },
    ],
    warnings: [],
  };
  const sources = { answers: new Map(), panels: new Map<string, unknown>([['tail', reply], ['__offset', 64]]), live: new Map() };
  const opts: DivisionLogProjectionOptions = {
    events: 'both',
    showTotals: false,
    lossesAvailable: true,
    fromHour,
    looks: { GER: [{ from: 0, name: 'Germany' }] },
    manifestWarnings: [],
    manifestGaps: [{ firstHour: hour + 10, lastHour: hour + 20, reason: 'Upcoming gap' }],
  };
  const model = divisionLogProjection.project(sources, hour, opts);
  const layout = buildLogLayout(model, 'comfortable', true);
  assert.ok(layout.items.some(i => i.kind === 'gap' && i.span.reason === 'Inside page 2'), 'non-zero page offset renders its gaps');
  assert.ok(!layout.items.some(i => i.kind === 'gap' && i.span.reason === 'Before range'), 'gaps before fromHour are clipped out');

  const geom = scrollGeometryOf(layout);
  const row0Y = logicalYForRowIndex(geom, 64);
  const row1Y = logicalYForRowIndex(geom, 65);
  assert.ok(row1Y - row0Y > geom.rowHeight, 'month header and gap sit between row 64 and row 65');
  assert.equal(rowIndexAtLogicalY(geom, row0Y), 64);
  assert.equal(rowIndexAtLogicalY(geom, row1Y), 65);

  assert.equal(divisionLogProjection.nextChange?.(sources, hour, opts), hour + 10);
  assert.equal(divisionLogProjection.nextChange?.(sources, hour + 10, opts), hour + 11);
  assert.equal(divisionLogProjection.nextChange?.(sources, hour + 20, opts), hour + 21);
  assert.equal(divisionLogProjection.nextChange?.(sources, hour + 21, opts), Infinity);
});

test('S29: log family options omit the removed rowAnimation setting', () => {
  assert.ok(!LOG_FAMILY_OPTIONS.some(o => o.key === 'rowAnimation'));
  assert.ok(!DIVISION_OPTIONS.some(o => o.key === 'rowAnimation'));
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
    log: 'division-events',
    columns: DIV_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    total: '5',
    rows: [4, 3, 2, 1, 0].map(i => makeDivisionRow(i, hour0 - (4 - i))),
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const reply1: LogTailReplyWire = {
    ...reply0,
    total: '6',
    rows: [5, 4, 3, 2, 1, 0].map(i => makeDivisionRow(i, hour0 + 1 - (5 - i))),
  };
  const opts: DivisionLogProjectionOptions = {
    events: 'both',
    showTotals: false,
    lossesAvailable: true,
    looks: { GER: [{ from: 0, name: 'Germany' }] },
    manifestWarnings: [],
    manifestGaps: [],
  };

  const alphasSeen: number[] = [];
  const { ctx } = countingCanvasContext();
  const rawCtx = ctx as unknown as { fillText(text: string, x: number, y: number): void; globalAlpha: number };
  const baseFillText = rawCtx.fillText.bind(rawCtx);
  rawCtx.fillText = (text: string, x: number, y: number) => {
    if (rawCtx.globalAlpha < 1) alphasSeen.push(rawCtx.globalAlpha);
    baseFillText(text, x, y);
  };
  const pic = createLogPicture();
  const m0 = divisionLogProjection.project(makeTailSources(reply0), hour0, opts);
  const l0 = buildLogLayout(m0, 'comfortable', false);
  const m1 = divisionLogProjection.project(makeTailSources(reply1), hour0 + 1, opts);
  const l1 = buildLogLayout(m1, 'comfortable', false);

  const r0 = drawLog(ctx, pic, {
    model: m0,
    layout: l0,
    density: 'comfortable',
    showMonths: false,
    width: 480,
    height: 300,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    prevCanvas: null,
  });
  assert.equal(r0.drew, true);
  assert.equal(r0.visitedItems, 5);

  const prevCanvas = {} as CanvasImageSource;
  const r1Fade = drawLog(ctx, pic, {
    model: m1,
    layout: l1,
    density: 'comfortable',
    showMonths: false,
    width: 480,
    height: 300,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    prevCanvas,
    rowAlphas: new Map([[m1.rows[0]!.key, 0.5]]),
  });
  assert.equal(r1Fade.drew, true);
  assert.equal(r1Fade.incremental, true, 'tail advance with fading top row still uses incremental blit');
  assert.equal(r1Fade.visitedItems, 1, 'tail advance visits only the 1 new row, not all visible rows');
  assert.ok(alphasSeen.some(a => a > 0 && a < 1), 'fading row draws with globalAlpha < 1');

  const r1Done = drawLog(ctx, pic, {
    model: m1,
    layout: l1,
    density: 'comfortable',
    showMonths: false,
    width: 480,
    height: 300,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    prevCanvas,
  });
  assert.equal(r1Done.drew, true);
  const r1Idle = drawLog(ctx, pic, {
    model: m1,
    layout: l1,
    density: 'comfortable',
    showMonths: false,
    width: 480,
    height: 300,
    scrollTop: 0,
    theme: THEME,
    inspectedKey: null,
    focusedKey: null,
    selectedKey: null,
    icons: new Map(),
    dpr: 1,
    prevCanvas,
  });
  assert.equal(r1Idle.drew, false, 'after fade completes, identical frame skips repaint');
  assert.equal(r1Idle.visitedItems, 0);

  if (typeof OffscreenCanvas !== 'undefined') {
    let fakeNow = 1000;
    const laneView = familyLogLaneView(() => {}, { clock: () => fakeNow });
    const makeBinding = (overrides: Partial<FamilyLogBinding<DivisionLogProjectionOptions>> = {}): FamilyLogBinding<DivisionLogProjectionOptions> => ({
      family: 'log',
      kind: 'division-log',
      options: opts,
      density: 'comfortable',
      months: false,
      sources: { tail: 1 },
      width: 480,
      height: 300,
      scrollTop: 0,
      theme: THEME,
      ...overrides,
    });
    const runPaint = (
      state: Map<string, unknown>,
      reply: LogTailReplyWire,
      hour: number,
      speed: number,
      binding: FamilyLogBinding<DivisionLogProjectionOptions>,
    ) => {
      let seq = 0;
      return laneView.paint({
        widget: 1,
        hour,
        now: fakeNow,
        speed,
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
    assert.equal(pFade.motion, true, '24 h/s new row triggers single-pass fade motion');
    const phasesFade = pFade.fields.phases?.value as LogPhasesField | undefined;
    assert.ok((phasesFade?.fadeActiveFrames ?? 0) >= 1, 'fadeActiveFrames increments during active fade');

    fakeNow += LOG_FADE_DURATION_MS + 20;
    const pDone = runPaint(st24, reply1, hour0 + 1, 24, makeBinding());
    assert.equal(pDone.motion, false, 'motion stops cleanly once fade completes (zero idle frames)');

    const stFast = new Map<string, unknown>();
    runPaint(stFast, reply0, hour0, 1440, makeBinding());
    const pFast = runPaint(stFast, reply1, hour0 + 1, 1440, makeBinding());
    assert.equal(pFast.motion, false, '>48 h/s skips fade-in');

    const stReduced = new Map<string, unknown>();
    runPaint(stReduced, reply0, hour0, 24, makeBinding({ reducedMotion: true }));
    const pReduced = runPaint(stReduced, reply1, hour0 + 1, 24, makeBinding({ reducedMotion: true }));
    assert.equal(pReduced.motion, false, 'reducedMotion skips fade-in');

    const stDisabled = new Map<string, unknown>();
    runPaint(stDisabled, reply0, hour0, 24, makeBinding({ fadeEnabled: false }));
    const pDisabled = runPaint(stDisabled, reply1, hour0 + 1, 24, makeBinding({ fadeEnabled: false }));
    assert.equal(pDisabled.motion, false, 'fadeEnabled=false skips fade-in');
  }
});

test('Direction B canonical audit & PR #292 market ledger parity preserve seller/buyer states, unread delivery amounts, and new-row badge', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
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
  assert.equal(model.rows.length, 2);
  const row0 = model.rows[0]!;
  const row1 = model.rows[1]!;
  const audit0 = canonicalLogRowAudit(row0);
  const audit1 = canonicalLogRowAudit(row1);
  assert.ok(audit0.includes('notRecorded'), 'unknown mode/buyer/amount encoded as notRecorded in canonical audit');
  assert.ok(audit0.includes('notApplicable'), 'missing seller encoded as notApplicable distinctly from notRecorded');
  assert.ok(audit1.includes('Design missing from recording'), 'missing equipment reason preserved in audit');
  assert.equal(row0.meta && ledgerSelection(row0.meta as never), null, 'mode-unknown kind 34 cannot open drill');
  assert.notEqual(row1.meta && ledgerSelection(row1.meta as never), null, 'valid contract row opens drill');

  const layout = buildLogLayout(model, 'comfortable', false);
  const auditWithBadge = deriveLogAuditRows(model, layout.items, 0, layout.items.length, 0, 0, 300, 7);
  const auditNoBadge = deriveLogAuditRows(model, layout.items, 0, layout.items.length, 0, 0, 300, 0);
  assert.notDeepEqual(auditWithBadge, auditNoBadge, 'new-row badge count is part of canonical audit');
  assert.ok(auditWithBadge.some(r => r.key === '__summary__' && r.value?.includes('newRowCount') && r.value?.includes('7')));
});
