import assert from 'node:assert/strict';
import test from 'node:test';
import type { LogCell, LogPageRow } from '../../generated/dashboard';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import type { FamilySources } from '../contract.ts';
import { notRecorded, zeroValue } from '../value.ts';
import { buildLogLayout, resolveLogColumns, visibleItemRange } from './layout.ts';
import { logLookAt } from './declaration.ts';
import {
  canonicalLogRowAudit,
  deriveLogAudit,
  deriveLogAuditRows,
  deriveLogMirrorItems,
  deriveLogTooltip,
} from './model.ts';
import { createLogPicture, drawLog, type LogTheme } from './painter.ts';
import { familyLogLaneView, type FamilyLogBinding, type LogMetaField } from './laneView.ts';
import { reconcileFrozenView, verifyLogAuditSnapshot, type FrozenViewState, type LogAuditSnapshot } from './frozenView.ts';
import './registerAll.ts';
import {
  mergeFrozenPageIntoTail,
  newestFirstWireRows,
  pageReplyToTail,
  scrollPageOrdinalRange,
  type LogTailReplyWire,
} from './tailReply.ts';
import { ledgerFacets, ledgerSelection, ledgerTitle, type LedgerItem } from '../../dashboard/marketLedger.ts';
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

function capturingCanvasContext() {
  const texts: { readonly text: string; readonly x: number; readonly y: number }[] = [];
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
    stroke() {},
    strokeRect() {},
    fillRect() {},
    fillText(text: string, x: number, y: number) {
      texts.push({ text, x, y });
    },
    drawImage() {},
    measureText(text: string) {
      return { width: text.length * 7 };
    },
  };
  return { ctx: ctx as never, texts };
}

test('market-ledger projection preserves notRecorded/missing/zero/notApplicable states, carries flagReason, guards kind-34 drill, and keeps audit presentation-free', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '4',
    rows: [
      makeMarketRow(3, hour, {
        kind: { kind: 'notRecorded' },
        contract: { kind: 'notRecorded' },
        variant: { kind: 'notRecorded' },
        amountRaw: { kind: 'notRecorded' },
      }),
      makeMarketRow(2, hour - 1, {
        kind: numCell(34),
        mode: { kind: 'notRecorded' },
        seller: { kind: 'missing' },
        buyer: { kind: 'notRecorded' },
        amountRaw: { kind: 'notRecorded' },
        variant: txtCell('0'),
        equipment: { kind: 'missing' },
      }),
      makeMarketRow(1, hour - 2, {
        kind: numCell(20),
        amountRaw: numCell('0'),
        equipment: { kind: 'missing' },
        equipmentMissing: txtCell('Design missing from recording'),
      }),
      makeMarketRow(0, hour - 3, {
        kind: numCell(12),
        variant: txtCell('0'),
        amountRaw: { kind: 'missing' },
      }),
    ],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const looks = {
    GER: [{ from: 200, name: 'Germany', flagReason: 'Flag atlas missing for GER' }],
    FRA: [{ from: 0, name: 'France' }],
  };
  assert.equal(logLookAt(looks, 'GER', 150), null);
  assert.equal(logLookAt(looks, 'GER', 200)?.name, 'Germany');

  const model = marketLedgerProjection.project(makeTailSources(reply), hour, { looks });
  const [unrecRow, unknownModeRow, zeroRow, naRow] = model.rows;
  assert.equal(unrecRow!.rawMagnitudes?.kind?.kind, 'notRecorded');
  assert.equal(unrecRow!.rawMagnitudes?.contract?.kind, 'notRecorded');
  assert.equal(unrecRow!.rawMagnitudes?.amount?.kind, 'notRecorded');
  assert.ok(canonicalLogRowAudit(unrecRow!).includes('Equipment design not recorded'));

  assert.deepEqual(unknownModeRow!.rawMagnitudes?.sellerTag, notRecorded('Country name unavailable'));
  assert.deepEqual(unknownModeRow!.rawMagnitudes?.buyerTag, notRecorded('Country not recorded'));
  assert.equal(unknownModeRow!.rawMagnitudes?.mode?.kind, 'notRecorded');
  assert.equal(unknownModeRow!.rawMagnitudes?.auxiliary?.kind, 'notApplicable');
  assert.equal(ledgerSelection(unknownModeRow!.meta as LedgerItem), null);
  assert.ok(!ledgerTitle(unknownModeRow!.meta as LedgerItem, t => t ?? '').toLowerCase().includes('lend-lease'));

  assert.deepEqual(zeroRow!.rawMagnitudes?.amount, zeroValue(0n));
  assert.ok(canonicalLogRowAudit(zeroRow!).includes('Design missing from recording'));
  const flagRow = deriveLogTooltip(zeroRow!)?.rows.find(r => r.label === 'Seller flag');
  assert.ok(flagRow?.value?.includes('Flag atlas missing for GER'));
  assert.equal(ledgerSelection(zeroRow!.meta as LedgerItem), 'contract:false:0000000000000012');

  assert.equal(naRow!.rawMagnitudes?.amount?.kind, 'notApplicable');
  assert.equal(naRow!.slots[1]?.kind === 'value' ? naRow!.slots[1].value.kind : null, 'notApplicable');
  assert.ok(canonicalLogRowAudit(naRow!).includes('notApplicable'));

  const audit = deriveLogAudit(model);
  assert.equal(audit.ordered, true);
  assert.deepEqual([...audit.values.keys()], ['__summary__', 'mkt:3', 'mkt:2', 'mkt:1', 'mkt:0']);
  assert.notEqual(canonicalLogRowAudit(unrecRow!), canonicalLogRowAudit(zeroRow!));

  const restyled = {
    ...zeroRow!,
    art: zeroRow!.art ? { ...zeroRow!.art, color: '#ff00ff' } : undefined,
    slots: zeroRow!.slots.map(slot => (slot.kind === 'text' ? { ...slot, tone: 'accent' as const, weight: 'bold' as const } : slot)),
  };
  assert.equal(canonicalLogRowAudit(restyled), canonicalLogRowAudit(zeroRow!));

  const layout = buildLogLayout(model, 'comfortable', false);
  assert.notDeepEqual(
    deriveLogAuditRows(model, layout.items, 0, layout.items.length, 0, 0, 300, 7),
    deriveLogAuditRows(model, layout.items, 0, layout.items.length, 0, 0, 300, 0),
  );

  const sameHourRows = [makeMarketRow(12, hour), makeMarketRow(11, hour), makeMarketRow(10, hour - 1)];
  assert.deepEqual(newestFirstWireRows(sameHourRows).map(r => r.ordinal), ['11', '12', '10']);
  assert.deepEqual(scrollPageOrdinalRange(50, 200, 64, 64), { cursor: '122', pageSize: 64 });
  const reversed = pageReplyToTail(
    { branchId: '1', log: 'market-events', columns: MARKET_COLUMNS, firstOrdinal: '122', total: '200', cursor: '186', rows: [makeMarketRow(122, 100), makeMarketRow(123, 101)], missing: [], warnings: [] },
    50,
    200,
  );
  assert.deepEqual(reversed.rows.map(r => r.key), ['mkt:123', 'mkt:122']);

  const partyFacet = ledgerFacets([20, 34], {
    kind: 'membership',
    membership: [{ country: 'GER', firstHour: 100, lastHour: 250 }],
    unobserved: [{ firstHour: 120, lastHour: 140 }],
  }).find(f => f.name === 'country|receiver');
  assert.deepEqual(partyFacet?.countries, []);
  assert.deepEqual(partyFacet?.unobserved, [{ firstHour: 120, lastHour: 140 }]);
});

