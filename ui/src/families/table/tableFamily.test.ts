import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import { IdentityIndex } from '../identity/index.ts';
import { PENDING_ART, productArt, readyArt, type ArtResult } from '../art/types.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseAnswer } from '../../dashboard/follow/message.ts';
import { parseFollowed } from '../../test-support/followMessage.ts';
import { message, type Spec } from '../../dashboard/follow/messages.fixture.ts';
import { assertContinuity, assertFreshPaint, assertQuietFrames, assertTooltipDated, driveLane, firstRow, installRecordingCanvas, LaneTransport } from '../laneRuntime.fixture.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { liveKey } from '../../charts/live.ts';
import { valuesOf } from '../../test-support/liveValues.ts';
import { ROW_HEIGHT, tableSettingsOf, type TableSettings, type TableTheme } from './settings.ts';
import { datedIdentity } from '../identity/dated.fixture.ts';
import { resolveHeadSpan } from '../broker.ts';
import { tableHostAdapter } from './hostAdapter.ts';
import { tableLane } from './laneView.ts';
import { registerFamilyProjection } from '../registry.ts';
import type { ApmReply, CampaignClockReply, DivisionTemplatesReply, EventFeedPageRow, MarketLedgerTailReply, ScoreboardColumn, ScoreboardReply, ScriptPageRow } from '../../generated/dashboard';
import { pictureError } from '../../shell/perf.ts';
import { CLOCK_COLUMNS, CLOCK_TABLE_BASE, campaignClockProjection } from '../../widgets/campaign-clock/clockProjection.ts';
import { decisionsProjection } from '../../widgets/decisions/decisionsProjection.ts';
import { divisionTemplateColumns, divisionTemplateProjection } from '../../widgets/division-template/divisionTemplateProjection.ts';
import { eventChoicesProjection } from '../../widgets/event-choices/eventChoicesProjection.ts';
import { eventFeedProjection } from '../../widgets/event-feed/eventFeedProjection.ts';
import { gameScoreColumns, gameScoreProjection } from '../../widgets/game-score/gameScoreProjection.ts';
import { marketLedgerProjection } from '../../widgets/market-ledger/marketLedgerProjection.ts';
import { buildRowSourceFromHead, validateGenericTailReply } from './rowSource.ts';
import { COLUMNS as OCCUPIED_DEFAULTS, TABLE_BASE as OCCUPIED_BASE } from '../../widgets/occupied-states/definition.ts';
import { occupiedStatesProjection } from '../../widgets/occupied-states/occupiedProjection.ts';
import { apmTableColumns, apmTableProjection } from '../../widgets/player-apm/apmProjection.ts';
import { TABLE_BASE as SCOREBOARD_BASE } from '../../widgets/scoreboard/definition.ts';
import { RANK_SOURCE, scoreboardColumnsSpec, scoreboardProjection } from '../../widgets/scoreboard/scoreboardProjection.ts';
import { AuditSnapshotStore, canonicalAuditData, canonicalAuditValue } from '../audit.ts';
import { CadenceGate, TrailingFlushScheduler } from '../cadence.ts';
import { createBoundedReadView } from '../boundedView.ts';
import { holderRunAt, nextHoldersChange, nextRangesChange, nextWindowRangesChange, overlapsHourRanges, resolveSlotHolderRuns } from '../holders.ts';
import { showBundle } from '../readUnits.ts';
import { effectiveCountryUnobserved, selectionUnobservedReason } from '../sources.ts';
import { SlotTags } from '../../dashboard/countrySlots.ts';
import { groupUnobserved } from '../../dashboard/groups.ts';
import { capturePictures, capturePicturesBatch } from '../../dashboard/pulled.ts';
import { isFamilyAuditing, subscribeFamilyAuditing } from '../audit.ts';
import { InvalidFamilyValueError, knownValue, notApplicable, notRecorded, partialValue, zeroValue } from '../value.ts';
import { buildColumnLayouts, layoutFamilyTable, measureFixedColumnWidths } from './layout.ts';
import { computeColumnMaxima, declareTableProjection, tableRank, deriveTableAudit, deriveTableMirror, deriveTableTooltip, resolveCellPaintModel, resolveTableRows, resolveTableRowsIncremental, updateColumnMaximaIncremental, type FlatTableRow, type TableColumnSpec, type TableModel, type TableModelSlot, type TableRow } from './model.ts';
import { createTablePicture, drawFamilyTable, glideCrossing, hitFamilyTable, NO_GLIDE, tableGlide } from './painter.ts';
import { LaneArtCache } from '../art/laneCache.ts';
import { treeLevelsOf, treeTextStart } from './cellPainters.ts';
import { TREE_CHEVRON_LANE } from './layout.ts';
import { computeRowLayout, TABLE_WINDOW_OVERSCAN, tableRowWindow } from './layout.ts';
import { NOT_NAMED } from '../../widgets/occupied-states/occupiedStatesData.ts';
import { createLaneView } from '../laneRuntime.ts';
import { PayloadSender } from '../../dashboard/follow/payloads.ts';

const NO_OPTIONS = {};
const NO_SOURCES = {};
const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

const floatBits = (value: number): bigint => {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value, true);
  return view.getBigInt64(0, true);
};

function headTextAnswer(rows: readonly { readonly key: bigint; readonly id: string; readonly values: readonly (bigint | string | null)[] }[]) {
  const cols = rows[0]?.values.length ?? 0;
  const texts: string[] = [];
  const held = new Map<string, number>();
  const intern = (s: string) => {
    let at = held.get(s);
    if (at === undefined) held.set(s, at = texts.push(s) - 1);
    return at;
  };
  const names: number[] = [];
  const rawVals: bigint[] = [];
  const states: number[] = [];
  for (const row of rows) {
    names.push(intern(row.id));
    for (const v of row.values) {
      if (v === null) { rawVals.push(0n); states.push(CELL.missing); }
      else if (typeof v === 'string') { rawVals.push(BigInt(intern(v))); states.push(CELL.value); }
      else { rawVals.push(v); states.push(CELL.value); }
    }
  }
  const bytes: number[] = [];
  const u8 = (v: number) => bytes.push(v & 0xff);
  const u32 = (v: number) => { for (let i = 0; i < 4; i++) u8(Number((BigInt(v) >> BigInt(8 * i)) & 0xffn)); };
  const u64 = (v: bigint) => { for (let i = 0; i < 8; i++) u8(Number((BigInt.asUintN(64, v) >> BigInt(8 * i)) & 0xffn)); };
  const pad = () => { while (bytes.length % 8) u8(0); };
  u8(1); u8(1); u8(0); u8(0); u32(rows.length); u32(cols); u32(texts.length);
  rows.forEach(r => u64(r.key));
  rows.forEach(r => u64(r.key >> 64n));
  rawVals.forEach(v => u64(v));
  states.forEach(s => u8(s)); pad();
  for (let i = 0; i < rows.length; i++) u32(0xffff_ffff); pad();
  names.forEach(n => u32(n)); pad();
  const enc = texts.map(t => new TextEncoder().encode(t));
  let end = 0; u32(0);
  for (const b of enc) { end += b.length; u32(end); }
  pad();
  for (const b of enc) b.forEach(u8);
  pad();
  return parseAnswer(new Uint8Array(bytes));
}

const TEST_THEME: TableTheme = {
  surface: '#111111',
  text: '#eeeeee',
  muted: '#888888',
  line: '#333333',
  lineSoft: '#222222',
  accent: '#3b82f6',
  blue: '#3b82f6',
  red: '#ef4444',
  amber: '#f59e0b',
  green: '#22c55e',
  grey: '#888888',
  hatch: '#555555',
  font: 'sans-serif',
  fontSize: { small: 11, body: 13 },
  lineWidth: 1,
  hatchSpacing: 6,
};

function instrumentedContext() {
  let measureCalls = 0;
  let fillTextCalls = 0;
  const ctx = {
    font: '',
    fillStyle: '',
    strokeStyle: '',
    lineWidth: 1,
    textAlign: 'left',
    textBaseline: 'alphabetic',
    save() {},
    restore() {},
    beginPath() {},
    closePath() {},
    rect() {},
    roundRect() {},
    clip() {},
    fillRect() {},
    strokeRect() {},
    clearRect() {},
    moveTo() {},
    lineTo() {},
    stroke() {},
    fill() {},
    arc() {},
    drawImage() {},
    setLineDash() {},
    createLinearGradient() { return { addColorStop() {} }; },
    measureText(text: string) {
      measureCalls++;
      return { width: text.length * 7 };
    },
    fillText() {
      fillTextCalls++;
    },
  };
  return {
    ctx: ctx as unknown as CanvasRenderingContext2D,
    counts: () => ({ measureCalls, fillTextCalls }),
    reset: () => { measureCalls = 0; fillTextCalls = 0; },
  };
}

test('declarative table projection refuses bare null where a 5-state value is required', () => {
  const broken = declareTableProjection<{ items: { id: string; count: number }[] }, { id: string; count: number }, Record<string, never>>({
    kind: 'test-null-guard',
    columns: [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, value: row => knownValue(row.id) },
      { key: 'count', label: 'Count', as: 'plain', offered: ['plain'], priority: 1, numeric: true, value: () => null as unknown as ReturnType<typeof knownValue> },
    ],
    rows: reply => ({ rows: reply.items }),
    rowKey: row => row.id,
    rowName: row => row.id,
  });
  const model = broken.project(createBoundedReadView({
    hour: 10,
    panels: new Map([['main', { items: [{ id: 'a', count: 1 }] }]]),
  }), {});
  const settings = tableSettingsOf(undefined, model.columns, { density: 'comfortable', sort: { column: 'count', direction: 'descending' } });
  assert.throws(() => resolveTableRows(model, settings), InvalidFamilyValueError);
});

test('campaign-clock projection produces identical audit output for followed wire answers and exact replies', () => {
  const wire = answer({
    keys: [0n],
    columns: 4,
    values: [floatBits(3661), floatBits(2400), floatBits(1261), 1n],
    states: [CELL.value, CELL.value, CELL.value, CELL.value],
  });
  const reply: CampaignClockReply = {
    available: true,
    recordedSeconds: 3661,
    playingSeconds: 2400,
    pausedSeconds: 1261,
    complete: true,
    warnings: [],
  };
  const settings = tableSettingsOf(undefined, CLOCK_COLUMNS, CLOCK_TABLE_BASE);
  const clockLayouts = buildColumnLayouts(CLOCK_COLUMNS, settings);
  const followedModel = campaignClockProjection.project(createBoundedReadView({ hour: 50, answers: new Map([['main', wire]]) }), {});
  const exactModel = campaignClockProjection.project(createBoundedReadView({ hour: 50, panels: new Map([['main', reply]]) }), {});
  const followedAudit = deriveTableAudit({ model: followedModel, flatRows: resolveTableRows(followedModel, settings), visibleColumns: clockLayouts });
  const exactAudit = deriveTableAudit({ model: exactModel, flatRows: resolveTableRows(exactModel, settings), visibleColumns: clockLayouts });
  assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);
  assert.ok(followedAudit.find(r => r.key === 'total:time')?.value?.includes(canonicalAuditValue(knownValue(3661))));
});

test('scoreboard projection matches followed and exact audits with missing, partial, zero, and tied rows', () => {
  const columns: ScoreboardColumn[] = [
    { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null, family: { kind: 'f', measure: 'm' }, partial: null, noneAtZero: false },
    { key: 'casualties', label: 'Casualties', unit: 'people', decimals: 0, available: true, reason: null, family: { kind: 'c', measure: 't' }, partial: { kind: 'c', measure: 'p' }, noneAtZero: true },
  ];
  const identity = datedIdentity({
    GER: [{ from: 0, name: 'Germany' }],
    FRA: [{ from: 0, name: 'France' }],
    ITA: [{ from: 0, name: 'Italy' }],
    ENG: [{ from: 0, name: 'United Kingdom' }],
  });
  const rankAns = answer({ keys: [1n, 2n, 3n, 4n], columns: 1, names: ['ITA', 'FRA', 'GER', 'ENG'], values: [10n, 10n, 0n, 0n], states: [CELL.value, CELL.value, CELL.value, CELL.value] });
  const facAns = answer({
    keys: [1n, 2n, 3n, 4n],
    columns: 1,
    names: ['ITA', 'FRA', 'GER', 'ENG'],
    values: [10n, 10n, 0n, 0n],
    states: [CELL.value, CELL.value, CELL.value, CELL.missing],
    reasons: [null, null, null, 'Factories not recorded'],
  });
  const casAns = answer({
    keys: [1n, 2n, 3n, 4n],
    columns: 1,
    names: ['ITA', 'FRA', 'GER', 'ENG'],
    values: [500n, 0n, 200n, 0n],
    states: [CELL.value, CELL.value, CELL.value, CELL.value],
  });
  const partAns = answer({
    keys: [1n, 2n, 3n, 4n],
    columns: 1,
    names: ['ITA', 'FRA', 'GER', 'ENG'],
    values: [1n, 0n, 0n, 0n],
    states: [CELL.value, CELL.value, CELL.value, CELL.value],
  });
  const exactReply: ScoreboardReply = {
    branchId: '1',
    hour: 40,
    missing: null,
    warnings: [],
    rows: [
      { country: 'GER', tag: 'GER', cells: [{ valueRaw: '0', missing: null, partial: false }, { valueRaw: '200', missing: null, partial: false }] },
      { country: 'ENG', tag: 'ENG', cells: [{ valueRaw: null, missing: 'Factories not recorded', partial: false }, { valueRaw: '0', missing: null, partial: false }] },
      { country: 'ITA', tag: 'ITA', cells: [{ valueRaw: '10', missing: null, partial: false }, { valueRaw: '500', missing: null, partial: true }] },
      { country: 'FRA', tag: 'FRA', cells: [{ valueRaw: '10', missing: null, partial: false }, { valueRaw: '0', missing: null, partial: false }] },
    ],
  };

  const options = { shown: columns, candidates: ['GER', 'FRA', 'ITA', 'ENG'], top: 4, rankMarkers: false };
  const specs = scoreboardColumnsSpec(columns, 4);

  for (const direction of ['descending', 'ascending'] as const) {
    const settings: TableSettings = { ...tableSettingsOf(undefined, specs, SCOREBOARD_BASE), order: specs.map(s => s.key), sort: { column: 'factories', direction } };
    const followedModel = scoreboardProjection.project(createBoundedReadView({
      hour: 40,
      answers: new Map([[RANK_SOURCE, rankAns], ['factories', facAns], ['casualties', casAns], ['casualties:partial', partAns]]),
      identity,
    }), options);
    const exactModel = scoreboardProjection.project(createBoundedReadView({
      hour: 40,
      panels: new Map([['main', exactReply]]),
      identity,
    }), options);

    const followedFlat = resolveTableRows(followedModel, settings);
    const exactFlat = resolveTableRows(exactModel, settings);
    const sbLayouts = buildColumnLayouts(specs, settings);
    const followedAudit = deriveTableAudit({ model: followedModel, flatRows: followedFlat, visibleColumns: sbLayouts });
    const exactAudit = deriveTableAudit({ model: exactModel, flatRows: exactFlat, visibleColumns: sbLayouts });

    assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);
    assert.equal(followedFlat.at(-1)?.kind === 'data' && followedFlat.at(-1)?.key, 'ENG', `missing ENG sorts last in ${direction}`);
    const order = followedFlat.map(r => r.key);
    assert.ok(order.indexOf('FRA') < order.indexOf('ITA'), 'tied France precedes Italy by name A-Z regardless of wire vs reply input order');
    assert.ok(followedAudit.find(r => r.key === 'FRA:casualties')?.value?.includes(canonicalAuditValue(zeroValue(0))), 'known zero renders zero state');
    assert.ok(followedAudit.find(r => r.key === 'ITA:casualties')?.value?.includes(canonicalAuditValue(partialValue(500n, 'Some inputs were not recorded at this hour'))));
  }

  const hourly = { shown: columns, candidates: ['GER', 'FRA', 'ITA', 'ENG'], top: null, rankMarkers: false };
  const at = (hour: number, gerFactories: bigint) => scoreboardProjection.project(createBoundedReadView({
    hour,
    identity,
    answers: new Map([
      ['factories', answer({ keys: [1n, 2n, 3n, 4n], columns: 1, names: ['ITA', 'FRA', 'GER', 'ENG'], values: [10n, 10n, gerFactories, 0n], states: [CELL.value, CELL.value, CELL.value, CELL.missing], reasons: [null, null, null, 'Factories not recorded'] })],
      ['casualties', answer({ keys: [1n, 2n, 3n, 4n], columns: 1, names: ['ITA', 'FRA', 'GER', 'ENG'], values: [500n, 0n, 200n, 0n], states: [CELL.value, CELL.value, CELL.value, CELL.value] })],
      ['casualties:partial', answer({ keys: [1n, 2n, 3n, 4n], columns: 1, names: ['ITA', 'FRA', 'GER', 'ENG'], values: [1n, 0n, 0n, 0n], states: [CELL.value, CELL.value, CELL.value, CELL.value] })],
    ]),
  }), hourly);
  const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: '1', content: 1 });
  const style = (name: string, valid: [number, number]) => ({ tag: 'GER', name, color: [102, 102, 102] as [number, number, number], valid, flagChoice: { keys: ['GER', null, null] as [string, null, null], reasons: [null, null, null] as [null, null, null] } });
  index.ingestStylesReply({ branchId: '1', hour: 0, styles: [style('Germany', [0, 49])], warnings: [] });
  index.ingestStylesReply({ branchId: '1', hour: 50, styles: [style('German Reich', [50, 100])], warnings: [] });
  const renamed = createSnapshotIdentityProvider(index);
  const factoriesOnly = new Map([['factories', answer({ keys: [1n], columns: 1, names: ['GER'], values: [1n], states: [CELL.value] })]]);
  assert.equal(scoreboardProjection.project(createBoundedReadView({ hour: 40, identity: renamed, answers: factoriesOnly }), hourly).rows.find(row => row.key === 'GER')?.name, 'Germany');
  assert.equal(scoreboardProjection.project(createBoundedReadView({ hour: 50, identity: renamed, answers: factoriesOnly }), hourly).rows.find(row => row.key === 'GER')?.name, 'German Reich', 'a dated rename reaches the country cell through identity');

  const first = at(40, 0n);
  const quiet = at(41, 0n);
  assert.deepEqual(quiet.rows.map((row, index) => row === first.rows[index]), [true, true, true, true], 'a quiet hour\'s fresh answers rebuild no row');
  const moved = at(42, 7n);
  assert.deepEqual(moved.rows.map(row => row === quiet.rows.find(held => held.key === row.key)), moved.rows.map(row => row.key !== 'GER'), 'only the row whose answer changed is rebuilt');
});

test('apm-table projection matches followed wire answers and exact replies, including partial gaps and notApplicable zero minutes', () => {
  const wire = headTextAnswer([
    { key: 1n, id: 'GER', values: [floatBits(45.2), floatBits(45.2), 113n, 15n, floatBits(2.5), 1n, 1n, 'GER', 100n, 'GER', null, '113', '5', '2'] },
    { key: 2n, id: 'ITA', values: [null, null, 0n, 0n, floatBits(0), 1n, 0n, 'ITA', 100n, 'ITA', null, '0', '0', '1'] },
  ]);
  const reply: ApmReply = {
    available: true,
    warnings: [],
    players: [],
    series: [
      { id: 'GER', name: 'GER', nameMissing: null, tag: 'GER', hour: 100, summary: { apm: 45.2, confirmedApm: 45.2, orders: '113', confirmedOrders: '113', pausedOrders: '15', unpausedMinutes: 2.5, unpausedMinutesNum: '5', unpausedMinutesDen: '2', complete: true, gaps: true }, points: [] },
      { id: 'ITA', name: 'ITA', nameMissing: null, tag: 'ITA', hour: 100, summary: { apm: null, confirmedApm: null, orders: '0', confirmedOrders: '0', pausedOrders: '0', unpausedMinutes: 0, unpausedMinutesNum: '0', unpausedMinutesDen: '1', complete: true, gaps: false }, points: [] },
    ],
  };
  const opts = {
    players: false,
    showPaused: true,
   
    rankMarkers: false,
  };
  const cols = apmTableColumns(false, true);
  const settings = tableSettingsOf(undefined, cols, { density: 'comfortable', sort: { column: 'apm', direction: 'descending' } });
  const apmLayouts = buildColumnLayouts(cols, settings);
  const fModel = apmTableProjection.project(createBoundedReadView({ hour: 100, answers: new Map([['main', wire]]) }), opts);
  const eModel = apmTableProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', reply]]) }), opts);
  const fAudit = deriveTableAudit({ model: fModel, flatRows: resolveTableRows(fModel, settings), visibleColumns: apmLayouts });
  const eAudit = deriveTableAudit({ model: eModel, flatRows: resolveTableRows(eModel, settings), visibleColumns: apmLayouts });
  assert.equal(pictureError('table', fAudit, { values: new Map(eAudit.map(r => [r.key, r.value!])), ordered: true }), null);
  assert.ok(fAudit.find(r => r.key === 'GER:apm')?.value?.includes('["partial",["str","45.2"],'));
  assert.ok(fAudit.find(r => r.key === 'ITA:apm')?.value?.includes('["notApplicable",'));
});

test('occupied-states, game-score (with faction groups), and division-template (with obsolete tree) project deterministically', () => {
  const occReply = {
    states: [
      { state: 1, name: 'Warsaw', nameMissing: null, owner: 'POL', occupier: 'GER', occupied: 'POL', garrisonRequiredSign: 1, garrisonAlert: false, assignedKnown: true, garrisonMissing: null, law: 'civilian', lawMissing: null },
    ],
    complete: true,
    missing: null,
    laws: [{ token: 'civilian', name: 'Civilian Oversight', icon: null, missing: null }],
    strengthMissing: 'Missing strength',
    garrisonRedRaw: '30000',
    warnings: [],
  };
  const occModel = occupiedStatesProjection.project(createBoundedReadView({
    hour: 50,
    identity: datedIdentity({ GER: [{ from: 0, name: 'Germany' }], POL: [{ from: 0, name: 'Poland' }] }),
    panels: new Map([['main', occReply]]),
    live: new Map([['main', { get: k => (k.endsWith('resistance') ? 2_500_000 : k.endsWith('compliance') ? 1_000_000 : k.endsWith('garrison') ? 500_000 : 100_000), missing: () => 'Not recorded' }]]),
  }), {
    filter: { passing: null, unrecorded: [], bounds: [] },
    role: 'occupier',
    lawAs: 'dot',
    filterAll: true,
  });
  const occSettings = tableSettingsOf(undefined, OCCUPIED_DEFAULTS, OCCUPIED_BASE);
  const occRows = resolveTableRows(occModel, occSettings);
  assert.equal(occRows.length, 1);
  const occLayouts = buildColumnLayouts(occModel.columns, occSettings);
  assert.ok(deriveTableAudit({ model: occModel, flatRows: occRows, visibleColumns: occLayouts }).find(r => r.key === '1:resistance')?.value?.includes(canonicalAuditValue(knownValue(2_500_000))));

  const gsReply = {
    missing: null,
    warnings: [],
    hints: {},
    multipliers: [{ part: 'total', raw: '100000', missing: null }],
    rows: [
      { tag: 'GER', total: { missing: null }, army: { missing: null }, navy: { missing: null }, air: { missing: null }, industry: { missing: null }, land: { missing: null } },
      { tag: 'ENG', total: { missing: null }, army: { missing: null }, navy: { missing: null }, air: { missing: null }, industry: { missing: null }, land: { missing: null } },
    ],
  };
  const gsModel = gameScoreProjection.project(createBoundedReadView({
    hour: 50,
    identity: datedIdentity({ GER: [{ from: 0, name: 'Germany' }], ENG: [{ from: 0, name: 'United Kingdom' }] }),
    panels: new Map([['main', gsReply]]),
    live: new Map([['main', { get: k => (k.startsWith('GER') ? 300 : 200), missing: () => 'Not recorded' }]]),
  }), {
    all: true,
    holders: null,
    membership: null,
    top: null,
    by: 'total',
    factions: true,
    factionRuns: [
      { id: '1', tag: 'GER', firstHour: 0, lastHour: 100, names: [{ firstHour: 0, lastHour: 100, name: 'Axis' }] },
      { id: '2', tag: 'ENG', firstHour: 0, lastHour: 100, names: [{ firstHour: 0, lastHour: 100, name: 'Allies' }] },
    ],
    rankMarkers: false,
   
  });
  const gsCols = gameScoreColumns(undefined);
  const gsFlat = resolveTableRows(gsModel, tableSettingsOf(undefined, gsCols, { density: 'comfortable', sort: { column: 'total', direction: 'descending' } }));
  assert.deepEqual(gsFlat.filter(r => r.kind === 'group-header').map(r => r.label), ['Allies', 'Axis']);

  const gsIncOptions = { all: true, holders: null, membership: null, top: null, by: 'total' as const, factions: false, factionRuns: null, rankMarkers: false };
  const gsLive = new Map([['main', { get: (key: string) => (key.startsWith('GER') ? 300 : 200), missing: () => 'Not recorded' }]]);
  const gsAt = (hour: number, identity?: ReturnType<typeof datedIdentity>) => gameScoreProjection.project(createBoundedReadView({
    hour,
    panels: new Map([['main', gsReply]]),
    live: gsLive,
    ...(identity ? { identity } : {}),
  }), gsIncOptions);
  const waiting = gsAt(60, datedIdentity({ GER: [{ from: 0, name: 'Germany' }] }));
  const loadingEng = waiting.rows.find(row => row.key === 'ENG')!;
  assert.notEqual(loadingEng.name, 'ENG', 'a name still loading is never the tag as if it were the name');
  assert.deepEqual(loadingEng.cells.country!.value, notRecorded('Name is loading'), 'a name still loading is a missing state that says it is loading');
  assert.equal(loadingEng.cells.country!.loading, true);
  const waitingFlat = resolveTableRows(waiting, tableSettingsOf(undefined, gameScoreColumns(undefined), { density: 'comfortable', sort: { column: 'total', direction: 'descending' } }));
  const waitingCol = buildColumnLayouts(gameScoreColumns(undefined), tableSettingsOf(undefined, gameScoreColumns(undefined), { density: 'comfortable', sort: { column: 'total', direction: 'descending' } })).find(col => col.key === 'country')!;
  const waitingPaint = resolveCellPaintModel(loadingEng.cells.country, waitingCol, new Map());
  assert.equal(waitingPaint.loading, true, 'the painted cell knows the name is loading');
  assert.match(deriveTableMirror(waitingFlat, waiting.columns, false).rows.find(row => row.key === 'ENG')!.cells.find(cell => cell.column === 'country')!.spoken, /^Loading/, 'the mirror says loading, never not recorded');
  assert.deepEqual(waiting.rows.find(row => row.key === 'GER')!.cells.country!.value, knownValue('Germany'));
  const named = gsAt(61, datedIdentity({ GER: [{ from: 0, name: 'Germany' }], ENG: [{ from: 0, name: 'United Kingdom' }] }));
  assert.deepEqual(named.rows.map(row => row.name), ['Germany', 'United Kingdom'], 'names that arrive with no live change rebuild the rows');

  const renamed = gameScoreProjection.project(createBoundedReadView({
    hour: 50,
    panels: new Map([['main', gsReply]]),
    live: gsLive,
    identity: datedIdentity({ GER: [{ from: 0, name: 'Germany' }, { from: 70, name: 'German Reich' }], ENG: [{ from: 0, name: 'United Kingdom' }] }),
  }), gsIncOptions);
  assert.equal(renamed.nextChangeAt, 70, 'a declarative table names its dated name changes as its next change');

  const gsTopOptions = { all: false, holders: null, membership: null, top: 1, by: 'army' as const, factions: false, factionRuns: null, rankMarkers: false };
  const gsTopAt = (top: unknown, live: (key: string) => number) => gameScoreProjection.project(createBoundedReadView({
    hour: 50,
    panels: new Map([['main', { ...gsReply, ...(top === undefined ? {} : { top }) }]]),
    live: new Map([['main', { get: live, missing: () => 'Not recorded' }]]),
  }), gsTopOptions);
  const gsRanked = gsTopAt({ tags: ['ENG'], missing: null }, key => (key.startsWith('GER') ? 300 : 200));
  assert.deepEqual(gsRanked.rows.map(row => row.key), ['ENG'], 'the top is the product\u2019s, not a client sort of live values that favour GER');
  const gsRefused = gsTopAt({ tags: [], missing: 'The top 1 by army score needs every country\u2019s army score, and 1 of 3 were not recorded at this hour' }, () => 1);
  assert.deepEqual([gsRefused.rows.length, gsRefused.empty], [0, 'The top 1 by army score needs every country\u2019s army score, and 1 of 3 were not recorded at this hour'], 'an unproven top is refused with the product\u2019s reason');
  assert.equal(gsTopAt(undefined, () => 1).rows.length, 0, 'a top never answered is not ranked in the client');

  const dtReply: DivisionTemplatesReply = {
    tag: 'GER',
    countFor: 'owner',
    fieldedMissing: null,
    warnings: [],
    templates: [
      { template: 'inf', name: 'Infantry', nameMissing: null, icon: null, iconMissing: null, rosterOrdinal: 1, obsolete: false, obsoleteHour: null, fielded: 24, baseWidthRaw: '2000000', trainingDays: 90, priority: 1, priorityName: 'Default', priorityMissing: null, units: [{ key: 'infantry', name: 'Infantry', nameMissing: null, local: true, count: 9, support: false }], unitsMissing: null, locked: false, forceRecruiting: false, newEquipment: true, foreignEquipment: true, manpower: 9000, settingsMissing: null, designer: 'ordinary' },
      { template: 'old', name: 'Old Cav', nameMissing: null, icon: null, iconMissing: null, rosterOrdinal: 2, obsolete: true, obsoleteHour: 10, fielded: 0, baseWidthRaw: '1200000', trainingDays: 60, priority: 0, priorityName: 'Reserve', priorityMissing: null, units: [], unitsMissing: null, locked: false, forceRecruiting: false, newEquipment: true, foreignEquipment: true, manpower: 4000, settingsMissing: null, designer: 'ordinary' },
    ],
  };
  const dtCols = divisionTemplateColumns();
  const dtSettings = tableSettingsOf(undefined, dtCols, { density: 'comfortable', sort: { column: 'template', direction: 'ascending' } });
  const dtModel = divisionTemplateProjection.project(createBoundedReadView({ hour: 50, panels: new Map([['main', dtReply]]), live: new Map([['main', repliedCells(dtReply).read()]]) }), {
    countryName: 'Germany',
   
    canOpenDesigner: true,
  });
  assert.equal(resolveTableRows(dtModel, dtSettings, {}).length, 2, 'obsolete group collapsed by default');
  assert.equal(resolveTableRows(dtModel, dtSettings, { obsolete: true }).length, 3, 'expanding obsolete reveals child template');
});

test('dirty cell painter repaints only changed cells and bounds text measurements after warm-up', () => {
  const columns: TableColumnSpec[] = [
    { key: 'name', label: 'Country', as: 'plain', offered: ['plain'], priority: 0, text: true, widest: ['Country 999'] },
    { key: 'v1', label: 'Score', as: 'faint', tone: 'blue', offered: ['plain', 'faint'], priority: 1, numeric: true, format: 'integer', widest: ['999,999'] },
    { key: 'v2', label: 'Rate', as: 'plain', offered: ['plain'], priority: 2, numeric: true, format: 'decimal1', widest: ['999.9'] },
  ];
  const settings = tableSettingsOf(undefined, columns, { density: 'comfortable', sort: { column: 'v1', direction: 'descending' } });
  const baseCols = buildColumnLayouts(columns, settings);
  const { ctx, counts, reset } = instrumentedContext();
  const fixedWidths = measureFixedColumnWidths(baseCols, settings, TEST_THEME, text => ctx.measureText(text).width);
  const layout = layoutFamilyTable(buildColumnLayouts(columns, settings, fixedWidths), 480, { rankShown: false, rankWidth: 0 });
  const picture = createTablePicture();

  const makeRows = (vals: [number, number][]) => resolveTableRows({
    columns,
    hour: 1,
    rows: vals.map(([a, b], idx) => ({
      key: `C${idx}`,
      name: `Country ${idx}`,
      cells: {
        name: { value: knownValue(`Country ${idx}`) },
        v1: { value: knownValue(a), format: 'integer' },
        v2: { value: idx === 2 ? notRecorded('No data') : idx === 3 ? notApplicable('N/A') : partialValue(b, 'Gaps'), format: 'decimal1' },
      },
    })),
  }, settings);

  const r1 = makeRows([[100, 1.5], [90, 2.5], [80, 0], [70, 0], [60, 4.5]]);
  const firstDraw = drawFamilyTable(ctx, { layout, rows: r1, settings, hour: 1, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 1, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, dpr: 1 }, picture);
  assert.equal(firstDraw.paintedCells, 15, 'initial paint draws all 5x3 visible cells');

  reset();
  const sameDraw = drawFamilyTable(ctx, { layout, rows: r1, settings, hour: 2, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 2, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, dpr: 1 }, picture);
  assert.equal(sameDraw.paintedCells, 0, 'unchanged frame paints zero cells');
  assert.equal(sameDraw.drew, false);
  assert.equal(counts().measureCalls, 0);

  const r2 = makeRows([[100, 1.5], [90, 2.5], [80, 0], [70, 0], [60, 9.9]]);
  reset();
  const oneCellDraw = drawFamilyTable(ctx, { layout, rows: r2, settings, hour: 3, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 3, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, dpr: 1 }, picture);
  assert.equal(oneCellDraw.paintedCells, 1, 'changing one cell without reordering repaints exactly 1 cell');

  const r3 = makeRows([[100, 1.5], [95, 2.5], [80, 0], [70, 0], [99, 9.9]]);
  reset();
  const swapDraw = drawFamilyTable(ctx, { layout, rows: r3, settings, hour: 4, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 4, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, dpr: 1 }, picture);
  assert.ok(swapDraw.paintedCells > 0 && swapDraw.paintedCells <= 15, `reorder repaints only dirty rows: ${swapDraw.paintedCells}`);

  reset();
  for (let frame = 0; frame < 60; frame++) {
    const stepRows = makeRows([[100, 1.5], [90, 2.5], [80, 0], [70, 0], [60, (frame % 5) + 0.5]]);
    drawFamilyTable(ctx, { layout, rows: stepRows, settings, hour: 10 + frame, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 10 + frame, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, dpr: 1 }, picture);
  }
  assert.ok(counts().measureCalls <= 15, `text measurements stay bounded after warm-up: ${counts().measureCalls} across 60 frames`);
});

test('1,500 countries project, sort, audit identically, and paint only visible rows with bounded measurements', () => {
  const count = 1500;
  const tags = Array.from({ length: count }, (_, i) => `C${String(i).padStart(4, '0')}`);
  const identity = datedIdentity(Object.fromEntries(tags.map(tag => [tag, [{ from: 0, name: `Country ${tag}` }]])));
  const columns: ScoreboardColumn[] = [
    { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null, family: { kind: 'f', measure: 'm' }, partial: null, noneAtZero: false },
  ];
  const values = tags.map((_, i) => BigInt((i * 7919 + 17) % 10000));
  const wire = answer({
    keys: tags.map((_, i) => BigInt(i + 1)),
    columns: 1,
    names: tags,
    values,
    states: tags.map((_, i) => (i % 50 === 0 ? CELL.unrecorded : CELL.value)),
  });
  const reply: ScoreboardReply = {
    branchId: '1',
    hour: 200,
    missing: null,
    warnings: [],
    rows: tags.map((tag, i) => ({
      country: tag,
      tag,
      cells: [{ valueRaw: i % 50 === 0 ? null : String(values[i]!), missing: i % 50 === 0 ? 'Not recorded at this hour' : null, partial: false }],
    })),
  };

  const options = { shown: columns, candidates: tags, top: null, rankMarkers: false };
  const specs = scoreboardColumnsSpec(columns, null);
  const settings: TableSettings = { ...tableSettingsOf(undefined, specs, SCOREBOARD_BASE), order: specs.map(s => s.key), sort: { column: 'factories', direction: 'descending' } };
  const followedModel = scoreboardProjection.project(createBoundedReadView({ hour: 200, answers: new Map([['factories', wire]]), identity }), options);
  const exactModel = scoreboardProjection.project(createBoundedReadView({ hour: 200, panels: new Map([['main', reply]]), identity }), options);
  const followedRows = resolveTableRows(followedModel, settings);
  const exactRows = resolveTableRows(exactModel, settings);
  assert.equal(followedRows.length, count);

  const sb1500Layouts = buildColumnLayouts(specs, settings);
  const followedAudit = deriveTableAudit({ model: followedModel, flatRows: followedRows, visibleColumns: sb1500Layouts, window: { first: 0, count: 30 } });
  const exactAudit = deriveTableAudit({ model: exactModel, flatRows: exactRows, visibleColumns: sb1500Layouts, window: { first: 0, count: 30 } });
  assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);

  const { ctx, counts, reset } = instrumentedContext();
  const baseCols = buildColumnLayouts(specs, settings);
  const fixedWidths = measureFixedColumnWidths(baseCols, settings, TEST_THEME, text => ctx.measureText(text).width);
  const layout = layoutFamilyTable(buildColumnLayouts(specs, settings, fixedWidths), 480, { rankShown: true, rankWidth: 36 });
  const picture = createTablePicture();
  reset();
  const draw = drawFamilyTable(ctx, { layout, rows: followedRows, settings, hour: 200, width: 480, height: 320, scrollLeft: 0, scrollTop: 0, rankShown: true, rankWidth: 36, theme: TEST_THEME, now: 1, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, dpr: 1 }, picture);
  assert.ok(draw.paintedCells <= 30, `only visible viewport rows painted out of 1,500: ${draw.paintedCells} cells`);
  assert.ok(counts().measureCalls <= 50, `bounded text measurements for 1,500-country table: ${counts().measureCalls}`);
});

test('occupied-states projection names countries at the hour, filter ranges, unrecorded membership, and live tooltip details', () => {
  const mkState = (id: number, occupier: string | null, occupied: string | null, extra: Record<string, unknown> = {}) => ({
    state: id, name: `State ${id}`, nameMissing: null, owner: occupied, occupier, occupied, garrisonRequiredSign: 1 as const,
    garrisonAlert: false, assignedKnown: true, garrisonMissing: null, law: 'mod_law', lawMissing: null, ...extra,
  });
  const reply = {
    states: [mkState(1, 'GER', 'POL'), mkState(2, 'SOV', 'POL'), mkState(3, null, 'POL')],
    complete: true, missing: null,
    laws: [{ token: 'mod_law', name: 'Moderate', icon: null, missing: null }],
    strengthMissing: 'Strength needs inputs', garrisonRedRaw: '30000', warnings: [],
  };
  const filter = { passing: { GER: [[10, 20]] as const }, unrecorded: [[30, 40, 'Membership not recorded']] as const, bounds: [10, 21, 30, 41] };
  const live = valuesOf(new Map([
    [liveKey('1', 'resistance'), 2_500_000],
    [liveKey('1', 'compliance'), 0],
    [liveKey('1', 'requiredManpower'), 1200],
    [liveKey('1', 'garrison'), 250_000],
    [liveKey('1', 'garrisonStrength'), 87_500],
  ]), NOT_NAMED);
  const index = new IdentityIndex({ sourceId: 's', revision: '1', branchId: '1', content: 1 });
  const style = (tag: string, name: string, valid: [number, number]) => ({ tag, name, color: [102, 102, 102] as [number, number, number], valid, flagChoice: { keys: [tag, null, null] as [string, null, null], reasons: [null, null, null] as [null, null, null] } });
  index.ingestStylesReply({ branchId: '1', hour: 0, styles: [style('GER', 'Germany', [0, 19]), style('POL', 'Poland', [0, 100])], warnings: [] });
  index.ingestStylesReply({ branchId: '1', hour: 20, styles: [style('GER', 'German Reich', [20, 100])], warnings: [] });
  const identity = createSnapshotIdentityProvider(index);
  const run = (hour: number) => occupiedStatesProjection.project(
    createBoundedReadView({ hour, identity, panels: new Map([['main', reply]]), live: new Map([['main', live]]) }),
    { filter, role: 'occupier', lawAs: 'dot', filterAll: false },
  );
  const at15 = run(15);
  assert.deepEqual(at15.rows.map(r => r.key), ['1']);
  assert.equal(at15.nextChangeAt, 20, 'the occupier is renamed at 20, so the model holds until then');
  const settings = tableSettingsOf(undefined, OCCUPIED_DEFAULTS, OCCUPIED_BASE);
  const flat15 = resolveTableRows(at15, settings);
  const tip = deriveTableTooltip(flat15, '1', at15.columns);
  assert.ok(tip);
  assert.equal(tip.rows.find(r => r.label === 'Resistance')?.valueState?.text, '25%');
  assert.equal(tip.rows.find(r => r.label === 'Assigned manpower')?.valueState?.text, '0', 'known assigned with no live cell reads 0');
  assert.equal(tip.rows.find(r => r.label === 'Garrison strength')?.valueState?.text, '87.5%');
  assert.equal(tip.rows.find(r => r.label === 'Occupied country')?.valueState?.text, 'Poland');
  const waitingNames = (names: Parameters<typeof datedIdentity>[0]) => occupiedStatesProjection.project(
    createBoundedReadView({ hour: 15, identity: datedIdentity(names), panels: new Map([['main', reply]]), live: new Map([['main', live]]) }),
    { filter, role: 'occupier', lawAs: 'dot', filterAll: false },
  );
  const beforeNames = waitingNames({ GER: [{ from: 0, name: 'Germany' }] });
  const beforeTip = deriveTableTooltip(resolveTableRows(beforeNames, settings), '1', beforeNames.columns)!;
  assert.deepEqual(beforeTip.rows.find(r => r.label === 'Occupied country')?.valueState, { kind: 'missing', reason: 'Name is loading' }, 'the occupied country still loading says so, never the tag');
  const afterNames = waitingNames({ GER: [{ from: 0, name: 'Germany' }], POL: [{ from: 0, name: 'Poland' }] });
  const afterTip = deriveTableTooltip(resolveTableRows(afterNames, settings), '1', afterNames.columns)!;
  assert.equal(afterTip.rows.find(r => r.label === 'Occupied country')?.valueState?.text, 'Poland', 'once names arrive the tooltip names the occupied country');

  assert.equal(run(25).empty, 'Occupied states unavailable for these countries.');
  assert.equal(run(35).empty, 'Membership not recorded');
  assert.equal(run(20).rows[0]!.cells.occupier!.value.kind, 'known');
  assert.equal((run(20).rows[0]!.cells.occupier!.value as { value: string }).value, 'German Reich');
});

test('layout hides lowest-priority columns into tooltip or keeps all columns in scroll mode, and bigints sort beyond 2^53', () => {
  const specs: TableColumnSpec[] = [
    { key: 'state', label: 'State', as: 'plain', offered: ['plain'], priority: 0, text: true, widest: ['State Name'] },
    { key: 'resistance', label: 'Resistance', as: 'meterAfter', offered: ['meterAfter'], priority: 1, numeric: true },
    { key: 'owner', label: 'Owner', as: 'plain', offered: ['plain'], priority: 6, text: true },
  ];
  const settings = tableSettingsOf(undefined, specs, { density: 'comfortable', sort: { column: 'resistance', direction: 'descending' } });
  const cols = buildColumnLayouts(specs, settings, { state: 140, resistance: 130, owner: 140 });
  const narrowLayout = layoutFamilyTable(cols, 370);
  assert.deepEqual(narrowLayout.columns.map(c => c.key), ['state', 'resistance', 'owner'], 'a narrow table keeps every column and scrolls');
  assert.equal(narrowLayout.scrollable, true);
  assert.deepEqual(narrowLayout.columns.map(c => [c.leadInset ?? 0, c.trailInset ?? 0]), [[6, 0], [0, 0], [0, 6]], 'the 6 px side margin sits on the first and last columns');
  const scrollLayout = layoutFamilyTable(cols, 250);
  assert.equal(scrollLayout.scrollable, true);
  assert.equal(scrollLayout.pinnedWidth, 146);
  const narrowing = buildColumnLayouts(specs.map(spec => spec.key === 'owner' ? { ...spec, narrow: 'hide' as const } : spec), settings, { state: 140, resistance: 130, owner: 140 });
  assert.equal(layoutFamilyTable(narrowing, 500).columns.some(c => c.narrowed), false, 'a table that fits keeps every cell\'s full text');
  assert.deepEqual(layoutFamilyTable(narrowing, 370).columns.map(c => [c.key, c.narrowed]), [['state', true], ['resistance', true]], 'a narrowed table lets cells draw their narrow text');

  const bigModel = {
    columns: specs.slice(0, 2),
    hour: 1,
    rows: [
      { key: 'A', name: 'A', cells: { state: { value: knownValue('A') }, resistance: { value: knownValue((1n << 60n) + 1n) } } },
      { key: 'B', name: 'B', cells: { state: { value: knownValue('B') }, resistance: { value: knownValue((1n << 60n) + 2n) } } },
    ],
  };
  assert.deepEqual(resolveTableRows(bigModel, settings).map(r => r.key), ['B', 'A']);
});