test('rowCache never reuses another recording row with identical id, ordinal, hour, and cellCount but different cells', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
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
  const modelA = marketLedgerProjection.project(makeTailSources(replyA), hour, { looks: {} });
  const modelB = marketLedgerProjection.project(makeTailSources(replyB), hour, { looks: {} });
  assert.equal((modelA.rows[0]!.meta as LedgerItem).amountRaw, '100000');
  assert.equal((modelB.rows[0]!.meta as LedgerItem).amountRaw, '999000');
  assert.equal((modelB.rows[0]!.meta as LedgerItem).contract, '00000000000000bb');
  assert.notEqual(canonicalLogRowAudit(modelA.rows[0]!), canonicalLogRowAudit(modelB.rows[0]!));
});

test('narrow widths keep the figure column and append ellipsis instead of dropping trailing rich parts', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 5;
  const reply: LogTailReplyWire = {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '1',
    rows: [makeMarketRow(1, hour, { kind: numCell(21), mode: numCell(21), seller: txtCell('GER'), buyer: txtCell('FRA'), equipment: txtCell('Rifle'), amountRaw: numCell('12500') })],
    facetTotals: [],
    missing: [],
    warnings: [],
  };
  const model = marketLedgerProjection.project(makeTailSources(reply), hour, {
    looks: { GER: [{ from: 0, name: 'Germany' }], FRA: [{ from: 0, name: 'France' }] },
  });
  const layout = buildLogLayout(model, 'comfortable', false);
  for (const width of [344, 300]) {
    assert.ok(resolveLogColumns(model.columns, width, 12, 10).some(c => c.id === 'figure'));
    const { ctx, texts } = capturingCanvasContext();
    drawLog(ctx, createLogPicture(), {
      model, layout, density: 'comfortable', showMonths: false, width, height: 120, scrollTop: 0,
      theme: THEME, inspectedKey: null, focusedKey: null, selectedKey: null, icons: new Map(), dpr: 1,
    });
    assert.ok(texts.some(t => t.text.includes('CIC')));
    const subJoined = texts.filter(t => t.y > 20 && t.x > 80).map(t => t.text).join('');
    assert.ok(subJoined.endsWith('…'), `width ${width} truncated rich subline must end with ellipsis, got ${JSON.stringify(subJoined)}`);
  }
});