test('lane view skips projection on next(), caches unchanged sources across serves, throttles mirror to 1 Hz while playing, and omits audit unless requested', () => {
  let projectCalls = 0;
  const proj = declareTableProjection<{ rows: { id: string; v: number }[] }, { id: string; v: number }, Record<string, never>>({
    kind: 'test-lane-cache',
    columns: [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, path: 'id' },
      { key: 'v', label: 'Val', as: 'plain', offered: ['plain'], priority: 1, numeric: true, format: 'integer', path: 'v' },
    ],
    rows: reply => {
      projectCalls++;
      return { rows: reply.rows };
    },
    rowKey: row => row.id,
    rowName: row => row.id,
  });
  registerFamilyProjection(proj);
  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return { ...instrumentedContext().ctx, setTransform() {} }; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const view = createLaneView(tableLane, () => {});
    const settings = tableSettingsOf(undefined, [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'] },
      { key: 'v', label: 'Val', as: 'plain', offered: ['plain'] },
    ], { density: 'comfortable', sort: { column: 'v', direction: 'descending' } });
    const staticPanels = { main: { rows: [{ id: 'GER', v: 10 }] } };
    const options = {};
    const model = {
      family: 'table' as const, theme: TEST_THEME,
      kind: 'test-lane-cache',
      options,
      settings,
      sources: {},
      staticPanels,
      width: 320,
      height: 200,
      scrollLeft: 0,
      scrollTop: 0,
      window: { first: 0, count: 10 },
    };
    const scene = { widget: 1, subs: [], pictures: [], model };
    assert.equal(view.next?.(scene, 10), null, 'with nothing held, next() proves no boundary and the lane paints at its target');
    assert.equal(projectCalls, 0, 'next() must never invoke projection.project()');

    const state = new Map<string, unknown>();
    let seq = 1;
    const content = () => `c${seq++}`;
    const transport = new LaneTransport();
    const plain = transport.carry(model);
    const audited = transport.carry({ ...model, audit: true });
    const mkInput = (now: number, speed: number, audit = false) => ({
      ...scene,
      model: audit ? audited : plain,
      hour: 10,
      seek: 0,
      speed,
      binding: { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now,
    });
    const first = view.paint(mkInput(1000, 1440), state, content);
    assert.equal(projectCalls, 1);
    assert.equal(first.fields.audit, undefined, 'audit omitted when bound.audit is false');
    const firstMirrorContent = first.fields.mirror!.content;

    const second = view.paint(mkInput(1100, 1440), state, content);
    assert.equal(projectCalls, 1, 'same sources/options/hour reuses cached projection');
    assert.equal(second.fields.mirror!.content, firstMirrorContent, 'mirror stays cached under 1 Hz while playing');

    const withAudit = view.paint(mkInput(1200, 1440, true), state, content);
    const auditVal = withAudit.fields.audit?.value as { id: number; rows: unknown[] } | undefined;
    assert.ok(auditVal && Array.isArray(auditVal.rows), 'audit built only when requested');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }
});

test('dirty mirror and tooltip flush on cadence or pause, focus and hover stay independent, animated hit testing uses painted Y, and meta/audit/headSpan hold invariants', () => {
  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return { ...instrumentedContext().ctx, setTransform() {} }; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  let wakes = 0;
  try {
    const view = createLaneView(tableLane, () => { wakes++; });
    const transport = new LaneTransport();
    const settings = tableSettingsOf(undefined, [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'] },
      { key: 'v', label: 'Val', as: 'plain', offered: ['plain'] },
    ], { density: 'comfortable', sort: { column: 'v', direction: 'descending' } });
    const makeModel = (rows: { id: string; v: number }[], focusedKey: string | null = null, extra: Record<string, unknown> = {}) => ({
      family: 'table' as const, theme: TEST_THEME,
      kind: 'test-lane-cache',
      options: NO_OPTIONS,
      settings,
      sources: NO_SOURCES,
      staticPanels: { main: { rows } },
      width: 320,
      height: 200,
      scrollLeft: 0,
      scrollTop: 0,
      focusedKey,
      window: { first: 0, count: 10 },
      ...extra,
    });
    const state = new Map<string, unknown>();
    let seq = 1;
    const content = () => `c${seq++}`;
    const paintAt = (rows: { id: string; v: number }[], now: number, speed: number, inspect: string | null = null, focusedKey: string | null = null, extra: Record<string, unknown> = {}) =>
      view.paint({
        widget: 1,
        subs: [],
        pictures: [],
        model: transport.carry(makeModel(rows, focusedKey, extra)),
        hour: 10,
        seek: 0,
        speed,
        binding: { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null },
        inspect,
        now,
      }, state, content);

    const rowsA = [{ id: 'GER', v: 20 }, { id: 'FRA', v: 10 }];
    const rowsB = [{ id: 'GER', v: 5 }, { id: 'FRA', v: 30 }];
    const p1 = paintAt(rowsA, 1000, 1440, 'row:FRA', 'GER');
    assert.deepEqual((p1.fields.mirror!.value as { rows: { key: string }[] }).rows.map(r => r.key), ['GER', 'FRA']);
    assert.equal((p1.fields.tooltip!.value as { text: { rowKey: string } }).text.rowKey, 'FRA', 'hover tooltip tracks inspected FRA');
    assert.equal((p1.fields.focusTooltip?.value as { text: { rowKey: string } } | undefined)?.text.rowKey, 'GER', 'focusTooltip tracks focused GER independently of hover');

    const p2 = paintAt(rowsB, 1100, 1440, 'row:FRA', 'GER');
    assert.equal(p2.fields.mirror, undefined, 'inside the 1 s window the mirror of changed rows is withheld, never the old rows beside the new picture');

    const p3 = paintAt(rowsB, 2100, 1440, 'row:FRA', 'GER');
    assert.deepEqual((p3.fields.mirror!.value as { rows: { key: string }[] }).rows.map(r => r.key), ['FRA', 'GER'], 'dirty mirror flushes when due even without a new model change');

    paintAt(rowsA, 2200, 1440, 'row:FRA', 'GER');
    const pPause = paintAt(rowsA, 2250, 0, 'row:FRA', 'GER');
    assert.deepEqual((pPause.fields.mirror!.value as { rows: { key: string }[] }).rows.map(r => r.key), ['GER', 'FRA'], 'pause immediately flushes pending mirror revision');
    assert.ok(wakes >= 0);

    const audit1 = paintAt(rowsA, 2300, 0, null, null, { audit: true, auditId: 7 });
    const audit2 = paintAt(rowsA, 2310, 0, null, null, { audit: true, auditId: 8 });
    assert.equal((audit1.fields.audit!.value as { id: number }).id, 7, 'audit carries snapshot id 7');
    assert.equal((audit2.fields.audit!.value as { id: number }).id, 8, 'audit carries snapshot id 8 when snapshot changes');

    const cols: TableColumnSpec[] = [{ key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true }];
    const hitSettings = tableSettingsOf(undefined, cols, { density: 'comfortable', sort: { column: 'name', direction: 'ascending' } });
    const hitLayout = layoutFamilyTable(buildColumnLayouts(cols, hitSettings, { name: 160 }), 320);
    const hitRows = resolveTableRows({
      columns: cols,
      hour: 1,
      rows: [
        { key: 'A', name: 'Alpha', cells: { name: { value: knownValue('Alpha') } } },
        { key: 'B', name: 'Bravo', cells: { name: { value: knownValue('Bravo') } } },
      ],
    }, hitSettings);
    const glide = new Map([
      ['A', { from: 34, to: 0, started: 100 }],
      ['B', { from: -34, to: 0, started: 100 }],
    ]);
    const hitTop = hitFamilyTable(hitLayout, hitRows, { x: 20, y: 10 }, 34, 0, 0, false, 0, tableGlide(glide, hitRows, new Map([['A', 0], ['B', 1]]), null, 34), 100, 180);
    assert.equal(hitTop.rowKey, 'B', 'clicking y=10 during glide hits B which is painted at y=0, not destination A');

    const branch = { firstHour: 10, lastHour: 500 };
    assert.deepEqual(resolveHeadSpan('trailing-window', 168, 'campaign', branch, { firstHour: 10, lastHour: 500 }, 500), { first: 10, last: 500 }, 'campaign mode keeps whole branch span');
    assert.deepEqual(resolveHeadSpan('trailing-window', 168, 'range', branch, { firstHour: 50, lastHour: 120 }, 120), { first: 50, last: 120 }, 'range mode keeps explicit range');
    assert.deepEqual(resolveHeadSpan('trailing-window', 168, 'follow', branch, { firstHour: 10, lastHour: 500 }, 300), { first: 133, last: 300 }, 'follow mode uses trailing window');
    assert.deepEqual(resolveHeadSpan('branch-to-hour', 168, 'pin', branch, { firstHour: 10, lastHour: 240 }, 240), { first: 10, last: 240 }, 'branch-to-hour pin runs from branch start to pinned hour');
    assert.deepEqual(resolveHeadSpan('branch-to-hour', 168, 'range', branch, { firstHour: 40, lastHour: 240 }, 240), { first: 40, last: 240 }, 'branch-to-hour range keeps explicit range');

    const { ctx } = instrumentedContext();
    const numCols: TableColumnSpec[] = [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true },
      { key: 'v', label: 'Val', as: 'faint', tone: 'blue', offered: ['faint'], priority: 1, numeric: true, format: 'integer' },
    ];
    const numSettings = tableSettingsOf(undefined, numCols, { density: 'comfortable', sort: { column: 'v', direction: 'descending' } });
    const numLayout = layoutFamilyTable(buildColumnLayouts(numCols, numSettings, { name: 120, v: 120 }), 320);
    const numRows = resolveTableRows({
      columns: numCols,
      hour: 1,
      rows: [
        { key: 'A', name: 'A', cells: { name: { value: knownValue('A') }, v: { value: knownValue(100), format: 'integer' } } },
        { key: 'B', name: 'B', cells: { name: { value: knownValue('B') }, v: { value: knownValue(50), format: 'integer' } } },
      ],
    }, numSettings);
    const pic = createTablePicture();
    drawFamilyTable(ctx, { layout: numLayout, rows: numRows, settings: numSettings, hour: 1, width: 320, height: 200, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 1, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, dpr: 1 }, pic);
    const firstMaxima = pic.maxima;
    drawFamilyTable(ctx, { layout: numLayout, rows: numRows, settings: numSettings, hour: 1, width: 320, height: 200, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 2, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), inspectedKey: 'A', selectedKey: null, focusedKey: null, dpr: 1 }, pic);
    assert.equal(pic.maxima, firstMaxima, 'hover repaint reuses cached column maxima without scanning all rows');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }
});

test('round 2 regressions: canonical audit, snapshot store, retained static span, cadence trailing flush, density invalidation, off-screen focus index, long-distance glide hit/draw, dated country colours, missing occupation law, and meta/priority invalidation', () => {
  const cols: TableColumnSpec[] = [
    { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true },
    { key: 'rate', label: 'Rate', as: 'faint', tone: 'country', offered: ['faint'], priority: 1, numeric: true, format: 'decimal1' },
  ];
  const settings = tableSettingsOf(undefined, cols, { density: 'comfortable', sort: { column: 'rate', direction: 'descending' } });
  const mkModel = (rate: number, groupLabel: string) => ({
    columns: cols,
    hour: 10,
    rank: true,
    rows: [
      { key: 'A', name: 'A', group: { key: 'g1', label: groupLabel, order: 0 }, cells: { name: { value: knownValue('A') }, rate: { value: rate === 0 ? zeroValue(0) : knownValue(rate), format: 'decimal1' as const, country: 'GER' } } },
    ],
  });
  const colLayouts = buildColumnLayouts(cols, settings);
  const mZero = mkModel(0, 'Axis');
  const mTiny = mkModel(0.04, 'Axis');
  const mRenamed = mkModel(0.04, 'New Axis');
  const auditZero = deriveTableAudit({ model: mZero, flatRows: resolveTableRows(mZero, settings), visibleColumns: colLayouts, rankShown: true });
  const auditTiny = deriveTableAudit({ model: mTiny, flatRows: resolveTableRows(mTiny, settings), visibleColumns: colLayouts, rankShown: true });
  assert.notEqual(
    pictureError('table', auditZero, { values: new Map(auditTiny.map(r => [r.key, r.value!])), ordered: true }),
    null,
    'canonical audit distinguishes 0 from 0.04 despite identical display rounding',
  );
  const auditRenamedGroup = deriveTableAudit({ model: mRenamed, flatRows: resolveTableRows(mRenamed, settings), visibleColumns: colLayouts, rankShown: true });
  assert.notEqual(
    pictureError('table', auditTiny, { values: new Map(auditRenamedGroup.map(r => [r.key, r.value!])), ordered: true }),
    null,
    'canonical audit catches group header changes',
  );
  const auditOneCol = deriveTableAudit({ model: mTiny, flatRows: resolveTableRows(mTiny, settings), visibleColumns: [colLayouts[0]!], rankShown: true });
  assert.notEqual(
    pictureError('table', auditTiny, { values: new Map(auditOneCol.map(r => [r.key, r.value!])), ordered: true }),
    null,
    'canonical audit catches visible column set changes',
  );

  const store = new AuditSnapshotStore<{ tag: string }>(2);
  const id1 = store.create({ tag: 'first' });
  const id2 = store.create({ tag: 'second' });
  assert.equal(store.require(id1).tag, 'first');
  assert.equal(store.require(id2).tag, 'second');
  assert.throws(() => store.require(undefined), /Not auditable/);
  assert.throws(() => store.require(999), /Not auditable/);

  assert.deepEqual(effectiveCountryUnobserved({ kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false }, [{ firstHour: 10, lastHour: 20 }]), []);
  assert.deepEqual(effectiveCountryUnobserved({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, [{ firstHour: 10, lastHour: 20 }]), [[10, 20]]);

  const gate = new CadenceGate(20);
  assert.equal(gate.due(100, 24, 'w'), true);
  gate.record(100, 'w');
  gate.invalidate();
  assert.equal(gate.due(110, 24, 'w'), false);
  let scheduledMs = -1;
  let scheduledCb: (() => void) | null = null;
  let flushed = false;
  const flusher = new TrailingFlushScheduler([gate], {
    set: (fn, ms) => {
      scheduledCb = fn;
      scheduledMs = ms;
      return 1 as unknown as ReturnType<typeof setTimeout>;
    },
    clear: () => { scheduledCb = null; },
  });
  flusher.sync(110, 24, () => { flushed = true; });
  assert.equal(scheduledMs, 10, 'schedules trailing flush for remaining interval');
  assert.ok(scheduledCb !== null);
  (scheduledCb as () => void)();
  assert.equal(flushed, true, 'trailing flush callback fires cleanly');
  assert.equal(gate.due(115, 0, 'w'), true, 'pause flushes immediately');
  flusher.cancel();

  const occMissingLaw = occupiedStatesProjection.project(createBoundedReadView({
    hour: 50,
    panels: new Map([['main', {
      states: [{ state: 1, name: 'Warsaw', nameMissing: null, owner: 'POL', occupier: 'GER', occupied: 'POL', garrisonRequiredSign: 0, garrisonAlert: false, assignedKnown: true, garrisonMissing: null, law: 'custom_law', lawMissing: null }],
      complete: true,
      missing: null,
      laws: [{ token: 'custom_law', name: null, icon: null, missing: 'Law localization absent' }],
      strengthMissing: 'Missing strength',
      garrisonRedRaw: null,
      warnings: [],
    }]]),
    live: new Map([['main', { get: () => 0, missing: () => 'Not recorded' }]]),
  }), { filter: { passing: null, unrecorded: [], bounds: [] }, role: 'occupier', lawAs: 'dot', filterAll: true });
  assert.deepEqual(occMissingLaw.rows[0]!.cells.law!.value, { kind: 'notRecorded', reason: 'Law localization absent' });

  const manyRows = Array.from({ length: 15 }, (_, i) => ({
    key: `R${i}`,
    name: `Row ${i}`,
    cells: { name: { value: knownValue(`Row ${i}`) }, rate: { value: knownValue(15 - i), format: 'decimal1' as const, country: 'GER' } },
  }));
  const flatMany = resolveTableRows({ columns: cols, hour: 10, rows: manyRows }, settings);
  const mirror = deriveTableMirror(flatMany, cols, false, { first: 5, count: 3 }, 'R0');
  assert.equal(mirror.rows[0]!.key, 'R0');
  assert.equal(mirror.rows[0]!.rowIndex, 0, 'off-screen focused row carries its true rowIndex, not window offset');

  const layout = layoutFamilyTable(buildColumnLayouts(cols, settings, { name: 140, rate: 140 }), 320);
  const longGlide = new Map([['R12', { from: -12 * 28, to: 0, started: 100 }]]);
  const hitGliding = hitFamilyTable(layout, flatMany, { x: 20, y: 10 }, 28, 0, 0, false, 0, tableGlide(longGlide, flatMany, new Map(flatMany.flatMap((item, index) => (item.kind === 'data' ? [[item.key, index] as const] : []))), null, 28), 100, 180);
  assert.equal(hitGliding.rowKey, 'R12', 'row gliding from index 12 into y=0 is hit-tested accurately');

  const { ctx } = instrumentedContext();
  const pic = createTablePicture();
  const fills: string[] = [];
  const origFillRect = ctx.fillRect.bind(ctx);
  const origFill = ctx.fill.bind(ctx);
  (ctx as unknown as { fillRect: (...a: unknown[]) => void; fill: (...a: unknown[]) => void }).fillRect = (...a: unknown[]) => {
    fills.push(String(ctx.fillStyle));
    (origFillRect as (...b: unknown[]) => void)(...a);
  };
  (ctx as unknown as { fill: (...a: unknown[]) => void }).fill = (...a: unknown[]) => {
    fills.push(String(ctx.fillStyle));
    (origFill as (...b: unknown[]) => void)(...a);
  };
  const tinted = (hour: number, colour: string) => resolveTableRows({ columns: cols, hour, rows: manyRows.slice(0, 2).map(row => ({ ...row, cells: { ...row.cells, rate: { ...row.cells.rate, countryColor: colour } } })) }, settings);
  drawFamilyTable(ctx, { layout, rows: tinted(10, 'rgb(10, 20, 30)'), settings, hour: 10, width: 320, height: 200, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 1, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), dpr: 1 }, pic);
  assert.ok(fills.includes('rgb(10, 20, 30)'), 'paints the country colour its cell carries at hour 10');
  const compactSettings = { ...settings, density: 'compact' as const };
  const afterDensity = drawFamilyTable(ctx, { layout, rows: tinted(25, 'rgb(200, 10, 10)'), settings: compactSettings, hour: 25, width: 320, height: 200, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 2, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), dpr: 1 }, pic);
  assert.equal(afterDensity.drew, true, 'density change invalidates frameKey and repaints');
  assert.ok(fills.includes('rgb(200, 10, 10)'), 'paints the colour a later projection gives the cell at hour 25');

  const metaProj = declareTableProjection<{ designer: string; priority: number }, { id: string; designer: string }, Record<string, never>>({
    kind: 'test-meta-priority-inv',
    columns: reply => [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, path: 'id' },
      { key: 'v', label: 'Val', as: 'plain', offered: ['plain'], priority: reply.priority, numeric: true, value: () => knownValue(1) },
    ],
    rows: reply => ({ rows: [{ id: 'A', designer: reply.designer }] }),
    rowKey: r => r.id,
    rowName: r => r.id,
    rowMeta: r => ({ designer: r.designer }),
  });
  registerFamilyProjection(metaProj);
  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return { ...instrumentedContext().ctx, setTransform() {} }; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const laneView = createLaneView(tableLane, () => {});
    const laneState = new Map<string, unknown>();
    const transport = new LaneTransport();
    let seq = 1;
    const mk = (designer: string, priority: number) => laneView.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: transport.carry({
        family: 'table', theme: TEST_THEME,
        kind: 'test-meta-priority-inv',
        options: NO_OPTIONS,
        settings: tableSettingsOf(undefined, [
          { key: 'name', label: 'Name', as: 'plain', offered: ['plain'] },
          { key: 'v', label: 'Val', as: 'plain', offered: ['plain'] },
        ], { density: 'comfortable', sort: { column: 'name', direction: 'ascending' } }),
        sources: NO_SOURCES,
        staticPanels: { main: { designer, priority } },
        width: 320,
        height: 200,
        scrollLeft: 0,
        scrollTop: 0,
      }),
      hour: 10,
      seek: 0, speed: 0,
      binding: { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 100,
    }, laneState, () => `c${seq++}`);
    const firstPaint = mk('ordinary', 1);
    const secondPaint = mk('headquarters', 4);
    assert.equal(firstPaint.mode, 'picture', 'N4: tableLane returns explicit mode picture');
    assert.notEqual(firstPaint.fields.nav!.content, secondPaint.fields.nav!.content, 'row.meta change invalidates nav field');
    assert.notEqual(firstPaint.fields.meta!.content, secondPaint.fields.meta!.content, 'column priority change invalidates meta field');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }
});

test('round 3 regressions: M1 typed scalars, N1 by-construction audit (bar/alert/flag/missing name/pill/tooltip), N6 native slot holder tie-break, M4 ever-gap refusal, S1 frozen static snapshot, M5 dateless game-score empty, and N18 frozen change caches', () => {
  assert.notEqual(canonicalAuditValue(knownValue('1000')), canonicalAuditValue(knownValue(1000)), 'M1: string "1000" and number 1000 do not collide');
  assert.equal(canonicalAuditValue(knownValue(1e21)), canonicalAuditValue(knownValue(10n ** 21n)), 'M1: 1e21 and 10n**21n serialize identically');
  assert.equal(canonicalAuditValue(knownValue(-0)), canonicalAuditValue(knownValue(0)));
  assert.notEqual(canonicalAuditData({ a: '1000' }), canonicalAuditData({ a: 1000 }));
  assert.equal(canonicalAuditData({ b: 1, a: 2 }), canonicalAuditData({ a: 2, b: 1 }), 'canonicalAuditData sorts object keys deterministically');

  const occCols: TableColumnSpec[] = [
    { key: 'country', label: 'Country', as: 'flagName', offered: ['flagName'], priority: 0, text: true },
    { key: 'garrison', label: 'Garrison', as: 'meterBefore', tone: 'blue', offered: ['meterBefore'], priority: 1, numeric: true, format: 'integer' },
  ];
  const occSettings = tableSettingsOf(undefined, occCols, { density: 'comfortable', sort: { column: 'garrison', direction: 'descending' } });
  const occLayouts = buildColumnLayouts(occCols, occSettings);
  const mkOcc = (opts: { fraction: number; alert: boolean; flag: string; missingName: boolean; pill: string; extra: string }) => {
    const model = {
      columns: occCols,
      hour: 50,
      rank: true,
      titleSummary: opts.pill,
      rows: [{
        key: 'S1',
        name: opts.missingName ? 'Name not recorded' : 'Germany',
        cells: {
          country: {
            value: opts.missingName ? notRecorded('Name absent') : knownValue('Germany'),
            iconSlot: true,
            country: 'GER',
            art: readyArt(opts.flag),
            artKey: opts.flag,
          },
          garrison: {
            value: knownValue(12),
            format: 'integer' as const,
            bar: { fraction: knownValue(opts.fraction), alert: opts.alert },
          },
        },
        tooltipExtra: [{ label: 'Equipment', value: knownValue(opts.extra) }],
      }],
    };
    return deriveTableAudit({ model, flatRows: resolveTableRows(model, occSettings), visibleColumns: occLayouts, rankShown: true });
  };
  const baseAudit = mkOcc({ fraction: 0.8, alert: false, flag: 'ger.png', missingName: false, pill: '1 state', extra: '80%' });
  const diff = (other: ReturnType<typeof mkOcc>) => pictureError('table', baseAudit, { values: new Map(other.map(r => [r.key, r.value!])), ordered: true });
  assert.notEqual(diff(mkOcc({ fraction: 0.4, alert: false, flag: 'ger.png', missingName: false, pill: '1 state', extra: '80%' })), null, 'N1: catches garrison bar fraction change');
  assert.notEqual(diff(mkOcc({ fraction: 0.8, alert: true, flag: 'ger.png', missingName: false, pill: '1 state', extra: '80%' })), null, 'N1: catches garrison alert flag change');
  assert.notEqual(diff(mkOcc({ fraction: 0.8, alert: false, flag: 'pol.png', missingName: false, pill: '1 state', extra: '80%' })), null, 'N1: catches flag identity change');
  assert.notEqual(diff(mkOcc({ fraction: 0.8, alert: false, flag: 'ger.png', missingName: true, pill: '1 state', extra: '80%' })), null, 'N1: catches a missing name');
  assert.notEqual(diff(mkOcc({ fraction: 0.8, alert: false, flag: 'ger.png', missingName: false, pill: '2 states', extra: '80%' })), null, 'N1: catches titleSummary change');
  assert.notEqual(diff(mkOcc({ fraction: 0.8, alert: false, flag: 'ger.png', missingName: false, pill: '1 state', extra: '40%' })), null, 'N1: catches tooltipExtra change');

  const slotRuns = resolveSlotHolderRuns([
    { tag: 'Z', firstHour: 0, lastHour: 20, countryId: 5 },
    { tag: 'A', firstHour: 10, lastHour: 20, countryId: 5 },
  ]);
  assert.deepEqual(slotRuns, [{ tag: 'Z', firstHour: 0, lastHour: 20, countryId: 5 }], 'N6: equal countryId breaks tie by earlier firstHour before tag');
  assert.equal(holderRunAt([
    { tag: 'Z', firstHour: 0, lastHour: 20, countryId: 5 },
    { tag: 'A', firstHour: 10, lastHour: 20, countryId: 5 },
  ], 10)?.tag, 'Z', 'N6: unsorted holderRunAt uses identical (countryId, firstHour, tag) precedence');
  const nativeCase = resolveSlotHolderRuns([
    { tag: 'GER', firstHour: 1, lastHour: 10, countryId: 10 },
    { tag: 'PRU', firstHour: 5, lastHour: 15, countryId: 10 },
    { tag: 'D01', firstHour: 6, lastHour: 8, countryId: 9 },
  ]);
  assert.deepEqual(nativeCase.map(r => [r.tag, r.firstHour, r.lastHour]), [['GER', 1, 5], ['D01', 6, 8], ['GER', 9, 10], ['PRU', 11, 15]]);

  assert.equal(
    selectionUnobservedReason({ kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false }, [{ firstHour: 10, lastHour: 20 }]),
    'Group membership is incomplete for this campaign.',
    'M4: membership ever refuses when unobserved gaps make campaign membership incomplete',
  );
  assert.equal(selectionUnobservedReason({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, [{ firstHour: 10, lastHour: 20 }]), null);

  const ctxA = { tag: 'A', role: 'occupier', settings: 'sA', looks: 'lA' };
  const ctxB = { tag: 'B', role: 'occupied', settings: 'sB', looks: 'lB' };
  const bundleAt50 = { span: [10, 50] };
  const bundleAt60 = { span: [20, 60] };
  const first = showBundle(null, bundleAt50, false, 'plan', ctxA);
  const refreshed = showBundle(first.last, bundleAt50, false, 'plan', ctxB);
  assert.equal(refreshed.shown?.frozen, ctxB, 'F1: when every reply answers the current requests, option, role, settings and looks changes refresh immediately');
  const retained = showBundle(refreshed.last, bundleAt50, true, 'plan', ctxA);
  assert.equal(retained.shown, refreshed.shown, 'S1: while a request is superseded, the last complete bundle is shown with its own context, settings and looks');
  assert.equal(retained.shown?.bundle, bundleAt50);
  const updated = showBundle(retained.last, bundleAt60, false, 'plan', ctxA);
  assert.equal(updated.shown?.bundle, bundleAt60, 'S1: updates once the new replies complete a bundle');
  assert.equal(updated.shown?.frozen, ctxA, 'S1: the updated bundle carries the new selection context');

  const emptyGsA = gameScoreProjection.project(createBoundedReadView({
    hour: 10,
    panels: new Map([['main', { missing: null, warnings: [], hints: {}, multipliers: [], rows: [] }]]),
    live: new Map([['main', { get: () => undefined, missing: () => 'Not recorded' }]]),
  }), { all: true, membership: null, top: null, by: 'total', factions: false, factionRuns: null, rankMarkers: false });
  const emptyGsB = gameScoreProjection.project(createBoundedReadView({
    hour: 250,
    panels: new Map([['main', { missing: null, warnings: [], hints: {}, multipliers: [], rows: [] }]]),
    live: new Map([['main', { get: () => undefined, missing: () => 'Not recorded' }]]),
  }), { all: true, membership: null, top: null, by: 'total', factions: false, factionRuns: null, rankMarkers: false });
  assert.equal(emptyGsA.empty, 'No country’s scores recorded at this time.', 'M5: dateless empty message');
  assert.equal(emptyGsA.empty, emptyGsB.empty, 'M5: empty message does not depend on advancing hour');

  const winRanges: [number, number][] = [[10, 20], [50, 60]];
  assert.equal(nextWindowRangesChange(winRanges, 168, 5), 10);
  assert.equal(nextWindowRangesChange(winRanges, 168, 15), 50);
  assert.equal(nextWindowRangesChange(winRanges, 168, 55), 188);
  assert.equal(nextWindowRangesChange(winRanges, 168, 190), 228);
  assert.throws(() => { winRanges.push([100, 110]); }, TypeError, 'N18: indexed ranges are frozen against in-place mutation');
  const mutHolders: Record<string, { tag: string; firstHour: number; lastHour: number }[]> = { GER: [{ tag: 'GER', firstHour: 0, lastHour: 10 }] };
  assert.equal(nextHoldersChange(mutHolders, 5), 11);
  assert.throws(() => { mutHolders.GER![0]!.lastHour = 99; }, TypeError, 'N18: indexed holder runs are frozen against in-place mutation');
  const mutRanges: [number, number][] = [[1, 5]];
  assert.equal(nextRangesChange(mutRanges, 2), 6);
  assert.throws(() => { mutRanges[0]![1] = 20; }, TypeError, 'N18: indexed range tuples are frozen against in-place mutation');
});

test('round 4 regressions: P1/N1 country colour & dateless tooltip audit, P2 whole-panel refusal, P5 capture-gated audit serialization, N6 UTF-8 tag tie-break, N18 deep tuple freeze & indexed overlap, M4 filter ever-gap refusal, and M9 focused index', async () => {
  const cols: TableColumnSpec[] = [
    { key: 'country', label: 'Country', as: 'flagName', offered: ['flagName'], priority: 0, text: true },
    { key: 'score', label: 'Score', as: 'faint', tone: 'country', offered: ['faint'], priority: 1, numeric: true, format: 'integer' },
  ];
  const settings = tableSettingsOf(undefined, cols, { density: 'comfortable', sort: { column: 'score', direction: 'descending' } });
  const layouts = buildColumnLayouts(cols, settings);
  const mkModel = (hour: number, colour = 'rgb(10, 20, 30)') => ({
    columns: cols,
    hour,
    rank: true,
    rows: [{
      key: 'GER',
      name: 'Germany',
      cells: {
        country: { value: knownValue('Germany'), iconSlot: true, country: 'GER' },
        score: { value: knownValue(100), format: 'integer' as const, country: 'GER', countryColor: colour },
      },
    }],
  });
  const m10 = mkModel(10);
  const m12 = mkModel(12);
  const flat10 = resolveTableRows(m10, settings);
  const flat12 = resolveTableRows(m12, settings);
  const audit10 = deriveTableAudit({ model: m10, flatRows: flat10, visibleColumns: layouts, rankShown: true });
  const audit12 = deriveTableAudit({ model: m12, flatRows: flat12, visibleColumns: layouts, rankShown: true });
  assert.equal(
    pictureError('table', audit10, { values: new Map(audit12.map(r => [r.key, r.value!])), ordered: true }),
    null,
    'P1: tooltip audit omits presentation hour so valid picture at hour 10 matches exact audit at hour 12',
  );
  const tip10 = deriveTableTooltip(flat10, 'GER', cols)!;
  assert.equal(tip10.subtitle, undefined, 'P1: deriveRowTooltip carries no hour-derived subtitle');
  const recoloured = mkModel(10, 'rgb(200, 10, 10)');
  const auditColorDiff = deriveTableAudit({ model: recoloured, flatRows: resolveTableRows(recoloured, settings), visibleColumns: layouts, rankShown: true });
  assert.notEqual(
    pictureError('table', audit10, { values: new Map(auditColorDiff.map(r => [r.key, r.value!])), ordered: true }),
    null,
    'N1/P1: resolved country colour change is caught by CellPaintModel audit',
  );
  const paintModel = resolveCellPaintModel(m10.rows[0]!.cells.score, layouts[1]!, new Map([['score', 100]]));
  assert.equal(paintModel.countryColor, 'rgb(10, 20, 30)', 'P1: CellPaintModel carries resolved countryColor for painter');

  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return { ...instrumentedContext().ctx, setTransform() {} }; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const laneView = createLaneView(tableLane, () => {});
    const laneState = new Map<string, unknown>();
    const transport = new LaneTransport();
    let seq = 1;
    const refusedPaint = laneView.paint({
      widget: 1,
      subs: [{ id: 7, channel: 'panel' }],
      pictures: [{ parts: [{ value: { kind: 'panel', data: null, refused: 'Game score panel refused by recording' } }] }],
      model: transport.carry({
        family: 'table', theme: TEST_THEME,
        kind: 'game-score',
        view: 'table',
        options: { all: true, membership: null, top: null, by: 'total', factions: false, factionRuns: null, rankMarkers: false },
        settings: tableSettingsOf(undefined, gameScoreColumns(), { density: 'comfortable', sort: { column: 'total', direction: 'descending' } }),
        sources: { main: 7 },
        width: 320,
        height: 200,
        scrollLeft: 0,
        scrollTop: 0,
      }),
      hour: 10,
      seek: 0, speed: 0,
      binding: { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 100,
    } as never, laneState, () => `r${seq++}`);
    assert.equal(refusedPaint.mode, 'refused', 'P2: whole-panel refusal produces mode refused, never empty mode picture');
    assert.equal(refusedPaint.reason, 'Game score panel refused by recording');

    const auditState = new Map<string, unknown>();
    const paintNoAudit = laneView.paint({
      widget: 2,
      subs: [],
      pictures: [],
      model: transport.carry({
        family: 'table', theme: TEST_THEME,
        kind: 'game-score',
        view: 'table',
        options: { all: true, membership: null, top: null, by: 'total', factions: false, factionRuns: null, rankMarkers: false },
        settings: tableSettingsOf(undefined, gameScoreColumns(), { density: 'comfortable', sort: { column: 'total', direction: 'descending' } }),
        sources: NO_SOURCES,
        staticPanels: { main: { missing: null, warnings: [], hints: {}, multipliers: [], rows: [{ tag: 'GER', total: { missing: null }, army: { missing: null }, navy: { missing: null }, air: { missing: null }, industry: { missing: null }, land: { missing: null } }] } },
        width: 320,
        height: 200,
        scrollLeft: 0,
        scrollTop: 0,
      }),
      hour: 10,
      seek: 0,
      speed: 24,
      binding: { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 100,
    }, auditState, () => `a${seq++}`);
    assert.equal(paintNoAudit.fields.audit, undefined, 'P5: zero audit serialization when capture is inactive');
    assert.equal(paintNoAudit.mode, 'picture', 'P2: populated table returns explicit mode picture');

    const seen: boolean[] = [];
    const unsub = subscribeFamilyAuditing(() => { seen.push(isFamilyAuditing()); });
    try {
      assert.equal(isFamilyAuditing(), false);
      await capturePictures(true);
      assert.equal(isFamilyAuditing(), true);
      await capturePictures(false);
      assert.equal(isFamilyAuditing(), false);
      assert.deepEqual(seen, [true, false], 'P5: shared family auditing hook notifies on capture start and end');
    } finally {
      unsub();
      await capturePicturesBatch(100);
    }

    const paintWithAudit = laneView.paint({
      widget: 2,
      subs: [],
      pictures: [],
      model: transport.carry({
        family: 'table', theme: TEST_THEME,
        kind: 'game-score',
        view: 'table',
        options: { all: true, membership: null, top: null, by: 'total', factions: false, factionRuns: null, rankMarkers: false },
        settings: tableSettingsOf(undefined, gameScoreColumns(), { density: 'comfortable', sort: { column: 'total', direction: 'descending' } }),
        sources: NO_SOURCES,
        staticPanels: { main: { missing: null, warnings: [], hints: {}, multipliers: [], rows: [{ tag: 'GER', total: { missing: null }, army: { missing: null }, navy: { missing: null }, air: { missing: null }, industry: { missing: null }, land: { missing: null } }] } },
        width: 320,
        height: 200,
        scrollLeft: 0,
        scrollTop: 0,
        audit: true,
        auditId: 42,
      }),
      hour: 10,
      seek: 0,
      speed: 24,
      binding: { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 100,
    }, auditState, () => `a${seq++}`);
    assert.equal((paintWithAudit.fields.audit?.value as { id?: number } | undefined)?.id, 42, 'P5: serializes audit once capture activates');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }

  const utf8Runs = [
    { tag: '\u{10000}', firstHour: 0, lastHour: 10, countryId: 1 },
    { tag: '\uE000', firstHour: 0, lastHour: 10, countryId: 1 },
  ];
  assert.equal(resolveSlotHolderRuns(utf8Runs)[0]!.tag, '\uE000', 'N6: U+E000 precedes U+10000 in UTF-8 byte order despite higher UTF-16 code unit');
  assert.equal(holderRunAt(utf8Runs, 5)?.tag, '\uE000', 'N6: holderRunAt uses native UTF-8 order');
  const slots = new SlotTags({
    countries: [
      { tag: '\u{10000}', identities: [{ slot: 1, countryId: 1, firstHour: 0, lastHour: 10 }], unresolved: [] },
      { tag: '\uE000', identities: [{ slot: 1, countryId: 1, firstHour: 0, lastHour: 10 }], unresolved: [] },
    ],
  });
  assert.equal(slots.tagAt(1, 5), '\uE000', 'N6: SlotTags.tagAt uses native UTF-8 order');

  const shallowFrozenOuter = Object.freeze([[10, 20] as [number, number], [40, 50] as [number, number]]);
  assert.equal(overlapsHourRanges(shallowFrozenOuter, 15, 25), true, 'N18: indexed overlap finds intersecting interval');
  assert.equal(overlapsHourRanges(shallowFrozenOuter, 21, 39), false, 'N18: indexed overlap skips disjoint interval');
  assert.equal(overlapsHourRanges(shallowFrozenOuter, 50, 60), true, 'N18: indexed overlap includes boundary');
  assert.throws(() => { (shallowFrozenOuter[0] as [number, number])[1] = 999; }, TypeError, 'N18: inner tuple frozen even when outer array was already frozen');

  const filterGroupUnobserved = groupUnobserved(
    {
      humans: [{ tag: 'GER', firstHour: 0, lastHour: 9 }],
      factions: [],
      wars: [],
      unobserved: { humans: [{ firstHour: 10, lastHour: 20 }], factions: [], wars: [] },
      countryIdentities: [],
      warnings: [],
    } as never,
    { type: 'humans' },
  );
  assert.equal(
    selectionUnobservedReason({ kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false }, filterGroupUnobserved),
    'Group membership is incomplete for this campaign.',
    'M4: filter policy applies ever-membership incompleteness refusal when groupUnobserved has gaps',
  );

  const mirrorRows = Array.from({ length: 10 }, (_, i) => ({
    key: `R${i}`,
    name: `Row ${i}`,
    cells: { country: { value: knownValue(`Row ${i}`) }, score: { value: knownValue(10 - i) } },
  }));
  const flatMirror = resolveTableRows({ columns: cols, hour: 10, rows: mirrorRows }, settings);
  const mirrorWithIdx = deriveTableMirror(flatMirror, cols, false, { first: 5, count: 2 }, 'R1', 1);
  assert.equal(mirrorWithIdx.rows[0]!.key, 'R1', 'M9: deriveTableMirror uses supplied O(1) focusedIndex');
  assert.equal(mirrorWithIdx.rows[0]!.rowIndex, 1);
});

test('round 5 regressions: F5 revealAt & child revealAt lane validity, F6 1500-country/20-row viewport art loader & unmount cleanup, F7 local vs stream failure origin, and F8 typed tooltip ValueState & audit', async () => {
  const { Lane } = await import('../../draw/lane.ts');
  const { Page } = await import('../../dashboard/follow/page.ts');
  const { tooltipText } = await import('../../charts/tooltip.ts');

  const cols: TableColumnSpec[] = [
    { key: 'name', label: 'Name', as: 'flagName', offered: ['flagName'], priority: 0, text: true },
    { key: 'val', label: 'Value', as: 'plain', offered: ['plain'], priority: 1, numeric: true, format: 'decimal1' },
  ];
  const settings = tableSettingsOf(undefined, cols, { density: 'comfortable', sort: { column: 'name', direction: 'ascending' } });

  registerFamilyProjection({
    kind: 'test-r5-reveal',
    family: 'table', theme: TEST_THEME,
    project(readView) {
      return {
        columns: cols,
        hour: readView.hour,
        unsorted: true,
        rows: [
          {
            key: 'parent',
            name: 'Parent',
            cells: { name: { value: knownValue('Parent') }, val: { value: knownValue(1) } },
            initiallyOpen: true,
            children: [
              { key: 'child-late', name: 'Child Late', revealAt: 25, cells: { name: { value: knownValue('Child Late') }, val: { value: knownValue(2) } } },
            ],
          },
          { key: 'row-late', name: 'Row Late', revealAt: 40, cells: { name: { value: knownValue('Row Late') }, val: { value: knownValue(3) } } },
        ],
      };
    },
  });

  registerFamilyProjection({
    kind: 'test-r5-art-1500',
    family: 'table', theme: TEST_THEME,
    project(readView) {
      return {
        columns: cols,
        hour: readView.hour,
        unsorted: true,
        ...(readView.hour < 200 ? { nextChangeAt: readView.hour < 100 ? 100 : 200 } : {}),
        rows: Array.from({ length: 1500 }, (_, i) => {
          const tag = `C${String(i).padStart(4, '0')}`;
          return {
            key: tag,
            name: tag,
            cells: {
              name: {
                value: knownValue(tag),
                iconSlot: true,
                country: tag,
                art: readView.art({ kind: 'url', url: `/flags/${tag}_${readView.hour < 100 ? 'early' : readView.hour < 200 ? 'mid' : 'late'}.png` }),
              },
              val: { value: knownValue(i + 1) },
            },
          };
        }),
      };
    },
  });

  let throwLocalPaint = false;
  registerFamilyProjection({
    kind: 'test-r5-local-fail',
    family: 'table', theme: TEST_THEME,
    project(readView) {
      if (throwLocalPaint) throw new Error('local projection crash');
      return {
        columns: cols,
        hour: readView.hour,
        rows: [{ key: 'ok', name: 'Ok', cells: { name: { value: knownValue('Ok') }, val: { value: knownValue(7) } } }],
      };
    },
  });

  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return { ...instrumentedContext().ctx, setTransform() {} }; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const presented: { widget: number; hour: number; navCount: number; mode: string; failureOrigin?: string }[] = [];
    const requestedArt: string[] = [];
    const abortedArt: string[] = [];
    const closedArt: string[] = [];
    const artResolvers = new Map<string, (bmp: ImageBitmap) => void>();

    const tableView = createLaneView(tableLane, () => {}, {
      artMaxEntries: 40,
      fetchBitmap: (url, signal) => new Promise<ImageBitmap>((resolve, reject) => {
        requestedArt.push(url);
        signal.addEventListener('abort', () => {
          abortedArt.push(url);
          reject(new DOMException('Aborted', 'AbortError'));
        }, { once: true });
        artResolvers.set(url, resolve);
      }),
    });

    const page = new Page<{ width: number; close(): void }>({ lanes: 1, post: (_lane, msg) => { if (msg.type === 'frame') lane.frame(msg); } });
    const lane = new Lane<{ width: number; close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => tableView,
      crop: async sources => sources.map(s => ({ width: s.width, close() {} })),
      present: presentation => {
        for (const entry of presentation.entries) {
          const nav = entry.fields.nav as readonly unknown[] | undefined;
          presented.push({ widget: entry.widget, hour: entry.lo, navCount: nav ? nav.length : -1, mode: entry.mode, failureOrigin: entry.failureOrigin });
        }
        page.offer(presentation);
        page.frame();
      },
      release: () => {},
      fail: () => {},
      later: run => queueMicrotask(run),
    });

    const flushTicks = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

    const sender = new PayloadSender(message => lane.payloads(structuredClone(message)));
    const wired = (model: unknown) => structuredClone(sender.acquire(model).wire);
    page.mount(1, { route: 0, lane: 0, surfaces: [{ install() {}, clear() {} }] });
    lane.mount(1, [], 'table');
    lane.bind(1, { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 10, c: 100 } }, wired({
      family: 'table', theme: TEST_THEME, kind: 'test-r5-reveal', options: NO_OPTIONS, settings, sources: NO_SOURCES, width: 320, height: 200, scrollLeft: 0, scrollTop: 0,
    }));
    page.publish(10, { playing: false });
    await flushTicks();
    assert.equal((page.view(1).fields?.nav as readonly unknown[]).length, 1, 'F5: at hour 10 only parent is visible');
    page.publish(25, { playing: false });
    await flushTicks();
    assert.equal((page.view(1).fields?.nav as readonly unknown[]).length, 2, 'F5: crossing child revealAt=25 invalidates lane window and reveals child row');
    page.publish(40, { playing: false });
    await flushTicks();
    assert.equal((page.view(1).fields?.nav as readonly unknown[]).length, 3, 'F5: crossing row revealAt=40 invalidates lane window and reveals top-level row');

    page.mount(2, { route: 0, lane: 0, surfaces: [{ install() {}, clear() {} }] });
    lane.mount(2, [], 'table');
    const viewportHeight20Rows = 20 * ROW_HEIGHT[settings.density];
    lane.bind(2, { generation: 1, surfaces: [{ width: 320, height: viewportHeight20Rows, dpr: 1 }], offset: 0, pin: null, dated: { h: 120, c: 120 } }, wired({
      family: 'table', theme: TEST_THEME, kind: 'test-r5-art-1500', options: NO_OPTIONS, settings, sources: NO_SOURCES, width: 320, height: viewportHeight20Rows, scrollLeft: 0, scrollTop: 0,
    }));
    page.publish(120, { playing: false });
    await flushTicks();
    assert.equal(requestedArt.length, 6, `F6: 1500-country model starts bounded maxConcurrent=6 visible flags (got ${requestedArt.length})`);
    assert.ok(requestedArt.every(url => url.endsWith('_mid.png')), 'F6: requests only the displayed hour flag, never past/future look history');
    for (let i = 0; i < 10; i++) {
      const url = requestedArt[i]!;
      artResolvers.get(url)!({ width: 20, height: 14, close() { closedArt.push(url); } } as unknown as ImageBitmap);
      await flushTicks();
    }
    assert.equal(requestedArt.length, 16, 'F6: resolving 10 flags pumps the next visible flags up to 6 in flight');
    assert.ok(requestedArt.every(url => url.endsWith('_mid.png')), 'F6: every pumped visible flag is from the displayed hour');
    lane.unmount(2);
    assert.equal(abortedArt.length, 6, 'F6: unmount aborts remaining 6 in-flight fetches');
    assert.equal(closedArt.length, 10, 'F6: unmount closes all 10 decoded bitmaps');

    page.mount(3, { route: 0, lane: 0, surfaces: [{ install() {}, clear() {} }] });
    lane.mount(3, [], 'table');
    throwLocalPaint = true;
    lane.bind(3, { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: 10, dated: { h: 10, c: 10 } }, {
      family: 'table', theme: TEST_THEME, kind: 'test-r5-local-fail', options: {}, settings, sources: {}, width: 320, height: 200, scrollLeft: 0, scrollTop: 0,
    });
    page.publish(10, { playing: false });
    await flushTicks();
    assert.equal(page.view(3).mode, 'failed', 'F7: projection error puts widget in failed mode');
    assert.equal(page.view(3).failureOrigin, 'local', 'F7: local projection error carries local failure origin');
    throwLocalPaint = false;
    page.retryLocal(3);
    lane.bind(3, { generation: page.view(3).binding!, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: 10, dated: { h: 10, c: 10 } }, {
      family: 'table', theme: TEST_THEME, kind: 'test-r5-local-fail', options: {}, settings, sources: {}, width: 320, height: 200, scrollLeft: 0, scrollTop: 0,
    });
    await flushTicks();
    assert.equal(page.view(3).mode, 'picture', 'F7: local retry recovers without restarting pipeline');

    page.name(3, 'failed', 'stream broken', 'stream');
    assert.equal(page.view(3).failureOrigin, 'stream', 'F7: subscription/stream failure carries stream failure origin');

    lane.unmount(1);
    lane.unmount(3);
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }

  const layouts = buildColumnLayouts(cols, settings);
  const mkTipModel = (extraVal: ReturnType<typeof partialValue> | ReturnType<typeof knownValue>) => ({
    columns: cols,
    hour: 10,
    rows: [{
      key: 'GER',
      name: 'Germany',
      cells: { name: { value: knownValue('Germany') }, val: { value: knownValue(10) } },
      tooltipExtra: [{ label: 'Strength', value: extraVal, format: 'decimal1' as const }],
    }],
  });
  const mAtLeast = mkTipModel(partialValue(1.001, 'Missing corps', { atLeast: true }));
  const mPlainPartial = mkTipModel(partialValue(1.001, 'Missing corps', { atLeast: false }));
  const mDiffUnderlying = mkTipModel(partialValue(1.004, 'Missing corps', { atLeast: true }));
  const tipAtLeast = deriveTableTooltip(resolveTableRows(mAtLeast, settings), 'GER', cols)!;
  const extraRow = tipAtLeast.rows.find(r => r.label === 'Strength')!;
  assert.equal(extraRow.value, undefined, 'F8: tooltip row keeps typed ValueState instead of flattened string');
  assert.equal(extraRow.valueState?.kind, 'partial');
  assert.ok(tooltipText(tipAtLeast).includes('1, partial: Missing corps'), 'F8: tooltipText speaks via valueStateText without at least wording');
  assert.ok(!tooltipText(tipAtLeast).includes('at least'), 'F8: tooltipText omits at least wording');

  const auditA = deriveTableAudit({ model: mAtLeast, flatRows: resolveTableRows(mAtLeast, settings), visibleColumns: layouts });
  const auditB = deriveTableAudit({ model: mPlainPartial, flatRows: resolveTableRows(mPlainPartial, settings), visibleColumns: layouts });
  const auditC = deriveTableAudit({ model: mDiffUnderlying, flatRows: resolveTableRows(mDiffUnderlying, settings), visibleColumns: layouts });
  assert.notEqual(
    auditA.find(r => r.key === 'GER:_tooltip')?.value,
    auditB.find(r => r.key === 'GER:_tooltip')?.value,
    'F8: tooltip audit distinguishes atLeast: true vs false',
  );
  assert.notEqual(
    auditA.find(r => r.key === 'GER:_tooltip')?.value,
    auditC.find(r => r.key === 'GER:_tooltip')?.value,
    'F8: tooltip audit distinguishes distinct underlying numeric values that format identically',
  );
});