test('per-index frozen page store resists late A success/failure over B and preserves full-interval coverage on interior pages', () => {
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
  assert.deepEqual(merged.missing, [fullGap]);
  assert.ok(merged.inlineMissing?.some(g => g.reason === pageGap.reason));
  assert.deepEqual(merged.facetTotals, frozenTail.facetTotals);

  const frozen: FrozenViewState = {
    gen: 1, queryKey: 'q1', hour, liveSeq: 1, baseOrdinal: 0, total: 1000, tailLoadedCount: 64, pageSize: 128,
    pages: new Map(), inFlight: new Map(), errors: new Map(),
    visibleStart: 645, visibleEnd: 660, offset: 0, loadedPages: [], panels: null,
  };

  frozen.pages.set(5, pageB);
  assert.equal(reconcileFrozenView(frozen).activeError, null);
  assert.equal(frozen.offset, 640);

  frozen.pages.set(1, pageA);
  assert.equal(reconcileFrozenView(frozen).activeError, null);
  assert.equal(frozen.offset, 640);
  assert.equal((frozen.panels?.tail as LogTailReplyWire).rows[0]!.key, pageB.rows[0]!.key);

  frozen.pages.delete(1);
  frozen.errors.set(1, new Error('Page A failed'));
  assert.equal(reconcileFrozenView(frozen).activeError, null);
  assert.equal(frozen.offset, 640);

  frozen.visibleStart = 130;
  frozen.visibleEnd = 145;
  assert.equal(reconcileFrozenView(frozen).activeError?.pageIndex, 1);
});

test('verifyLogAuditSnapshot fetches expanded offset-0 page when loadedPages includes page 0', async () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 50;
  const rows100 = Array.from({ length: 100 }, (_, i) => makeMarketRow(i, hour));
  const tail64: LogTailReplyWire = {
    branchId: '1', log: 'market-events', columns: MARKET_COLUMNS,
    baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '100',
    rows: rows100.slice(0, 64), facetTotals: [], missing: [], warnings: [],
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
      return { ...tail64, rows: rows100 };
    },
    frozen: {
      hour, baseOrdinal: 0, total: 100, pageSize: 128, offset: 0,
      visibleStart: 70, visibleEnd: 74, loadedPages: [{ pageIndex: 0, offset: 0, limit: 128 }],
    },
  };
  const verified = await verifyLogAuditSnapshot(hour, snap);
  assert.equal(pageCalls, 1);
  assert.ok(verified?.values.has('mkt:70'));
});

test('mirror derives items from visibleItemRange past gaps with non-colliding gap indices', () => {
  const hour = HOUR_EPOCH + 1941 * 365 * 24 + 200;
  const reply: LogTailReplyWire = {
    branchId: '1', log: 'market-events', columns: MARKET_COLUMNS,
    baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '3',
    rows: [makeMarketRow(2, hour - 10), makeMarketRow(1, hour - 40), makeMarketRow(0, hour - 80)],
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
  assert.notEqual(gapEntry.index, row0Entry.index);
  assert.equal(row0Entry.index, 0);
  assert.ok((gapEntry.index ?? 0) < 0);

  const [visAtRow1] = visibleItemRange(layout.items, layout.rowTops[1]!, 80);
  assert.equal(deriveLogMirrorItems(layout.items, visAtRow1, 2, null)[0]!.key, 'mkt:1');
});

test('worker freezes the acknowledged liveSeq tail rather than a newer painted-but-unacknowledged hour', () => {
  if (typeof OffscreenCanvas === 'undefined') return;
  const hour0 = HOUR_EPOCH + 1940 * 365 * 24 + 100;
  const reply100: LogTailReplyWire = {
    branchId: '1', log: 'market-events', columns: MARKET_COLUMNS,
    baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '2',
    rows: [makeMarketRow(1, hour0), makeMarketRow(0, hour0 - 1)],
    facetTotals: [], missing: [], warnings: [],
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
      widget: 1, hour, now: 1000, speed: 24,
      binding: { surfaces: [{ width: 480, height: 300, dpr: 1 }] },
      model: {
        family: 'log', kind: 'market-ledger', options: { looks: {} }, density: 'comfortable', months: false,
        sources: { tail: 1 }, width: 480, height: 300, scrollTop: 0, theme: THEME, ...overrides,
      },
      subs: [{ id: 1, spec: {} as never }],
      pictures: [{ hour, through: hour, parts: [{ kind: 'panel', value: { kind: 'panel', data: reply, refused: null } }] }],
      inspect: null,
    } as never, state, () => `c${++seq}`);

  const meta100 = paint(hour0, reply100).fields.meta!.value as LogMetaField;
  paint(hour0 + 1, reply101);

  const pFrozen = paint(hour0 + 1, reply101, {
    frozenGen: 1, frozenHour: hour0, frozenLiveSeq: meta100.liveSeq, frozenTotal: meta100.total, scrollTop: 40,
  });
  const metaFrozen = pFrozen.fields.meta!.value as LogMetaField;
  assert.equal(metaFrozen.hour, hour0);
  assert.equal(metaFrozen.total, 2);
  assert.equal(pFrozen.fields.liveNewCount?.value, 1);
});