test('incremental row resolution, column maxima, scrollable/grouped dirty painting, and one-shot rank marker timers', () => {
  const cols: TableColumnSpec[] = [
    { key: 'name', label: 'Country', as: 'plain', offered: ['plain'], priority: 0, text: true },
    { key: 'score', label: 'Score', as: 'faint', tone: 'blue', offered: ['faint'], priority: 1, numeric: true, format: 'integer' },
    { key: 'extra', label: 'Extra', as: 'faint', tone: 'blue', offered: ['faint'], priority: 2, numeric: true, format: 'integer' },
  ];
  const settings = tableSettingsOf(undefined, cols, { density: 'compact', sort: { column: 'score', direction: 'descending' } });
  const layouts = buildColumnLayouts(cols, settings);
  const N = 120;
  let seed = 0x51a7f00d;
  const nextRand = () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed;
  };
  const scores = Array.from({ length: N }, (_, i) => (i * 17) % 97);
  const extras = Array.from({ length: N }, (_, i) => (i * 13) % 83);
  const makeRow = (i: number): TableRow => {
    const key = `C${String(i).padStart(3, '0')}`;
    return {
      key,
      name: `Country ${key}`,
      cells: {
        name: { value: knownValue(`Country ${key}`) },
        score: { value: scores[i] === 0 ? zeroValue(0) : knownValue(scores[i]!), format: 'integer' },
        extra: { value: extras[i] === 0 ? zeroValue(0) : knownValue(extras[i]!), format: 'integer' },
      },
    };
  };
  let rows = Array.from({ length: N }, (_, i) => makeRow(i));
  let prevModel: TableModel = { columns: cols, rows, hour: 1, rowTimeBoundaries: false, rank: true };
  let prevFlat = resolveTableRows(prevModel, settings);
  let prevIndex = new Map<string, number>();
  prevFlat.forEach((item, idx) => { if (item.kind === 'data') prevIndex.set(item.key, idx); });
  let prevMaxima = computeColumnMaxima(prevFlat, layouts);

  for (let step = 2; step <= 65; step++) {
    const k = step % 7 === 0 ? 0 : 1 + (nextRand() % 8);
    const nextRows = rows.slice();
    const changedKeys = new Set<string>();
    const changedRows = new Map<string, TableRow>();
    for (let j = 0; j < k; j++) {
      const idx = nextRand() % N;
      if (j === 0 && step % 5 === 0) {
        const leaderKey = prevFlat[0]!.key;
        const leaderIdx = Number(leaderKey.slice(1));
        scores[leaderIdx] = 1;
        extras[leaderIdx] = 1;
        const built = makeRow(leaderIdx);
        nextRows[leaderIdx] = built;
        changedKeys.add(built.key);
        changedRows.set(built.key, built);
      } else {
        scores[idx] = nextRand() % 120;
        extras[idx] = nextRand() % 150;
        const built = makeRow(idx);
        nextRows[idx] = built;
        changedKeys.add(built.key);
        changedRows.set(built.key, built);
      }
    }
    const nextModel: TableModel = {
      columns: cols,
      rows: nextRows,
      hour: step,
      rowTimeBoundaries: false,
      rank: true,
      changedKeys,
      changedRows,
    };
    const inc = resolveTableRowsIncremental(prevModel, prevFlat, prevIndex, nextModel, settings);
    assert.ok(inc !== null, `step ${step} uses incremental sort`);
    const incRows: ReturnType<typeof resolveTableRows> = inc.flatRows;
    const full = resolveTableRows(nextModel, settings);
    assert.equal(incRows.length, full.length, `step ${step} length matches`);
    for (let i = 0; i < full.length; i++) {
      const incItem = incRows[i]!;
      const fullItem = full[i]!;
      assert.equal(incItem.kind, 'data');
      assert.equal(fullItem.kind, 'data');
      assert.equal(incItem.key, fullItem.key, `step ${step} key at ${i}`);
      assert.equal(incItem.rank, fullItem.rank, `step ${step} rank at ${i} (${incItem.key})`);
      assert.equal(inc.rowIndex.get(incItem.key), i, `step ${step} rowIndex for ${incItem.key}`);
    }
    const incMax = updateColumnMaximaIncremental(prevMaxima, prevFlat, prevIndex, incRows, inc.rowIndex, layouts, changedKeys);
    const fullMax = computeColumnMaxima(full, layouts);
    assert.deepEqual([...incMax.entries()].sort(), [...fullMax.entries()].sort(), `step ${step} maxima match`);

    rows = nextRows;
    prevModel = nextModel;
    prevFlat = incRows;
    prevIndex = inc.rowIndex;
    prevMaxima = incMax;
  }

  const { ctx, counts } = instrumentedContext();
  (ctx as unknown as { createLinearGradient: () => { addColorStop: () => void } }).createLinearGradient = () => ({ addColorStop() {} });
  const scrollSettings = tableSettingsOf(undefined, cols, { density: 'compact', sort: { column: 'score', direction: 'descending' } });
  const scrollLayout = layoutFamilyTable(buildColumnLayouts(cols, scrollSettings, { name: 160, score: 160, extra: 160 }), 260);
  assert.equal(scrollLayout.scrollable, true);
  const groupedRows = resolveTableRows({
    columns: cols,
    hour: 1,
    rank: true,
    rows: [
      { key: 'A', name: 'Alpha', group: { key: 'g1', label: 'Axis', order: 0 }, cells: { name: { value: knownValue('Alpha') }, score: { value: knownValue(100), format: 'integer' }, extra: { value: knownValue(50), format: 'integer' } } },
      { key: 'B', name: 'Bravo', group: { key: 'g1', label: 'Axis', order: 0 }, cells: { name: { value: knownValue('Bravo') }, score: { value: knownValue(80), format: 'integer' }, extra: { value: knownValue(40), format: 'integer' } } },
      ...Array.from({ length: 20 }, (_, i) => ({
        key: `Z${i}`,
        name: `Z${i}`,
        group: { key: 'g2', label: 'Other', order: 1 },
        cells: { name: { value: knownValue(`Z${i}`) }, score: { value: knownValue(20 - i), format: 'integer' as const }, extra: { value: knownValue(10), format: 'integer' as const } },
      })),
    ],
  }, scrollSettings);
  const pic = createTablePicture();
  drawFamilyTable(ctx, { layout: scrollLayout, rows: groupedRows, settings: scrollSettings, hour: 1, width: 260, height: 100, scrollLeft: 0, scrollTop: 0, rankShown: true, rankWidth: 36, theme: TEST_THEME, now: 1, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), dpr: 1 }, pic);
  const fillsAfterFull = counts().fillTextCalls;
  const offscreenMutated = groupedRows.slice();
  const lastIdx = offscreenMutated.length - 1;
  const lastItem = offscreenMutated[lastIdx]!;
  if (lastItem.kind === 'data') {
    offscreenMutated[lastIdx] = { ...lastItem, row: { ...lastItem.row, cells: { ...lastItem.row.cells, score: { value: knownValue(19), format: 'integer' } } } };
  }
  const offscreenRes = drawFamilyTable(ctx, { layout: scrollLayout, rows: offscreenMutated, settings: scrollSettings, hour: 2, width: 260, height: 100, scrollLeft: 0, scrollTop: 0, rankShown: true, rankWidth: 36, theme: TEST_THEME, now: 2, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), dpr: 1 }, pic);
  assert.equal(offscreenRes.drew, false, 'off-screen row change in scrollable grouped table performs zero paint');
  assert.equal(counts().fillTextCalls, fillsAfterFull, 'off-screen row change emits zero fillText calls');

  const visibleMutated = offscreenMutated.slice();
  const visItem = visibleMutated[2]!;
  if (visItem.kind === 'data') {
    visibleMutated[2] = { ...visItem, row: { ...visItem.row, cells: { ...visItem.row.cells, score: { value: knownValue(81), format: 'integer' } } } };
  }
  const visRes = drawFamilyTable(ctx, { layout: scrollLayout, rows: visibleMutated, settings: scrollSettings, hour: 3, width: 260, height: 100, scrollLeft: 0, scrollTop: 0, rankShown: true, rankWidth: 36, theme: TEST_THEME, now: 3, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), dpr: 1 }, pic);
  assert.equal(visRes.drew, true);
  assert.equal(visRes.paintedCells, 1, 'scrollable grouped single-cell change repaints only 1 dirty cell');
  assert.ok(counts().fillTextCalls - fillsAfterFull < fillsAfterFull, `scrollable grouped single-cell change draws fewer texts (${counts().fillTextCalls - fillsAfterFull} vs full ${fillsAfterFull})`);

  registerFamilyProjection(declareTableProjection<{ rows: { id: string; v: number }[] }, { id: string; v: number }, Record<string, never>>({
    kind: 'test-rank-marker-timer',
    rank: true,
    rankMarkers: true,
    columns: [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, field: 'id', text: true },
      { key: 'v', label: 'Val', as: 'plain', offered: ['plain'], priority: 1, field: 'v', numeric: true },
    ],
    rows: reply => ({ rows: reply.rows }),
    rowKey: r => r.id,
    rowName: r => r.id,
  }));
  const recording = installRecordingCanvas();
  try {
    const armed: number[] = [];
    let due: (() => void) | null = null;
    let wakes = 0;
    const view = createLaneView(tableLane, () => { wakes++; }, {
      timers: {
        set: (fn, ms) => { due = fn; armed.push(ms); return armed.length as unknown as ReturnType<typeof setTimeout>; },
        clear: () => { due = null; },
      },
    });
    const lane = driveLane(view, recording.canvases, { width: 320, height: 180, dpr: 1 });
    const rmSettings = tableSettingsOf(undefined, [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'] },
      { key: 'v', label: 'Val', as: 'plain', offered: ['plain'] },
    ], { density: 'compact', sort: { column: 'v', direction: 'descending' }, showRank: true });
    const options = {};
    const sources = {};
    const bound = (rows: { id: string; v: number }[]) => ({ family: 'table' as const, kind: 'test-rank-marker-timer', options, settings: rmSettings, sources, staticPanels: { main: { rows } }, width: 320, height: 180, scrollLeft: 0, scrollTop: 0, theme: TEST_THEME, reducedMotion: true });
    const arrow = () => lane.picture().some(mark => mark.includes('▲'));
    const swappedRows = bound([{ id: 'A', v: 5 }, { id: 'B', v: 30 }]);
    lane.paint({ label: 'before', binding: bound([{ id: 'A', v: 20 }, { id: 'B', v: 10 }]), hour: 1, speed: 24, seek: 1, now: 1000 });
    const swapped = lane.paint({ label: 'swapped', binding: swappedRows, hour: 2, speed: 24, seek: 1, now: 1050 });
    assert.ok(arrow(), 'the forward swap draws the rank arrow');
    assert.equal(swapped.motion, false, 'rank marker expiry does not hold the 60 FPS motion loop');
    assert.equal(swapped.fields.mirror, undefined, 'the mirror is withheld until its cadence is due');
    assert.deepEqual(armed, [950], 'one timer, for the mirror, which is due before the arrow expires');
    (due as (() => void) | null)?.();
    const mirrored = lane.paint({ label: 'mirror wake', binding: swappedRows, hour: 2, speed: 24, seek: 1, now: 2000 });
    assert.ok(mirrored.fields.mirror, 'the mirror wake publishes the mirror');
    assert.ok(arrow(), 'the arrow survives the earlier mirror wake');
    assert.deepEqual(armed, [950, 1550], 'then one timer for the arrow’s own deadline');
    (due as (() => void) | null)?.();
    lane.paint({ label: 'arrow expired', binding: swappedRows, hour: 2, speed: 24, seek: 1, now: 3550 });
    assert.equal(arrow(), false, 'the arrow disappears at its own deadline');
    assert.deepEqual([wakes, armed.length], [2, 2], 'one wake per deadline and no timer after it');
    view.unmount?.(1, lane.state);
  } finally {
    recording.restore();
  }
});

function gridContext(width: number, height: number) {
  type Box = readonly [number, number, number, number];
  const marks = new Map<string, string>();
  const at = (x: number, y: number) => `${Math.round(x)},${Math.round(y)}`;
  const canvasBox: Box = [0, 0, width, height];
  let clipBox: Box = canvasBox;
  let pending: Box | null = null;
  const stack: Box[] = [];
  const inside = (x: number, y: number) => x >= clipBox[0] && x < clipBox[2] && y >= clipBox[1] && y < clipBox[3];
  const clear = (x: number, y: number, w: number, h: number) => {
    for (const key of [...marks.keys()]) {
      const [px, py] = key.split(',').map(Number) as [number, number];
      if (px >= x && px < x + w && py >= y && py < y + h && inside(px, py)) marks.delete(key);
    }
  };
  const mark = (x: number, y: number, text: string) => { if (inside(x, y)) marks.set(at(x, y), text); };
  const target: Record<string, unknown> = {
    font: '', fillStyle: '', strokeStyle: '', lineWidth: 1, textAlign: 'left', textBaseline: 'alphabetic', globalAlpha: 1,
    save: () => { stack.push(clipBox); },
    restore: () => { clipBox = stack.pop() ?? canvasBox; },
    beginPath: () => { pending = null; },
    rect: (x: number, y: number, w: number, h: number) => { pending = [x, y, x + w, y + h]; },
    clip: () => {
      if (pending) clipBox = [Math.max(clipBox[0], pending[0]), Math.max(clipBox[1], pending[1]), Math.min(clipBox[2], pending[2]), Math.min(clipBox[3], pending[3])];
    },
    fillText: (text: string, x: number, y: number) => mark(x, y, String(text)),
    fillRect: (x: number, y: number, w: number, h: number) => clear(x, y, w, h),
    clearRect: (x: number, y: number, w: number, h: number) => clear(x, y, w, h),
    drawImage: (_img: unknown, x: number, y: number) => mark(x, y, 'image'),
    measureText: (text: string) => ({ width: String(text).length * 7 }),
    createLinearGradient: () => ({ addColorStop() {} }),
    createPattern: () => null,
    getTransform: () => ({ a: 1, d: 1 }),
  };
  const ctx = new Proxy(target, { get: (t, k) => (k in t ? t[k as string] : () => {}) });
  return { ctx, marks };
}

test('an incremental table paint after a seek back, a same-hour correction and a reorder equals a fresh paint, cell by cell', () => {
  type Row = { id: string; v: number; flag: string };
  const columns: TableColumnSpec[] = [
    { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true },
    { key: 'flag', label: 'Flag', as: 'plain', offered: ['plain'], priority: 1, text: true },
    { key: 'v', label: 'Val', as: 'faint', barEligible: true, offered: ['plain', 'faint'], priority: 2, numeric: true },
  ];
  registerFamilyProjection({
    kind: 'test-fresh-equals-incremental',
    family: 'table', theme: TEST_THEME,
    project: readView => {
      const reply = readView.panel('main') as { rows: Row[]; refused?: string };
      return {
        hour: readView.hour,
        ...(reply.refused ? { refused: reply.refused } : {}),
        rank: true,
        defaultRankShown: true,
        rankMarkers: true,
        columns,
        rows: reply.rows.map(r => ({
          key: r.id,
          name: r.id,
          cells: {
            name: { value: knownValue(r.id) },
            flag: { value: knownValue('Owner'), flags: [{ tag: r.flag }] },
            v: { value: knownValue(r.v), format: 'integer' },
          },
        })),
      };
    },
  });
  const contexts: ReturnType<typeof gridContext>[] = [];
  const prevOffscreen = globalThis.OffscreenCanvas;
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = class {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { const grid = gridContext(this.width, this.height); contexts.push(grid); return grid.ctx; }
    transferToImageBitmap() { return { close() {} }; }
  };
  try {
    const settingsFor = (direction: 'ascending' | 'descending') => tableSettingsOf(undefined, columns, { density: 'normal', sort: { column: 'v', direction } });
    const lane = () => {
      const view = createLaneView(tableLane, () => {});
      const state = new Map<string, unknown>();
      const transport = new LaneTransport();
      let seq = 1;
      const before = contexts.length;
      const paint = (rows: Row[], hour: number, settings: TableSettings) => view.paint({
        widget: 1,
        subs: [],
        pictures: [],
        model: transport.carry({ family: 'table' as const, kind: 'test-fresh-equals-incremental', options: NO_OPTIONS, settings, sources: NO_SOURCES, staticPanels: { main: { rows } }, width: 360, height: 200, scrollLeft: 0, scrollTop: 0, theme: TEST_THEME, reducedMotion: true }),
        hour,
        seek: 0, speed: 24,
        binding: { generation: 1, surfaces: [{ width: 360, height: 200, dpr: 1 }], offset: 0, pin: null },
        inspect: null,
        now: 5000,
      }, state, () => `c${seq++}`);
      return { paint, marks: () => new Map(contexts[before]!.marks) };
    };
    const fresh = (rows: Row[], hour: number, settings: TableSettings) => {
      const other = lane();
      other.paint(rows, hour, settings);
      return other.marks();
    };
    const atOne: Row[] = [{ id: 'A', v: 20, flag: 'X' }, { id: 'B', v: 10, flag: 'Y' }, { id: 'C', v: 5, flag: 'Z' }];
    const atTwo: Row[] = [{ id: 'A', v: 5, flag: 'X' }, { id: 'B', v: 30, flag: 'Y' }, { id: 'C', v: 6, flag: 'Z' }];
    const corrected: Row[] = [{ id: 'A', v: 20, flag: 'W' }, { id: 'B', v: 10, flag: 'Y' }, { id: 'C', v: 5, flag: 'Z' }];
    const descending = settingsFor('descending');
    const ascending = settingsFor('ascending');
    const played = lane();
    const first = played.paint(atOne, 1, descending);
    const same = played.paint([...atOne], 1, descending);
    assert.equal(same.surfaces[0]?.content, first.surfaces[0]?.content, 'unchanged visible cells keep their bitmap');
    played.paint(atTwo, 2, descending);
    assert.ok([...played.marks().values()].some(text => text.includes('▲')), 'a forward step marks the rank change');

    played.paint(atOne, 1, descending);
    assert.deepEqual(played.marks(), fresh(atOne, 1, descending), 'after a seek back the picture equals a fresh paint');

    played.paint(corrected, 1, descending);
    assert.deepEqual(played.marks(), fresh(corrected, 1, descending), 'a same-hour correction of a cell’s sole flag repaints it');

    played.paint(corrected, 1, ascending);
    assert.deepEqual(played.marks(), fresh(corrected, 1, ascending), 'a reorder at the same hour equals a fresh paint');

    const replaced = played.paint([{ id: 'D', v: 40, flag: 'X' }, { id: 'B', v: 10, flag: 'Y' }, { id: 'C', v: 5, flag: 'Z' }], 2, ascending);
    const navKeys = (replaced.fields.nav?.value as readonly (readonly [string])[]).map(item => item[0]);
    assert.deepEqual([...navKeys].sort(), ['B', 'C', 'D'], 'navigation installed with the picture offers the subjects painted, never the replaced one');

    const grouped = resolveTableRows({
      hour: 10,
      columns,
      rows: [
        { key: 'P', name: 'P', cells: { name: { value: knownValue('P') }, v: { value: knownValue(1) } } },
        { key: 'Q', name: 'Q', revealAt: 20, group: { key: 'g', label: 'Group' }, cells: { name: { value: knownValue('Q') }, v: { value: knownValue(2) } } },
      ],
    }, descending);
    assert.deepEqual(grouped.map(item => item.kind), ['data'], 'a group revealed later adds no heading before its hour');

    const laneBinding = (rows: Row[], settings: TableSettings, scrollTop = 0, reducedMotion = true) => ({
      family: 'table' as const, theme: TEST_THEME, kind: 'test-fresh-equals-incremental', options: NO_OPTIONS, settings, sources: NO_SOURCES, staticPanels: { main: { rows } },
      width: 360, height: 200, scrollLeft: 0, scrollTop, reducedMotion,
    });
    const tableSurface = { width: 360, height: 200, dpr: 1 };
    const heldOne = laneBinding(atOne, descending);
    assertQuietFrames(() => createLaneView(tableLane, () => {}), { label: 'table', binding: heldOne, hour: 1 }, 24, tableSurface);
    assertTooltipDated(() => createLaneView(tableLane, () => {}), { label: 'table', binding: heldOne, hour: 1 }, firstRow, 5, tableSurface);
    assertFreshPaint(() => createLaneView(tableLane, () => {}), [
      { label: 'at one', binding: heldOne, hour: 1, check: true },
      { label: 'at two', binding: laneBinding(atTwo, descending), hour: 2, speed: 24 },
      { label: 'seek back', binding: heldOne, hour: 1, check: true },
      { label: 'correction', binding: laneBinding(corrected, descending), hour: 1, check: true },
      { label: 'reorder', binding: laneBinding(corrected, ascending), hour: 1, check: true },
      { label: 'scrolled', binding: laneBinding(corrected, ascending, 10), hour: 1, check: true },
    ], tableSurface);
    const refusedBinding = { ...laneBinding([], descending), staticPanels: { main: { rows: [], refused: 'Scores were not recorded at this hour' } } };
    for (const reducedMotion of [true, false]) {
      assertContinuity(() => createLaneView(tableLane, () => {}), {
        from: { label: `at one${reducedMotion ? '' : ', gliding'}`, binding: laneBinding(atOne, descending, 0, reducedMotion), hour: 1 },
        refused: { label: 'a refused hour', binding: refusedBinding, hour: 2 },
        to: { label: `rank markers at three${reducedMotion ? '' : ' with a glide'}`, binding: laneBinding(atTwo, descending, 0, reducedMotion), hour: 3 },
      }, tableSurface);
    }
    type Side = { key: string; lead: string; total: number; rows: Row[] };
    const shareColumns: TableColumnSpec[] = [
      { key: 'name', label: 'Country', as: 'plain', offered: ['plain'], priority: 0, text: true },
      ...['a', 'b', 'c', 'd', 'e', 'f'].map((key, at): TableColumnSpec => ({ key, label: key.toUpperCase(), as: 'plain', offered: ['plain'], priority: at + 1, numeric: true, cellKind: 'share', barEligible: false, minWidth: 96 })),
    ];
    const shareCell = (v: number) => ({ value: v === 0 ? { kind: 'zero' as const, value: 0 } : knownValue(v), format: { decimals: 1, percent: true } as const, bar: { fraction: knownValue(v / 100) }, countryColor: '#cc3333' });
    registerFamilyProjection({
      kind: 'test-panes-fresh',
      family: 'table', theme: TEST_THEME,
      project: readView => {
        const sides = (readView.panel('main') as { sides: Side[] }).sides;
        const row = (key: string, name: string, v: number, group?: boolean) => ({
          key, name,
          ...(group ? { group: { key: 'side', label: 'Share of the side' } } : {}),
          cells: { name: { value: knownValue(name) }, ...Object.fromEntries(['a', 'b', 'c', 'd', 'e', 'f'].map((col, at) => [col, shareCell(Math.max(0, v - at))])) },
        });
        return {
          hour: readView.hour,
          columns: shareColumns,
          rows: [],
          panes: sides.map(side => ({
            key: side.key,
            title: { name: knownValue(side.lead) },
            totalRow: row(`total:${side.key}`, 'Share of the war', side.total),
            rows: side.rows.map(r => row(`${side.key}:${r.id}`, r.id, r.v, true)),
          })),
        };
      },
    });
    const paneLane = () => {
      const view = createLaneView(tableLane, () => {});
      const state = new Map<string, unknown>();
      let seq = 1;
      const before = contexts.length;
      const transport = new LaneTransport();
      const paint = (sides: Side[], hour: number, settings: TableSettings, scrollLeft = 0) => view.paint({
        widget: 2,
        subs: [],
        pictures: [],
        model: { family: 'table' as const, kind: 'test-panes-fresh', options: transport.carry({}), settings, sources: {}, staticPanels: { main: { sides } }, width: 700, height: 240, scrollLeft, scrollTop: 0, theme: TEST_THEME, reducedMotion: true },
        hour,
        seek: 0, speed: 24,
        binding: { generation: 1, surfaces: [{ width: 700, height: 240, dpr: 1 }], offset: 0, pin: null },
        inspect: null,
        now: 5000,
      }, state, () => `p${seq++}`);
      return { paint, marks: () => new Map(contexts[before]!.marks) };
    };
    const freshPanes = (sides: Side[], hour: number, settings: TableSettings, scrollLeft = 0) => {
      const other = paneLane();
      other.paint(sides, hour, settings, scrollLeft);
      return other.marks();
    };
    const sharesBy = (direction: 'ascending' | 'descending') => tableSettingsOf(undefined, shareColumns, { density: 'compact', barsBehind: 'off', sort: { column: 'a', direction } });
    const sidesAt = (shift: number): Side[] => [
      { key: 'left', lead: 'Alpha', total: 60 + shift, rows: [{ id: 'P', v: 70 - shift, flag: '' }, { id: 'Q', v: 30 + shift, flag: '' }] },
      { key: 'right', lead: 'Beta', total: 40 - shift, rows: [{ id: 'R', v: 55, flag: '' }, { id: 'S', v: 45, flag: '' }, { id: 'T', v: 0, flag: '' }] },
    ];
    const panes = paneLane();
    panes.paint(sidesAt(0), 1, sharesBy('descending'));
    const stepped = panes.paint(sidesAt(30), 2, sharesBy('descending'));
    const paneNav = (stepped.fields.nav?.value as readonly (readonly [string])[]).map(item => item[0]);
    assert.deepEqual(paneNav, ['total:left', 'left:Q', 'left:P', 'total:right', 'right:R', 'right:S', 'right:T'], 'navigation runs down the first pane, then the second, each in its own sorted order under its total row');
    assert.ok([...panes.marks().values()].some(text => text.includes('Share of the war')), 'the total row is drawn');
    assert.ok([...panes.marks().values()].some(text => text.includes('Share of the side')), 'the group label is drawn');
    const widthsField = stepped.fields.widths?.value as { totalWidth: number };
    assert.ok(widthsField.totalWidth > 700, 'six share columns overflow each pane, so the view scrolls sideways');
    const paneHit = stepped.hit?.([700 - 20, 30 + 26 * 2]);
    assert.ok(typeof paneHit === 'string' && paneHit.startsWith('row:right:'), `a point in the second pane hits that pane's row (${String(paneHit)})`);
    panes.paint(sidesAt(0), 1, sharesBy('descending'));
    assert.deepEqual(panes.marks(), freshPanes(sidesAt(0), 1, sharesBy('descending')), 'two panes after a seek back equal a fresh paint');
    const correctedSides = sidesAt(0).map(side => (side.key === 'right' ? { ...side, lead: 'Beta renamed', total: 41 } : side));
    panes.paint(correctedSides, 1, sharesBy('descending'));
    assert.deepEqual(panes.marks(), freshPanes(correctedSides, 1, sharesBy('descending')), 'a same-hour correction in one pane equals a fresh paint');
    panes.paint(correctedSides, 1, sharesBy('ascending'));
    assert.deepEqual(panes.marks(), freshPanes(correctedSides, 1, sharesBy('ascending')), 'a reorder in both panes equals a fresh paint');
    panes.paint(correctedSides, 1, sharesBy('ascending'), 120);
    assert.deepEqual(panes.marks(), freshPanes(correctedSides, 1, sharesBy('ascending'), 120), 'one shared sideways offset moves both panes as a fresh paint would');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }
});

test('on a 1,500-row table a scroll or a focus change derives no navigation again and serialises nothing that grows with the rows, and a rank-animation frame copies and sorts no mover off the viewport', () => {
  type Row = { id: string; v: number };
  const columns: TableColumnSpec[] = [
    { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true },
    { key: 'v', label: 'Val', as: 'plain', offered: ['plain'], priority: 1, numeric: true },
  ];
  registerFamilyProjection({
    kind: 'test-rank-1500',
    family: 'table', theme: TEST_THEME,
    project: readView => ({
      hour: readView.hour, rank: true, defaultRankShown: true, rankMarkers: true, columns,
      rows: (readView.panel('main') as { rows: Row[] }).rows.map(r => ({ key: r.id, name: r.id, cells: { name: { value: knownValue(r.id) }, v: { value: knownValue(r.v), format: 'integer' } } })),
    }),
  });
  const N = 1500;
  const settings = tableSettingsOf(undefined, columns, { density: 'normal', sort: { column: 'v', direction: 'descending' } });
  const ascending = Array.from({ length: N }, (_, i): Row => ({ id: `C${String(i).padStart(4, '0')}`, v: i }));
  const reversed = ascending.map((row, i): Row => ({ ...row, v: N - i }));
  const panels = new Map<Row[], { readonly main: { readonly rows: Row[] } }>();
  const bound = (rows: Row[], scrollTop = 0, focusedKey: string | null = null) => ({
    family: 'table' as const, theme: TEST_THEME, kind: 'test-rank-1500', options: NO_OPTIONS, settings, sources: NO_SOURCES, staticPanels: panels.get(rows) ?? panels.set(rows, { main: { rows } }).get(rows)!,
    width: 600, height: 300, scrollLeft: 0, scrollTop, focusedKey, reducedMotion: false,
  });
  const counted = <T>(run: () => T): { readonly result: T; readonly serialised: number; readonly mapped: number; readonly sorted: number } => {
    const stringify = JSON.stringify;
    const set = Map.prototype.set;
    const sort = Array.prototype.sort;
    let serialised = 0;
    let mapped = 0;
    let sorted = 0;
    JSON.stringify = ((...args: Parameters<typeof stringify>) => { const text = stringify(...args); serialised += text?.length ?? 0; return text; }) as typeof stringify;
    Map.prototype.set = function (this: Map<unknown, unknown>, key: unknown, value: unknown) { mapped++; return set.call(this, key, value); };
    Array.prototype.sort = function (this: unknown[], compare?: (a: unknown, b: unknown) => number) { sorted += this.length; return sort.call(this, compare); };
    try {
      return { result: run(), serialised, mapped, sorted };
    } finally {
      JSON.stringify = stringify;
      Map.prototype.set = set;
      Array.prototype.sort = sort;
    }
  };
  const recording = installRecordingCanvas();
  try {
    const lane = driveLane(createLaneView(tableLane, () => {}), recording.canvases, { width: 600, height: 300, dpr: 1 });
    const first = lane.paint({ label: 'ascending', binding: bound(ascending), hour: 1, speed: 24, seek: 1, now: 1000 });
    const navText = JSON.stringify(first.fields.nav?.value).length;
    for (const [label, binding] of [['a scroll', bound(ascending, 600)], ['a focus change', bound(ascending, 600, 'C0700')]] as const) {
      const step = counted(() => lane.paint({ label, binding, hour: 1, speed: 24, seek: 1, now: 1001 }));
      for (const name of ['nav', 'widths', 'meta']) assert.equal(step.result.fields[name], first.fields[name], `${label}: ${name} is not derived again`);
      assert.ok(step.serialised < navText / 10, `${label}: serialises ${step.serialised} characters, not the ${navText} of the navigation`);
    }
    lane.paint({ label: 'every row moves', binding: bound(reversed, 600, 'C0700'), hour: 2, speed: 24, seek: 1, now: 2000 });
    const frame = counted(() => lane.paint({ label: 'a frame of the glide', binding: bound(reversed, 600, 'C0700'), hour: 2, speed: 24, seek: 1, now: 2050 }));
    assert.equal(frame.result.motion, true, 'the glide is still running');
    assert.ok(frame.mapped < 100, `an animation frame copies ${frame.mapped} map entries, not one per mover`);
    assert.ok(frame.sorted < 100, `an animation frame sorts ${frame.sorted} entries, not every mover`);
  } finally {
    recording.restore();
  }
});

test('the movers a viewport evaluates are exactly those whose path crosses it', () => {
  const N = 1500;
  const rows = Array.from({ length: N }, (_, i): FlatTableRow => ({ kind: 'data', key: `R${i}`, name: `R${i}`, level: 1, rank: i + 1, row: { key: `R${i}`, name: `R${i}`, cells: {} } } as FlatTableRow));
  const rowIndex = new Map(rows.map((row, i) => [row.key, i] as const));
  const offsets = new Map(rows.map((row, i) => [row.key, { from: (N - 1 - 2 * i) * 20, to: 0, started: 0 }] as const));
  const glide = tableGlide(offsets, rows, rowIndex, null, 20);
  for (const [top, bottom] of [[0, 300], [14000, 14300], [29700, 30000]] as const) {
    const brute = rows.flatMap((_, i) => {
      const from = offsets.get(`R${i}`)!.from;
      const lo = i * 20 + Math.min(from, 0);
      const hi = i * 20 + Math.max(from, 0) + 20;
      return hi > top && lo < bottom ? [i] : [];
    });
    assert.deepEqual([...glideCrossing(glide, top, bottom)].sort((a, b) => a - b), brute, `viewport ${top}..${bottom}`);
  }
  assert.ok(glideCrossing(glide, 0, 300).length < 40, 'a viewport at the top meets only the rows passing through it');
});

test('cell kinds for converted tables: an item column takes the spare width, two-way bars scale by magnitude, declared extras reach the cell, a blank cell draws nothing, and names that arrive re-measure the column', () => {
  const kinds: TableColumnSpec[] = [
    { key: 'pic', label: '', as: 'plain', offered: ['plain'], priority: 0, cellKind: 'kind', sortable: false },
    { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, cellKind: 'item' },
    { key: 'net', label: 'Net', as: 'plain', offered: ['plain'], priority: 1, numeric: true, cellKind: 'twoWay', widest: ['+99'] },
  ];
  const kindSettings = tableSettingsOf(undefined, kinds, { density: 'normal', sort: { column: 'net', direction: 'descending' } });
  const grown = layoutFamilyTable(buildColumnLayouts(kinds, kindSettings, { pic: 52, name: 128, net: 140 }), 600);
  assert.deepEqual(grown.columns.map(c => c.width), [58, 600 - 58 - 146, 146], 'the item column takes all the spare width; the kind and two-way columns keep theirs');

  const signedRows = resolveTableRows({ hour: 1, columns: kinds, rows: [
    { key: 'A', name: 'A', cells: { name: { value: knownValue('A') }, net: { value: knownValue(-30) } } },
    { key: 'B', name: 'B', cells: { name: { value: knownValue('B') }, net: { value: knownValue(10) } } },
  ] }, kindSettings);
  assert.equal(computeColumnMaxima(signedRows, grown.columns).get('net'), 30, 'a two-way column scales by the largest magnitude');
  const netColumn = grown.columns.find(col => col.key === 'net')!;
  const loss = signedRows.find(item => item.kind === 'data' && item.key === 'A');
  assert.equal(loss?.kind === 'data' ? resolveCellPaintModel(loss.row.cells.net, netColumn, new Map([['net', 30]])).bar.fraction : null, 1, 'a loss draws its bar at its magnitude');
  const measuredNet = measureFixedColumnWidths(buildColumnLayouts([kinds[2]!], kindSettings), kindSettings, TEST_THEME, text => text.length * 7).net!;
  assert.ok(measuredNet - 2 * 10 >= 72 + 8 + 7 * 3, `a two-way column leaves room for its 72 px slot beside the figure (${measuredNet})`);

  const partialModel = { hour: 1, rank: true, columns: kinds, populationPartial: 'Some countries were not read at this hour', rows: [
    { key: 'small', name: 'small', cells: { name: { value: knownValue('small') }, net: { value: knownValue(1) } } },
    { key: 'large', name: 'large', cells: { name: { value: knownValue('large') }, net: { value: knownValue(9) } } },
  ] };
  assert.deepEqual(resolveTableRows(partialModel, kindSettings).map(item => item.key), ['small', 'large'], 'a partly answered population keeps the product’s order, never re-ranked in the client');

  const shadedColumns: TableColumnSpec[] = [kinds[1]!, { key: 'oil', label: 'Oil', as: 'plain', offered: ['plain'], priority: 1, numeric: true, cellKind: 'shaded' }];
  const shadedSettings = tableSettingsOf(undefined, shadedColumns, { density: 'compact', sort: { column: 'oil', direction: 'descending' } });
  const shadedLayout = layoutFamilyTable(buildColumnLayouts(shadedColumns, shadedSettings, { name: 128, oil: 60 }), 400);
  const shadedRows = resolveTableRows({ hour: 1, columns: shadedColumns, rows: [
    { key: 'A', name: 'A', cells: { name: { value: knownValue('A') }, oil: { value: knownValue(-40) } } },
    { key: 'B', name: 'B', cells: { name: { value: knownValue('B') }, oil: { value: knownValue(12) } } },
  ] }, shadedSettings);
  assert.equal(computeColumnMaxima(shadedRows, shadedLayout.columns).get('oil'), 40, 'a shaded column scales imports and exports by the largest magnitude');

  const extraProjection = declareTableProjection<{ rows: { id: string; group: boolean }[] }, { id: string; group: boolean }, Record<string, never>>({
    kind: 'test-cell-extras',
    columns: [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, field: 'id', text: true, extra: row => ({ subtext: `${row.id} stored`, flags: [{ tag: 'GER', art: PENDING_ART }, { tag: 'SOV', art: PENDING_ART, dimmed: true }] }) },
      { key: 'total', label: 'Total', as: 'plain', offered: ['plain'], priority: 1, numeric: true, value: () => notApplicable('Group rows only'), extra: row => (row.group ? {} : { blank: true }) },
    ],
    rows: reply => ({ rows: reply.rows }),
    rowKey: r => r.id,
    rowName: r => r.id,
  });
  const extraModel = extraProjection.project(createBoundedReadView({ hour: 1, panels: new Map([['main', { rows: [{ id: 'G', group: true }, { id: 'C', group: false }] }]]) }), {});
  assert.equal(extraModel.rows[0]!.cells.name!.subtext, 'G stored');
  assert.deepEqual(extraModel.rows[0]!.cells.name!.flags?.map(f => [f.tag, f.dimmed === true]), [['GER', false], ['SOV', true]], 'two flags in one cell, the actor first and the other dimmed');
  assert.equal(extraModel.rows[1]!.cells.total!.blank, true);

  const grid = gridContext(400, 120);
  const extraSettings = tableSettingsOf(undefined, extraModel.columns, { density: 'normal', sort: { column: 'name', direction: 'ascending' } });
  const extraLayout = layoutFamilyTable(buildColumnLayouts(extraModel.columns, extraSettings, { name: 200, total: 120 }), 400);
  drawFamilyTable(grid.ctx as unknown as CanvasRenderingContext2D, { layout: extraLayout, rows: resolveTableRows(extraModel, extraSettings), settings: extraSettings, hour: 1, width: 400, height: 120, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 1, glideDuration: 0, glide: NO_GLIDE, markers: new Map(), dpr: 1 });
  const texts = [...grid.marks.values()];
  assert.equal(texts.filter(t => t === '—').length, 1, 'the group row shows its not-applicable dash; the blank child cell draws nothing');

  const names = new Map<string, string>([['A', 'A']]);
  registerFamilyProjection({
    kind: 'test-names-arrive',
    family: 'table', theme: TEST_THEME,
    project: readView => ({
      hour: readView.hour,
      columns: [{ key: 'name', label: 'Country', as: 'plain', offered: ['plain'], priority: 0, text: true }],
      rows: [{ key: 'A', name: names.get('A')!, cells: { name: { value: knownValue(names.get('A')!) } } }],
    }),
  });
  const prevOffscreen = globalThis.OffscreenCanvas;
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = class {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return gridContext(this.width, this.height).ctx; }
    transferToImageBitmap() { return { close() {} }; }
  };
  try {
    const view = createLaneView(tableLane, () => {});
    const state = new Map<string, unknown>();
    let seq = 1;
    const settings = tableSettingsOf(undefined, [{ key: 'name', label: 'Country', as: 'plain', offered: ['plain'] }], { density: 'normal', sort: { column: 'name', direction: 'ascending' } });
    const transport = new LaneTransport();
    const paint = () => view.paint({
      widget: 1, subs: [], pictures: [],
      model: { family: 'table' as const, kind: 'test-names-arrive', options: transport.carry({}), settings, sources: {}, staticPanels: { main: { at: seq } }, width: 120, height: 80, scrollLeft: 0, scrollTop: 0, theme: TEST_THEME, reducedMotion: true },
      hour: 1, seek: 0, speed: 0, binding: { generation: 1, surfaces: [{ width: 120, height: 80, dpr: 1 }], offset: 0, pin: null }, inspect: null, now: 1,
    }, state, () => `c${seq++}`);
    const widthOf = (painted: ReturnType<typeof paint>) => (painted.fields.widths?.value as { widths: Record<string, number> }).widths.name!;
    const before = widthOf(paint());
    names.set('A', 'A country whose recorded name arrived later and is long');
    const after = widthOf(paint());
    assert.ok(after > before, `a name that arrives after the first measurement widens its column (${before} -> ${after})`);
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }

  const withAction: TableColumnSpec[] = [
    kinds[0]!,
    { key: 'name', label: 'Organisation', as: 'plain', offered: ['plain'], priority: 0, text: true, cellKind: 'item' },
    { key: 'size', label: 'Size', as: 'plain', offered: ['plain'], priority: 1, numeric: true },
    { key: 'spare', label: '', as: 'plain', offered: ['plain'], priority: 2 },
    { key: 'view', label: '', as: 'plain', offered: ['plain'], priority: 3, cellKind: 'action', sortable: false },
  ];
  const named = 'Army Ordnance Department';
  const tipRows = resolveTableRows({ hour: 1, columns: withAction, rows: [{
    key: 'org', name: named, tooltipAction: 'Open its organisation window',
    cells: { pic: { value: knownValue(named), iconSlot: true }, name: { value: knownValue(named) }, size: { value: zeroValue(0) }, spare: { value: knownValue('stray') }, view: { value: knownValue('View'), actionLabel: 'View' } },
  }] }, tableSettingsOf(undefined, withAction, { density: 'normal', sort: { column: 'size', direction: 'descending' } }));
  const tip = deriveTableTooltip(tipRows, 'org', withAction)!;
  assert.equal(tip.title, named, 'the name column, not the picture before it, is the title');
  assert.deepEqual(tip.rows.map(row => row.label), ['Size'], 'no unlabelled row, no repeated name, no action word');
  assert.equal(tip.note, 'Open its organisation window', 'an action column contributes only its hint');
});

test('tree rows align whether or not they can open: a level with openable rows gives every row of that level the chevron lane, children start under their parent\'s name', () => {
  const tree: TableColumnSpec[] = [
    { key: 'pic', label: '', as: 'plain', offered: ['plain'], priority: 0, cellKind: 'kind', sortable: false },
    { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, cellKind: 'item' },
  ];
  const settings = tableSettingsOf(undefined, tree, { density: 'normal', sort: { column: 'name', direction: 'ascending' } });
  const leaf = (key: string, children?: TableRow[]): TableRow => ({ key, name: key, cells: { name: { value: knownValue(key) } }, ...(children ? { children } : {}) });
  const rows = resolveTableRows({ hour: 1, columns: tree, rows: [leaf('A', [leaf('A1')]), leaf('B'), leaf('C', [leaf('C1', [leaf('C1a')]), leaf('C2')])] }, settings, { A: true, C: true, C1: true });
  const data = rows.filter((item): item is Extract<FlatTableRow, { kind: 'data' }> => item.kind === 'data');
  assert.deepEqual(data.map(item => [item.key, item.level, item.isTreeGroup]), [['A', 1, true], ['A1', 2, false], ['B', 1, false], ['C', 1, true], ['C1', 2, true], ['C1a', 3, false], ['C2', 2, false]]);
  const levels = treeLevelsOf(rows);
  assert.equal(treeLevelsOf(rows), levels, 'held per row list');
  const starts = Object.fromEntries(data.map(item => [item.key, treeTextStart(item.level, levels)]));
  assert.deepEqual(starts, { A: TREE_CHEVRON_LANE, A1: 2 * TREE_CHEVRON_LANE, B: TREE_CHEVRON_LANE, C: TREE_CHEVRON_LANE, C1: 2 * TREE_CHEVRON_LANE, C1a: 2 * TREE_CHEVRON_LANE, C2: 2 * TREE_CHEVRON_LANE });
  const flat = resolveTableRows({ hour: 1, columns: tree, rows: [leaf('A', [leaf('A1')]), leaf('B')] }, settings, { A: true });
  const flatLevels = treeLevelsOf(flat);
  assert.deepEqual(flat.map(item => item.kind === 'data' ? treeTextStart(item.level, flatLevels) : -1), [TREE_CHEVRON_LANE, TREE_CHEVRON_LANE, TREE_CHEVRON_LANE], 'a leaf, a group and a child of a two-level tree start at one x');
  assert.equal(treeTextStart(1, treeLevelsOf(resolveTableRows({ hour: 1, columns: tree, rows: [leaf('B')] }, settings))), 0, 'a flat table reserves no lane');
  const layout = layoutFamilyTable(buildColumnLayouts(tree, settings, { pic: 52, name: 200 }), 400);
  const nameX = layout.columns[0]!.width;
  assert.equal(hitFamilyTable(layout, flat, { x: nameX + 12, y: 10 }, 36, 0, 0, false, 0).toggle, true, 'the chevron opens its group');
  assert.equal(hitFamilyTable(layout, flat, { x: nameX + 12, y: 36 * 2 + 10 }, 36, 0, 0, false, 0).toggle, false, 'a leaf has no chevron to hit');
});

test('log capability: headerless widths, date-grouped boundaries, gap/refused/loading slots, in-place children, and fresh-paint equality after seek back, same-hour correction and reorder', () => {
  const DAY1 = 43_800_000 + 400 * 24;
  const DAY2 = DAY1 + 24;
  const DAY3 = DAY2 + 24;
  const logColumns: TableColumnSpec[] = [
    { key: 'date', label: 'Occurred On Date', as: 'plain', offered: ['plain'], priority: 0, cellKind: 'date', sortable: true },
    { key: 'kind', label: 'Category', as: 'plain', offered: ['plain'], priority: 1, cellKind: 'kind', sortable: false },
    { key: 'country', label: 'Origin Country', as: 'flag', offered: ['flag', 'flagName'], priority: 2, sortable: false },
    { key: 'item', label: 'Item', as: 'plain', offered: ['plain'], priority: 0, text: true, cellKind: 'item' },
    { key: 'cost', label: 'Cost', as: 'plain', offered: ['plain'], priority: 3, numeric: true, format: 'integer' },
    { key: 'state', label: 'Outcome State', as: 'plain', offered: ['plain'], priority: 4, cellKind: 'status' },
  ];
  const baseSettings = tableSettingsOf(undefined, logColumns, { density: 'normal', sort: { column: 'date', direction: 'descending' } });
  const baseCols = buildColumnLayouts(logColumns, baseSettings);
  const measure = (text: string) => text.length * 7;
  const headedWidths = measureFixedColumnWidths(baseCols, baseSettings, TEST_THEME, measure);
  const headerlessWidths = measureFixedColumnWidths(baseCols, baseSettings, TEST_THEME, measure, undefined, { hideHeader: true });
  assert.ok(headerlessWidths.date! < headedWidths.date!, 'headerless mode ignores hidden header label width on date column');
  assert.ok(headerlessWidths.country! < headedWidths.country!, 'headerless mode keeps flag-only country column compact');
  const grown = layoutFamilyTable(buildColumnLayouts(logColumns, baseSettings, headerlessWidths), 720);
  assert.ok(grown.columns.find(c => c.key === 'item')!.width > 300, 'item column absorbs spare width while date/kind/flag/status stay fixed');

  const mkLogRow = (
    key: string,
    hour: number,
    cost: number,
    opts: {
      live?: boolean;
      statusKind?: 'running' | 'waiting' | 'succeeded' | 'finished' | 'ended';
      statusLabel?: string;
      inlinePrefix?: string;
      inlineSubtext?: string;
      title?: string;
      children?: readonly TableRow[];
    } = {},
  ): TableRow => ({
    key,
    name: opts.title ?? key,
    rowHour: hour,
    ...(opts.live ? { live: true } : {}),
    ...(opts.children ? { children: opts.children } : {}),
    cells: {
      date: { value: knownValue(hour) },
      kind: { value: knownValue('dec'), iconSlot: true, artShape: 'square', artName: 'Decision' },
      country: { value: knownValue('GER'), flags: [{ tag: 'GER', art: PENDING_ART }] },
      item: {
        value: knownValue(opts.title ?? key),
        ...(opts.inlinePrefix ? { inlinePrefix: opts.inlinePrefix } : {}),
        ...(opts.inlineSubtext ? { inlineSubtext: opts.inlineSubtext } : {}),
      },
      cost: { value: knownValue(cost), format: 'integer', unitSuffix: 'PP' },
      state: opts.statusKind
        ? { value: knownValue(opts.statusLabel ?? opts.statusKind), statusKind: opts.statusKind }
        : { value: notRecorded('Outcome not recorded'), missingFigure: 'Outcome' },
    },
  });

  const childRow = mkLogRow('c1', DAY2 + 2, 10, { title: 'Delivery 1', statusKind: 'finished', statusLabel: 'Delivered' });
  const slottedModel: TableModel = {
    columns: logColumns,
    hour: DAY3 + 12,
    hideHeader: true,
    unsorted: true,
    dateGrouped: true,
    rows: [],
    slots: [
      { kind: 'data', row: mkLogRow('live0', DAY3 + 8, 50, { live: true, title: 'First live same day', statusKind: 'running', statusLabel: 'Running' }) },
      { kind: 'data', row: mkLogRow('live1', DAY3 + 5, 50, { live: true, title: 'Stop the Strikes', inlineSubtext: 'Mission', statusKind: 'running', statusLabel: 'Running', children: [childRow] }) },
      { kind: 'data', row: mkLogRow('h1', DAY2 + 10, 25, { title: 'Event \uFFFC choice', inlinePrefix: 'A', statusKind: 'succeeded', statusLabel: 'Succeeded' }) },
      { kind: 'data', row: mkLogRow('h2', DAY2 + 4, 15, { title: 'Same day exact', statusKind: 'finished', statusLabel: 'Finished' }) },
      { kind: 'data', row: mkLogRow('h3', DAY2 + 2, 5, { title: 'Same day again', statusKind: 'ended', statusLabel: 'Ended' }) },
      { kind: 'gap', key: 'gap:1', firstHour: DAY1 + 6, lastHour: DAY2 - 1, reason: 'Collector detached' },
      { kind: 'data', row: mkLogRow('h4', DAY1 + 3, 100, { title: 'Before gap' }) },
      { kind: 'refused', key: 'ref:1', reason: 'Block damaged' },
      { kind: 'loading', key: 'load:1' },
    ],
  };

  const flatExpanded = resolveTableRows(slottedModel, baseSettings, { live1: true });
  assert.deepEqual(
    flatExpanded.map(item => (item.kind === 'data'
      ? [item.kind, item.key, item.level, item.showDate !== false, item.isDayEnd === true, item.isLiveEnd === true]
      : [item.kind, item.key])),
    [
      ['data', 'live0', 1, true, false, false],
      ['data', 'live1', 1, false, false, false],
      ['data', 'c1', 2, true, false, true],
      ['data', 'h1', 1, true, false, false],
      ['data', 'h2', 1, false, false, false],
      ['data', 'h3', 1, false, true, false],
      ['gap', 'gap:1'],
      ['data', 'h4', 1, true, true, false],
      ['refused', 'ref:1'],
      ['loading', 'load:1'],
    ],
    'same-day live rows suppress repeat date, live block band sits after its last open child, same-day exact repeats hide date while bounded dates stay visible, and gap/refused/loading slots preserve order',
  );

  const geom = computeRowLayout(flatExpanded, 'normal', 'standard');
  const queue = Array.from({ length: 120 }, (_, i) => ({ kind: 'data', key: `q${i}`, level: i % 4 === 0 ? 1 : 2, isLiveEnd: false, row: {} }) as unknown as FlatTableRow);
  const queueGeom = computeRowLayout(queue, 'comfortable', 'standard');
  const deep = tableRowWindow(queue.length, queueGeom, ROW_HEIGHT.comfortable, queueGeom.tops[100]!, 200);
  assert.equal(deep.first + TABLE_WINDOW_OVERSCAN, 100, 'deep in an expanded queue the published window starts at the row under the scroll position, not at scrollTop / 44');
  assert.ok(queueGeom.tops[deep.first + deep.count - TABLE_WINDOW_OVERSCAN - 1]! < queueGeom.tops[100]! + 200, 'and ends at the last row the viewport shows');
  const gapTop = geom.tops[6]!;
  assert.deepEqual(hitFamilyTable(grown, flatExpanded, { x: 40, y: gapTop + 10 }, 36, 0, 0, false, 0, undefined, undefined, undefined, geom), { kind: 'row', rowKey: 'gap:1' });
  const gapTip = deriveTableTooltip(flatExpanded, 'gap:1', logColumns);
  assert.equal(gapTip?.title, 'Not recorded');
  assert.equal(gapTip?.detail, 'Collector detached');
  const audit = deriveTableAudit({ model: slottedModel, flatRows: flatExpanded, visibleColumns: grown.columns });
  assert.ok(audit.some(r => r.key === 'gap:1:gap'));
  assert.ok(audit.some(r => r.key === 'ref:1:refused'));
  assert.ok(audit.some(r => r.key === 'load:1:loading'));

  const gappedModel: TableModel = {
    columns: logColumns,
    hour: DAY2 + 5,
    hideHeader: true,
    unsorted: true,
    dateGrouped: true,
    rows: [
      mkLogRow('live0', DAY2 + 4, 20, { live: true, statusKind: 'waiting', statusLabel: 'Waiting' }),
      mkLogRow('after', DAY2 + 1, 10, { statusKind: 'finished', statusLabel: 'Finished' }),
      mkLogRow('before', DAY1 + 1, 10, { statusKind: 'finished', statusLabel: 'Finished' }),
    ],
    gaps: [{ firstHour: DAY1 + 12, lastHour: DAY3, reason: 'Unrecorded stretch' }],
  };
  const gappedFlat = resolveTableRows(gappedModel, baseSettings);
  assert.deepEqual(gappedFlat.map(r => r.kind === 'gap' ? `gap:${r.firstHour}-${r.lastHour}` : r.key), [
    'live0',
    `gap:${DAY1 + 12}-${DAY2 + 5}`,
    'after',
    'before',
  ], 'interleaveLogGaps clamps gaps to model.hour and never splits the live block');

  const sortByCost = tableSettingsOf(undefined, logColumns, { density: 'normal', sort: { column: 'cost', direction: 'descending' } });
  const costSorted = resolveTableRows({
    columns: logColumns,
    hour: DAY3,
    dateGrouped: true,
    rows: [
      mkLogRow('a', DAY2 + 8, 10, { statusKind: 'finished' }),
      mkLogRow('b', DAY2 + 4, 40, { statusKind: 'finished' }),
    ],
  }, sortByCost);
  assert.deepEqual(costSorted.map(r => r.kind === 'data' && r.showDate !== false), [true, true], 'sorting by a non-date figure shows every row date');

  type LogScene = {
    readonly rows: readonly TableRow[];
    readonly slots?: readonly TableModelSlot[];
    readonly unsorted?: boolean;
  };
  registerFamilyProjection({
    kind: 'test-log-fresh-equals-incremental',
    family: 'table', theme: TEST_THEME,
    project: readView => {
      const reply = readView.panel('main') as LogScene;
      return {
        hour: readView.hour,
        hideHeader: true,
        dateGrouped: true,
        unsorted: reply.unsorted,
        columns: logColumns,
        rows: reply.rows,
        ...(reply.slots ? { slots: reply.slots } : {}),
      };
    },
  });

  const makeRichGrid = (width: number, height: number) => {
    const base = gridContext(width, height);
    const hPixels = new Map<string, string>();
    const hatchBoxes = new Map<string, string>();
    const canvasBox: readonly [number, number, number, number] = [0, 0, width, height];
    let clipBox: readonly [number, number, number, number] = canvasBox;
    let pending: readonly [number, number, number, number] | null = null;
    const clipStack: (readonly [number, number, number, number])[] = [];
    let moveX = 0;
    let moveY = 0;
    let lineX = 0;
    let lineY = 0;
    let hasLine = false;
    const raw = base.ctx as Record<string, unknown>;
    const wrapped = new Proxy(raw, {
      get(target, prop) {
        if (prop === 'save') {
          const orig = target.save as () => void;
          return () => { clipStack.push(clipBox); orig(); };
        }
        if (prop === 'restore') {
          const orig = target.restore as () => void;
          return () => { clipBox = clipStack.pop() ?? canvasBox; orig(); };
        }
        if (prop === 'beginPath') {
          const orig = target.beginPath as () => void;
          return () => { pending = null; orig(); };
        }
        if (prop === 'rect' || prop === 'roundRect') {
          const orig = target.rect as (x: number, y: number, w: number, h: number) => void;
          return (x: number, y: number, w: number, h: number) => {
            pending = [x, y, x + w, y + h];
            if (prop === 'rect') orig(x, y, w, h);
          };
        }
        if (prop === 'clip') {
          const orig = target.clip as () => void;
          return () => {
            if (pending) {
              clipBox = [Math.max(clipBox[0], pending[0]), Math.max(clipBox[1], pending[1]), Math.min(clipBox[2], pending[2]), Math.min(clipBox[3], pending[3])];
            }
            orig();
          };
        }
        if (prop === 'moveTo') return (x: number, y: number) => { moveX = x; moveY = y; hasLine = false; };
        if (prop === 'lineTo') return (x: number, y: number) => { lineX = x; lineY = y; hasLine = true; };
        if (prop === 'stroke') {
          return () => {
            if (hasLine) {
              const y0 = Math.floor(moveY);
              const y1 = Math.floor(lineY);
              if (y0 === y1 && y0 >= clipBox[1] && y0 < clipBox[3]) {
                const xStart = Math.max(Math.ceil(clipBox[0]), Math.round(Math.min(moveX, lineX)));
                const xEnd = Math.min(Math.floor(clipBox[2]), Math.round(Math.max(moveX, lineX)));
                for (let px = xStart; px < xEnd; px++) hPixels.set(`${px},${y0}`, String(target.strokeStyle));
              } else if (y0 !== y1) {
                hatchBoxes.set(`${Math.round(clipBox[0])},${Math.round(clipBox[1])},${Math.round(clipBox[2])},${Math.round(clipBox[3])}`, String(target.strokeStyle));
              }
              hasLine = false;
            }
          };
        }
        if (prop === 'fillRect' || prop === 'clearRect') {
          const orig = target[prop] as (x: number, y: number, w: number, h: number) => void;
          return (x: number, y: number, w: number, h: number) => {
            orig(x, y, w, h);
            const x0 = Math.max(clipBox[0], x);
            const y0 = Math.max(clipBox[1], y);
            const x1 = Math.min(clipBox[2], x + w);
            const y1 = Math.min(clipBox[3], y + h);
            for (const key of [...hPixels.keys()]) {
              const [px, py] = key.split(',').map(Number) as [number, number];
              if (px >= x0 && px < x1 && py >= y0 && py < y1) hPixels.delete(key);
            }
            for (const key of [...hatchBoxes.keys()]) {
              const [bx0, by0, bx1, by1] = key.split(',').map(Number) as [number, number, number, number];
              if (bx1 > x0 && bx0 < x1 && by1 > y0 && by0 < y1) hatchBoxes.delete(key);
            }
          };
        }
        return target[prop as string];
      },
    });
    return { ctx: wrapped, snapshot: () => ({ marks: new Map(base.marks), hPixels: new Map(hPixels), hatchBoxes: new Map(hatchBoxes) }) };
  };

  const grids: ReturnType<typeof makeRichGrid>[] = [];
  const prevOffscreen = globalThis.OffscreenCanvas;
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = class {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { const g = makeRichGrid(this.width, this.height); grids.push(g); return g.ctx; }
    transferToImageBitmap() { return { close() {} }; }
  };
  try {
    const createLane = () => {
      const view = createLaneView(tableLane, () => {});
      const state = new Map<string, unknown>();
      let seq = 1;
      const idx = grids.length;
      const transport = new LaneTransport();
      const paint = (scene: LogScene, hour: number, settings: TableSettings, expanded?: Readonly<Record<string, boolean>>) => view.paint({
        widget: 1,
        subs: [],
        pictures: [],
        model: transport.carry({
          family: 'table' as const,
          kind: 'test-log-fresh-equals-incremental',
          options: NO_OPTIONS,
          settings,
          sources: NO_SOURCES,
          staticPanels: { main: scene },
          width: 640,
          height: 360,
          scrollLeft: 0,
          scrollTop: 0,
          expanded,
          theme: TEST_THEME,
          reducedMotion: true,
        }),
        hour,
        seek: 0, speed: 24,
        binding: { generation: 1, surfaces: [{ width: 640, height: 360, dpr: 1 }], offset: 0, pin: null },
        inspect: null,
        now: 5000,
      }, state, () => `c${seq++}`);
      return { paint, snap: () => grids[idx]!.snapshot() };
    };
    const freshSnap = (scene: LogScene, hour: number, settings: TableSettings, expanded?: Readonly<Record<string, boolean>>) => {
      const other = createLane();
      other.paint(scene, hour, settings, expanded);
      return other.snap();
    };

    const uniformAt1: LogScene = {
      unsorted: true,
      rows: [
        mkLogRow('e2', DAY2 + 8, 50, { title: 'Event Two', inlineSubtext: 'Political', statusKind: 'finished', statusLabel: 'Finished' }),
        mkLogRow('e1', DAY2 + 2, 25, { title: 'Event \uFFFC One', inlinePrefix: 'A', statusKind: 'succeeded', statusLabel: 'Succeeded' }),
        mkLogRow('e0', DAY1 + 6, 10, { title: 'Earlier Day', statusKind: 'ended', statusLabel: 'Ended' }),
      ],
    };
    const uniformAt2: LogScene = {
      unsorted: true,
      rows: [
        mkLogRow('e3', DAY2 + 12, 75, { title: 'Event Three', statusKind: 'finished', statusLabel: 'Finished' }),
        ...uniformAt1.rows,
      ],
    };
    const correctedAt1: LogScene = {
      unsorted: true,
      rows: [
        mkLogRow('e2', DAY2 + 8, 50, { title: 'Event Two', inlineSubtext: 'Revised', statusKind: 'succeeded', statusLabel: 'Succeeded' }),
        mkLogRow('e1', DAY2 + 2, 25, { title: 'Event \uFFFC One', inlinePrefix: 'B', statusKind: 'ended', statusLabel: 'Ended' }),
        mkLogRow('e0', DAY1 + 6, 10, { title: 'Earlier Day', statusKind: 'ended', statusLabel: 'Ended' }),
      ],
    };
    const reorderScene: LogScene = {
      unsorted: false,
      rows: correctedAt1.rows,
    };

    const played = createLane();
    played.paint(uniformAt1, DAY2 + 9, baseSettings);
    played.paint(uniformAt2, DAY2 + 13, baseSettings);
    assert.deepEqual(played.snap(), freshSnap(uniformAt2, DAY2 + 13, baseSettings), 'forward log step shifting repeat-date suppression equals fresh paint');

    played.paint(uniformAt1, DAY2 + 9, baseSettings);
    assert.deepEqual(played.snap(), freshSnap(uniformAt1, DAY2 + 9, baseSettings), 'log seek back restores dates and hairlines identically to fresh paint');

    played.paint(correctedAt1, DAY2 + 9, baseSettings);
    assert.deepEqual(played.snap(), freshSnap(correctedAt1, DAY2 + 9, baseSettings), 'same-hour log correction (inlinePrefix, inlineSubtext, statusKind) equals fresh paint');

    played.paint(reorderScene, DAY2 + 9, sortByCost);
    assert.deepEqual(played.snap(), freshSnap(reorderScene, DAY2 + 9, sortByCost), 'reordering log by figure restores every row date identically to fresh paint');

    const slottedScene: LogScene = { unsorted: true, rows: [], slots: slottedModel.slots };
    played.paint(slottedScene, DAY3 + 12, baseSettings, { live1: true });
    assert.deepEqual(played.snap(), freshSnap(slottedScene, DAY3 + 12, baseSettings, { live1: true }), 'slotted log with live block, expanded children, gap, refused, and loading rows equals fresh paint');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }
});

test('the table host adapter stores and resolves table settings through its declaration policy and sizes media rows', () => {
  const columns = [{ key: 'a', label: 'A', priority: 1, as: 'number', offered: ['number'], width: 80 }, { key: 'b', label: 'B', priority: 2, as: 'number', offered: ['number'], width: 80 }] as never;
  const declaration = { family: 'table', kind: 'synthetic-settings', columns, settings: { kind: 'table', base: { density: 'normal', barsBehind: 'all', sort: { column: 'a', direction: 'descending' } } } };
  const policy = tableHostAdapter.settings!;
  const initial = policy.resolve(declaration, null, {});
  assert.equal(initial.sort.column, 'a');
  const stored: Record<string, unknown> = {};
  const next: TableSettings = { ...initial, density: 'compact', sort: { column: 'b', direction: 'ascending' } };
  policy.store!(declaration, next, initial, {} as never, (key, value) => { stored[key] = value; });
  const round = policy.resolve(declaration, null, stored);
  assert.deepEqual([round.density, round.sort.column, round.sort.direction], ['compact', 'b', 'ascending'], 'what the header stores is what the table resolves next');
  assert.equal(policy.editable!(declaration), true);
  assert.equal(policy.editable!({ ...declaration, settings: { kind: 'fixed', base: declaration.settings.base } }), false);

  assert.deepEqual(tableHostAdapter.getScrollExtent!({ widths: { widths: {}, totalWidth: 640 }, table: { count: 10, totalHeight: 280 } }), { width: 640, height: 280 });

  const ranked: TableModel = { columns: [{ key: 'a', label: 'A', as: 'number', offered: ['number'], numeric: true }] as never, hour: 5, rank: true, rows: [{ key: 'GER', name: 'Germany', cells: { a: { value: { kind: 'known', value: 3 } } } }] as never };
  const auditCtx = { viewport: { width: 320, height: 200, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null }, settings: round, expanded: {}, theme: undefined, fields: {} };
  const rankOf = (settings: TableSettings) => JSON.parse(tableHostAdapter.deriveAuditRows!(ranked, 5, { ...auditCtx, settings }).find(row => row.key === 'GER:_row')!.value!).rank;
  assert.equal(tableRank(ranked, { ...round, showRank: false }).shown, false);
  assert.equal(rankOf({ ...round, showRank: false }), null, 'ranks hidden on the canvas are absent from the exact audit too');
  assert.notEqual(rankOf({ ...round, showRank: true }), null, 'and present when shown');
  assert.deepEqual(tableHostAdapter.parseHit!('toggle:GER'), { key: 'GER', toggle: true, inspectToken: 'row:GER' }, 'hovering a tree chevron inspects its row, so the row tooltip shows');
});

test('batch 1 log projections: market-ledger contract grouping, decisions running-first, event-choices awaiting-first and detail meta, and event-feed missing dated faction name', () => {
  const marketReply: MarketLedgerTailReply = {
    branchId: '1',
    offset: '0',
    total: '2',
    live: '1',
    counts: [{ key: 'active', total: '1' }],
    nextChange: null,
    rows: [
      {
        key: '0000000000000012',
        hour: 100,
        observed: 100,
        exact: true,
        ordinal: '0',
        live: true,
        request: false,
        requestedHour: null,
        startedHour: 100,
        endedHour: null,
        status: 'active',
        seller: 'GER',
        sellerNotRecorded: false,
        buyer: 'FRA',
        buyerNotRecorded: false,
        items: [{
          variant: '6',
          name: 'Infantry Equipment I',
          nameMissing: null,
          icon: null,
          deliveredRaw: '25000000',
          contractedRaw: '50000000',
          deliveryUnavailable: null,
        }],
        deliveryMissing: null,
        paidRaw: '4800000',
        paidMissing: null,
        quotes: [{ variant: '6', name: 'Infantry Equipment I', nameMissing: null, priceRaw: '9600000' }],
        termsMissing: null,
        eventsTotal: '2',
        events: [
          { key: '3', hour: 108, kind: 'payment', variant: null, equipment: null, equipmentMissing: null, amountRaw: '4800000', amountNotRecorded: false, reason: null },
          { key: '2', hour: 105, kind: 'delivery', variant: '6', equipment: 'Infantry Equipment I', equipmentMissing: null, amountRaw: '25000000', amountNotRecorded: false, reason: null },
        ],
      },
      {
        key: '0000000000000099',
        hour: 100,
        observed: 100,
        exact: true,
        ordinal: '1',
        live: false,
        request: false,
        requestedHour: null,
        startedHour: 100,
        endedHour: 102,
        status: 'ended',
        seller: null,
        sellerNotRecorded: true,
        buyer: 'FRA',
        buyerNotRecorded: false,
        items: [{
          variant: '7',
          name: null,
          nameMissing: 'Design not named',
          icon: null,
          deliveredRaw: null,
          contractedRaw: null,
          deliveryUnavailable: 'Delivery amount unavailable: purchase manifest not acquired before removal',
        }],
        deliveryMissing: null,
        paidRaw: null,
        paidMissing: null,
        quotes: [],
        termsMissing: "The contract's terms were not recorded at its start",
        eventsTotal: '0',
        events: [],
      },
    ],
    missing: [],
    warnings: [],
  };
  const marketValidated = validateGenericTailReply(marketReply, 110);
  assert.equal(marketValidated.total, 2);
  const marketSource = buildRowSourceFromHead('mkt', marketReply, undefined, 110)!;
  const marketModel = marketLedgerProjection.project(createBoundedReadView({ hour: 110, rows: new Map([['main', marketSource]]) }), {
    perspective: 'FRA',
    selectionEmpty: null,
  });
  assert.equal(marketModel.rows.length, 2);
  assert.equal(marketModel.rows[0]!.cells.status.statusKind, 'running');
  assert.equal(marketModel.rows[0]!.cells.delivered.value.kind, 'known');
  assert.equal(marketModel.rows[0]!.cells.delivered.value.kind === 'known' ? marketModel.rows[0]!.cells.delivered.value.value : null, '250 / 500');
  assert.equal(marketModel.rows[0]!.cells.paid.value.kind === 'known' ? marketModel.rows[0]!.cells.paid.value.value : null, '\u221248');
  assert.equal(marketModel.rows[0]!.children?.length, 2, 'deliveries and payments remain separate child rows');
  assert.equal(marketModel.rows[0]!.children?.[0]!.cells.delivered.value.kind, 'notApplicable', 'payment child row never fabricates delivery count');
  assert.equal(marketModel.rows[0]!.children?.[1]!.cells.paid.value.kind, 'notApplicable', 'delivery child row never pairs payment');
  assert.equal(marketModel.rows[1]!.cells.delivered.value.kind, 'notRecorded', 'unavailable delivery stays notRecorded');
  assert.equal(marketModel.rows[1]!.cells.item.value.kind, 'notRecorded', 'unnamed design stays notRecorded');

  const decRows: ScriptPageRow[] = [
    {
      key: 'd-run',
      hour: 104,
      observed: 104,
      ordinal: '0',
      live: true,
      status: 'active',
      occurrence: {
        id: 'd-run',
        opened: true,
        edges: [{
          row: { id: 'd-run-1', hour: 104, readHour: 104, nameHour: 104, exact: true, category: 'politics', kind: 'decisionSelected', tags: ['GER'], participants: ['GER', null], item: { token: 'dec_two', name: 'Mobilize', description: null, icon: null }, amount: null },
          costs: null,
          lifecycle: null,
          days: 10,
          option: null,
        }],
      },
    },
    {
      key: 'd-done',
      hour: 108,
      observed: 108,
      ordinal: '1',
      live: false,
      status: 'taken',
      occurrence: {
        id: 'd-done',
        opened: true,
        edges: [{
          row: { id: 'd-done-1', hour: 108, readHour: 108, nameHour: 108, exact: true, category: 'politics', kind: 'decisionSelected', tags: ['GER'], participants: ['GER', null], item: { token: 'dec_one', name: 'War Bonds', description: null, icon: null }, amount: null },
          costs: [{ currency: 'politicalPower', amount: '-50000' }],
          lifecycle: null,
          days: null,
          option: null,
        }],
      },
    },
  ];
  const decSource = buildRowSourceFromHead('dec', { baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '2', rows: decRows, missing: [] }, undefined, 110)!;
  const decModel = decisionsProjection.project(createBoundedReadView({ hour: 110, rows: new Map([['main', decSource]]) }), {
    artwork: true,
    breaks: [],
  });
  assert.equal(decModel.rows.length, 2);
  assert.equal(decModel.rows[0]!.key, 'd-run', 'the product\'s order: the running decision first');
  assert.equal(decModel.rows[0]!.live, true);
  assert.equal(decModel.rows[0]!.cells.cost.value.kind, 'notRecorded');
  assert.equal(decModel.rows[1]!.key, 'd-done');
  assert.equal(decModel.rows[1]!.cells.cost.value.kind, 'known');

  const choiceRows: ScriptPageRow[] = [
    {
      key: 'ev-open',
      hour: 105,
      observed: 105,
      ordinal: '0',
      live: true,
      status: 'awaiting',
      occurrence: {
        id: 'ev-open',
        opened: true,
        edges: [{
          row: { id: 'ev-o-1', hour: 105, readHour: 105, nameHour: 105, exact: true, category: 'politics', kind: 'newsEventFired', tags: ['FRA'], participants: [null, 'FRA'], item: { token: 'news.2', name: 'Crisis', description: null, icon: null }, amount: null },
          costs: null,
          lifecycle: null,
          days: null,
          option: null,
        }],
      },
    },
    {
      key: 'ev-chosen',
      hour: 109,
      observed: 107,
      ordinal: '1',
      live: false,
      status: 'chosen',
      occurrence: {
        id: 'ev-chosen',
        opened: true,
        edges: [
          {
            row: { id: 'ev-c-1', hour: 107, readHour: 107, nameHour: 107, exact: true, category: 'politics', kind: 'scriptEventFired', tags: ['GER'], participants: [null, 'GER'], item: { token: 'ev.1', name: 'Rhineland', description: 'Text', icon: null, options: [{ ordinal: 0, name: 'Proceed' }] }, amount: null },
            costs: null,
            lifecycle: null,
            days: null,
            option: null,
          },
          {
            row: { id: 'ev-c-2', hour: 109, readHour: 109, nameHour: 109, exact: true, category: 'politics', kind: 'eventOptionSelected', tags: ['GER'], participants: ['GER', null], item: null, amount: null },
            costs: null,
            lifecycle: null,
            days: null,
            option: 0,
          },
        ],
      },
    },
  ];
  const choiceSource = buildRowSourceFromHead('evc', { baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '2', rows: choiceRows, missing: [] }, undefined, 110)!;
  const choiceModel = eventChoicesProjection.project(createBoundedReadView({ hour: 110, rows: new Map([['main', choiceSource]]) }), {
    events: ['country', 'news'],
    awaiting: false,
    artwork: true,
    breaks: [],
  });
  assert.equal(choiceModel.rows.length, 2);
  assert.equal(choiceModel.rows[0]!.key, 'ev-open', 'the product\'s order: the awaiting event first');
  assert.equal(choiceModel.rows[0]!.cells.choice.statusKind, 'waiting');
  assert.equal(choiceModel.rows[0]!.cells.action.actionLabel, 'View');
  assert.equal(choiceModel.rows[1]!.cells.choice.inlinePrefix, 'A');
  assert.ok(choiceModel.rows[1]!.meta, 'chosen event carries detail window subject');

  const feedRows: EventFeedPageRow[] = [
    {
      key: 'f-1',
      hour: 106,
      observed: 106,
      ordinal: '0',
      live: false,
      choice: null,
      row: {
        id: 'f-1',
        hour: 106,
        readHour: 106,
        nameHour: 106,
        exact: false,
        category: 'diplomacy',
        kind: 'factionJoined',
        tags: ['GER'],
        item: { token: 'faction:1', name: null, nameMissing: 'Faction name not recorded at this hour', description: null, icon: null },
        amount: null,
      },
    },
  ];
  const feedSource = buildRowSourceFromHead('feed', { baseOrdinal: '0', firstOrdinal: '0', offset: '0', total: '1', rows: feedRows, missing: [] }, undefined, 110)!;
  const feedModel = eventFeedProjection.project(createBoundedReadView({ hour: 110, rows: new Map([['main', feedSource]]) }), {
    categories: ['diplomacy'],
    artwork: true,
  });
  assert.equal(feedModel.rows.length, 1);
  assert.equal(feedModel.rows[0]!.cells.detail.value.kind, 'notRecorded', 'missing dated faction name stays notRecorded rather than guessing');
});

test('the audit compares which art a cell shows, never whether its bytes have arrived', () => {
  const settings = tableSettingsOf(undefined, CLOCK_COLUMNS, CLOCK_TABLE_BASE);
  const layouts = buildColumnLayouts(CLOCK_COLUMNS, settings);
  const base = campaignClockProjection.project(createBoundedReadView({ hour: 50, panels: new Map([['main', {
    available: true, recordedSeconds: 3661, playingSeconds: 2400, pausedSeconds: 1261, complete: true, warnings: [],
  } as CampaignClockReply]]) }), {});
  const column = CLOCK_COLUMNS[0]!.key;
  const withArt = (art: ArtResult, artKey = JSON.stringify(['sprite', 'GFX_counter', 0, null, false])): TableModel => ({
    ...base,
    rows: base.rows.map(row => ({ ...row, cells: { ...row.cells, [column]: { ...row.cells[column]!, iconSlot: true, artKey, art } } })),
  });
  const audit = (model: TableModel) => deriveTableAudit({ model, flatRows: resolveTableRows(model, settings), visibleColumns: layouts });
  const arrived = audit(withArt(readyArt('http://artwork.localhost/0123456789abcdef0123456789abcdef.png')));
  const arriving = audit(withArt(PENDING_ART));
  assert.equal(pictureError('table', arrived, { values: new Map(arriving.map(row => [row.key, row.value!])), ordered: true }), null);
  const other = audit(withArt(PENDING_ART, JSON.stringify(['sprite', 'GFX_other', 0, null, false])));
  assert.notEqual(pictureError('table', arrived, { values: new Map(other.map(row => [row.key, row.value!])), ordered: true }), null, 'a different picture is still a difference');
});

test('cold table art and flag slots waiting behind the fetch concurrency limit paint as pending, not confirmed missing', () => {
  const artCache = new LaneArtCache({
    maxConcurrent: 6,
    fetchBitmap: () => new Promise<ImageBitmap>(() => {}),
  });
  const columns: TableColumnSpec[] = [
    { key: 'country', label: 'Country', as: 'flagName', offered: ['flagName'], priority: 0, text: true, widest: ['Country 7'] },
    { key: 'badge', label: 'Kind', as: 'plain', offered: ['plain'], priority: 1, cellKind: 'kind', widest: ['Kind'] },
  ];
  const settings = tableSettingsOf(undefined, columns, { density: 'compact', sort: { column: 'country', direction: 'ascending' } });
  const baseCols = buildColumnLayouts(columns, settings);
  const fixedWidths = measureFixedColumnWidths(baseCols, settings, TEST_THEME, text => text.length * 7);
  const layout = layoutFamilyTable(buildColumnLayouts(columns, settings, fixedWidths), 320, { rankShown: false, rankWidth: 0 });

  const flagUrls = Array.from({ length: 7 }, (_, i) => `art://flag-${i + 1}`);
  const iconUrls = Array.from({ length: 7 }, (_, i) => `art://icon-${i + 1}`);
  artCache.requestVisible([...flagUrls, ...iconUrls]);
  assert.equal(artCache.stateOfUrl(flagUrls[0]!), 'pending');
  assert.equal(artCache.stateOfUrl(flagUrls[6]!), 'unrequested');
  assert.equal(artCache.stateOfUrl(iconUrls[6]!), 'unrequested');

  const rows = resolveTableRows({
    columns,
    hour: 10,
    rows: Array.from({ length: 7 }, (_, i) => ({
      key: `R${i + 1}`,
      name: `Country ${i + 1}`,
      cells: {
        country: {
          value: knownValue(`Country ${i + 1}`),
          country: `T0${i + 1}`,
          iconSlot: true,
          art: readyArt(flagUrls[i]!),
        },
        badge: {
          value: knownValue(`Icon ${i + 1}`),
          iconSlot: true,
          art: readyArt(iconUrls[i]!),
          artName: `Icon ${i + 1}`,
        },
      },
    })),
  }, settings);

  const texts: string[] = [];
  let roundRectCalls = 0;
  const base = instrumentedContext().ctx;
  const recordingCtx = {
    ...base,
    measureText(text: string) {
      return { width: text.length * 5 };
    },
    fillText(text: string) {
      texts.push(text);
    },
    roundRect() {
      roundRectCalls++;
    },
  } as unknown as CanvasRenderingContext2D;

  drawFamilyTable(recordingCtx, {
    layout,
    rows,
    settings,
    hour: 10,
    width: 320,
    height: 400,
    scrollLeft: 0,
    scrollTop: 0,
    rankShown: false,
    rankWidth: 0,
    theme: TEST_THEME,
    now: 1,
    glideDuration: 0,
    glide: NO_GLIDE,
    artCache,
    iconVersion: artCache.version,
    dpr: 1,
  });

  assert.equal(texts.includes('T07'), false, '7th queued flag waiting behind concurrency limit must not draw missing tag text');
  assert.equal(roundRectCalls, 0, 'queued unrequested art and flag slots must stay pending with no hatched missing box');

  const missingRows = resolveTableRows({
    columns,
    hour: 10,
    rows: [{
      key: 'M1',
      name: 'No Art',
      cells: {
        country: { value: knownValue('No Art'), country: 'MIS', iconSlot: true, art: productArt({ status: 'confirmedMissing', reason: 'no layer holds it' }) },
        badge: { value: knownValue('Missing'), iconSlot: true, artName: 'Missing', art: productArt({ status: 'confirmedMissing', reason: 'no layer holds it' }) },
      },
    }],
  }, settings);
  texts.length = 0;
  roundRectCalls = 0;
  drawFamilyTable(recordingCtx, {
    layout,
    rows: missingRows,
    settings,
    hour: 10,
    width: 320,
    height: 120,
    scrollLeft: 0,
    scrollTop: 0,
    rankShown: false,
    rankWidth: 0,
    theme: TEST_THEME,
    now: 2,
    glideDuration: 0,
    glide: NO_GLIDE,
    artCache,
    iconVersion: artCache.version,
    dpr: 1,
  });
  assert.ok(roundRectCalls > 0, 'confirmed-missing table art and flag slots paint the hatched missing box');
  assert.ok(texts.includes('MIS'), 'a confirmed-missing flag slot paints the country tag');
  artCache.dispose();
});

test('table partial and withheld notices live in the header summary marker, never a body note row, while mirror description and audit retain them', () => {
  const partialProj = declareTableProjection<{ count: number; detail: string }, { id: string }, Record<string, never>>({
    kind: 'test-header-partial-marker',
    columns: [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'], priority: 0, text: true, path: 'id' },
    ],
    rows: reply => ({
      rows: [{ id: 'Row1' }],
      titleSummary: `${reply.count} not shown`,
      titleSummaryPartial: `${reply.count} leaders of these countries can’t be shown; Details lists why.\n${reply.detail}`,
      warnings: [reply.detail],
    }),
    rowKey: r => r.id,
    rowName: r => r.id,
  });
  registerFamilyProjection(partialProj);
  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return { ...instrumentedContext().ctx, setTransform() {} }; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const laneView = createLaneView(tableLane, () => {});
    const laneState = new Map<string, unknown>();
    const transport = new LaneTransport();
    let seq = 1;
    const painted = laneView.paint({
      widget: 1,
      subs: [],
      pictures: [],
      model: transport.carry({
        family: 'table', theme: TEST_THEME,
        kind: 'test-header-partial-marker',
        options: NO_OPTIONS,
        settings: tableSettingsOf(undefined, [{ key: 'name', label: 'Name', as: 'plain', offered: ['plain'] }], { density: 'normal', sort: { column: 'name', direction: 'ascending' } }),
        sources: NO_SOURCES,
        staticPanels: { main: { count: 3, detail: 'Leader A: condition undecided' } },
        width: 320,
        height: 200,
        scrollLeft: 0,
        scrollTop: 0,
        audit: true,
      }),
      hour: 10,
      seek: 0, speed: 0,
      binding: { generation: 1, surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null },
      inspect: null,
      now: 100,
    }, laneState, () => `c${seq++}`);
    assert.equal(painted.mode, 'picture');
    const meta = painted.fields.meta!.value as { summaryText: string | null; summaryPartial: string | null; notes: readonly string[]; warnings: readonly string[] };
    assert.deepEqual(meta.notes, [], 'tables never emit a body note row');
    assert.equal(meta.summaryText, '3 not shown', 'the header marker carries the compact count');
    assert.equal(meta.summaryPartial, '3 leaders of these countries can’t be shown; Details lists why.\nLeader A: condition undecided', 'the header marker carries the full sentence and per-leader reasons');
    assert.deepEqual(meta.warnings, ['Leader A: condition undecided'], 'Details still lists the per-leader reasons');
    const nav = painted.fields.nav!.value as readonly (readonly unknown[])[];
    assert.equal(nav[0]![6], 0, 'the first row sits at y = 0 directly under the column headers');
    const desc = painted.fields.accessibleDescription?.value;
    assert.equal(typeof desc, 'string', 'accessible description is published for the partial summary');
    assert.ok((desc as string).includes('3 not shown') && (desc as string).includes('Leader A: condition undecided'));
    const audit = painted.fields.audit!.value as { id: number; rows: readonly { key: string; value?: string }[] };
    const metaAudit = audit.rows.find(row => row.key === '__table__:meta')?.value ?? '';
    assert.ok(metaAudit.includes('3 not shown') && metaAudit.includes('Leader A: condition undecided'), 'audit records the header marker and reasons');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }

  const occPartial = occupiedStatesProjection.project(
    createBoundedReadView({
      hour: 50,
      panels: new Map([['main', {
        hour: 50,
        complete: false,
        missing: null,
        strengthMissing: 'Not recorded',
        garrisonRedRaw: null,
        laws: [],
        states: [{
          state: 1,
          name: 'Alsace',
          nameMissing: null,
          owner: 'FRA',
          occupier: 'GER',
          occupied: 'FRA',
          law: null,
          lawMissing: 'Not recorded',
          garrisonRequiredSign: null,
          assignedKnown: false,
          garrisonMissing: 'Not recorded',
          garrisonAlert: false,
        }],
        warnings: [],
      }]]),
    }),
    { filter: { passing: null, unrecorded: [], bounds: [] }, role: 'occupier', lawAs: 'dot', filterAll: true },
  );
  assert.equal(occPartial.titleSummary, 'Partial');
  assert.equal(occPartial.titleSummaryPartial, 'Some occupied states are missing.');
  assert.equal('notes' in occPartial, false);
});

test('buildDeclarativeRow resolves country identity and flag art once per row across numeric columns and rowName', () => {
  type ScoreRow = { readonly tag: string; readonly army: number; readonly navy: number; readonly air: number; readonly industry: number; readonly total: number };
  const proj = declareTableProjection<{ readonly rows: readonly ScoreRow[] }, ScoreRow, Record<string, never>>({
    kind: 'test-declarative-identity-reuse',
    validate: raw => raw as { readonly rows: readonly ScoreRow[] },
    columns: () => [
      { key: 'country', label: 'Country', as: 'flagName', offered: ['flagName'], priority: 0, countryField: 'tag' },
      { key: 'army', label: 'Army', as: 'faint', offered: ['faint'], priority: 1, numeric: true, tone: 'country', country: 'tag', field: 'army' },
      { key: 'navy', label: 'Navy', as: 'faint', offered: ['faint'], priority: 2, numeric: true, tone: 'country', country: 'tag', field: 'navy' },
      { key: 'air', label: 'Air', as: 'faint', offered: ['faint'], priority: 3, numeric: true, tone: 'country', country: 'tag', field: 'air' },
      { key: 'industry', label: 'Industry', as: 'faint', offered: ['faint'], priority: 4, numeric: true, tone: 'country', country: 'tag', field: 'industry' },
      { key: 'total', label: 'Total', as: 'faint', offered: ['faint'], priority: 5, numeric: true, tone: 'country', country: 'tag', field: 'total' },
    ],
    rows: reply => ({ rows: reply.rows }),
    rowKey: r => r.tag,
    rowName: (r, hour, _o, who) => who(r.tag, hour).name,
  });
  let whoCalls = 0;
  let artCalls = 0;
  const view = createBoundedReadView({
    hour: 100,
    panels: new Map([['main', { rows: [{ tag: 'GER', army: 10, navy: 20, air: 30, industry: 40, total: 100 }] }]]),
    identity: {
      resolve(kind, key, atHour) {
        whoCalls++;
        return {
          kind,
          key,
          atHour,
          holder: { status: 'ready', value: key },
          tag: { status: 'ready', value: key },
          name: { status: 'ready', value: 'Germany' },
          colour: { status: 'ready', value: '#4a4a4a' },
          flagChoice: { status: 'ready', value: '["GER",null,null]' },
          validInterval: [0, 500],
          nextChangeAt: 501,
        };
      },
    },
    art: {
      resolve() {
        artCalls++;
        return { status: 'pending' };
      },
    },
  });
  const model = proj.project(view, {});
  assert.equal(model.rows.length, 1);
  assert.equal(model.rows[0]!.name, 'Germany');
  assert.equal(model.rows[0]!.cells.total!.countryColor, '#4a4a4a');
  assert.equal(whoCalls, 1, 'identity is resolved once per row at the same hour');
  assert.equal(artCalls, 1, 'flag art is resolved only for the country column, never numeric columns or rowName');
});
