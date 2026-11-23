import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message, type Spec } from '../../dashboard/follow/messages.fixture.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { tableSettingsOf, type TableSettings, type TableTheme } from './settings.ts';
import { nextLookChange } from './looks.ts';
import { familyTableLaneView } from './laneView.ts';
import { registerTableProjection } from './registry.ts';
import type { ApmReply, CampaignClockReply, DivisionTemplatesReply, ScoreboardColumn, ScoreboardReply } from '../../generated/dashboard';
import { pictureError } from '../../shell/perf.ts';
import { CLOCK_COLUMNS, CLOCK_TABLE_BASE, campaignClockProjection } from '../../widgets/campaign-clock/clockProjection.ts';
import { divisionTemplateColumns, divisionTemplateProjection } from '../../widgets/division-template/divisionTemplateProjection.ts';
import { gameScoreColumns, gameScoreProjection } from '../../widgets/game-score/gameScoreProjection.ts';
import { COLUMNS as OCCUPIED_DEFAULTS, TABLE_BASE as OCCUPIED_BASE } from '../../widgets/occupied-states/definition.ts';
import { occupiedStatesProjection } from '../../widgets/occupied-states/occupiedProjection.ts';
import { apmTableColumns, apmTableProjection } from '../../widgets/player-apm/apmProjection.ts';
import { TABLE_BASE as SCOREBOARD_BASE } from '../../widgets/scoreboard/definition.ts';
import { RANK_SOURCE, scoreboardColumnsSpec, scoreboardProjection } from '../../widgets/scoreboard/scoreboardProjection.ts';
import type { FamilySources } from '../contract.ts';
import { InvalidFamilyValueError, knownValue, notApplicable, notRecorded, partialValue } from '../value.ts';
import { buildColumnLayouts, layoutFamilyTable, measureFixedColumnWidths } from './layout.ts';
import { declareTableProjection, deriveTableAudit, deriveTableTooltip, resolveTableRows, type TableColumnSpec } from './model.ts';
import { createTablePicture, drawFamilyTable } from './painter.ts';
import { NOT_NAMED } from '../../widgets/occupied-states/occupiedStatesData.ts';

const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

const floatBits = (value: number): bigint => {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value, true);
  return view.getBigInt64(0, true);
};

const packTag = (tag: string): bigint => {
  let out = 0n;
  for (let i = tag.length - 1; i >= 0; i--) out = (out << 8n) | BigInt(tag.charCodeAt(i));
  return out;
};

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
  const sources: FamilySources = {
    answers: new Map(),
    panels: new Map([['main', { items: [{ id: 'a', count: 1 }] }]]),
    live: new Map(),
  };
  const model = broken.project(sources, 10, {});
  const settings = tableSettingsOf(undefined, model.columns, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'count', direction: 'descending' } });
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
  const followedModel = campaignClockProjection.project({ answers: new Map([['main', wire]]), panels: new Map(), live: new Map() }, 50, {});
  const exactModel = campaignClockProjection.project({ answers: new Map(), panels: new Map([['main', reply]]), live: new Map() }, 50, {});
  const followedAudit = deriveTableAudit(resolveTableRows(followedModel, settings), CLOCK_COLUMNS, 50, followedModel.empty);
  const exactAudit = deriveTableAudit(resolveTableRows(exactModel, settings), CLOCK_COLUMNS, 50, exactModel.empty);
  assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);
  assert.equal(followedAudit.find(r => r.key === 'total:time')?.value, '01:01:01');
});

test('scoreboard projection matches followed and exact audits with missing, partial, zero, and tied rows', () => {
  const columns: ScoreboardColumn[] = [
    { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null, family: { kind: 'f', measure: 'm' }, partial: null, noneAtZero: false },
    { key: 'casualties', label: 'Casualties', unit: 'people', decimals: 0, available: true, reason: null, family: { kind: 'c', measure: 't' }, partial: { kind: 'c', measure: 'p' }, noneAtZero: true },
  ];
  const looks = {
    GER: [{ from: 0, name: 'Germany' }],
    FRA: [{ from: 0, name: 'France' }],
    ITA: [{ from: 0, name: 'Italy' }],
    ENG: [{ from: 0, name: 'United Kingdom' }],
  };
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

  const options = { shown: columns, candidates: ['GER', 'FRA', 'ITA', 'ENG'], top: 4, rankMarkers: false, looks };
  const specs = scoreboardColumnsSpec(columns, 4);

  for (const direction of ['descending', 'ascending'] as const) {
    const settings: TableSettings = { ...tableSettingsOf(undefined, specs, SCOREBOARD_BASE), order: specs.map(s => s.key), sort: { column: 'factories', direction } };
    const followedModel = scoreboardProjection.project({
      answers: new Map([[RANK_SOURCE, rankAns], ['factories', facAns], ['casualties', casAns], ['casualties:partial', partAns]]),
      panels: new Map(),
      live: new Map(),
    }, 40, options);
    const exactModel = scoreboardProjection.project({
      answers: new Map(),
      panels: new Map([['main', exactReply]]),
      live: new Map(),
    }, 40, options);

    const followedFlat = resolveTableRows(followedModel, settings);
    const exactFlat = resolveTableRows(exactModel, settings);
    const followedAudit = deriveTableAudit(followedFlat, specs, 40, followedModel.empty);
    const exactAudit = deriveTableAudit(exactFlat, specs, 40, exactModel.empty);

    assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);
    assert.equal(followedFlat.at(-1)?.kind === 'data' && followedFlat.at(-1)?.key, 'ENG', `missing ENG sorts last in ${direction}`);
    const order = followedFlat.map(r => r.key);
    assert.ok(order.indexOf('FRA') < order.indexOf('ITA'), 'tied France precedes Italy by name A-Z regardless of wire vs reply input order');
    assert.equal(followedAudit.find(r => r.key === 'FRA:casualties')?.value, '0', 'known zero renders 0, never None');
    assert.equal(followedAudit.find(r => r.key === 'ITA:casualties')?.value, '500, partial: Some inputs were not recorded at this hour');
  }
});

test('apm-table projection matches followed wire answers and exact replies, including partial gaps and notApplicable zero minutes', () => {
  const wire = answer({
    keys: [1n, 2n],
    columns: 9,
    names: ['GER', 'ITA'],
    values: [
      floatBits(45.2), floatBits(45.2), 120n, 15n, floatBits(2.6), 1n, 1n, packTag('GER'), 100n,
      0n, 0n, 0n, 0n, floatBits(0), 1n, 0n, packTag('ITA'), 100n,
    ],
    states: [
      CELL.value, CELL.value, CELL.value, CELL.value, CELL.value, CELL.value, CELL.value, CELL.value, CELL.value,
      CELL.missing, CELL.missing, CELL.value, CELL.value, CELL.value, CELL.value, CELL.value, CELL.value, CELL.value,
    ],
  });
  const reply: ApmReply = {
    available: true,
    warnings: [],
    players: [],
    series: [
      { id: 'GER', name: 'GER', tag: 'GER', hour: 100, summary: { apm: 45.2, confirmedApm: 45.2, orders: '120', pausedOrders: '15', unpausedMinutes: 2.6, complete: true, gaps: true }, points: [] },
      { id: 'ITA', name: 'ITA', tag: 'ITA', hour: 100, summary: { apm: null, confirmedApm: null, orders: '0', pausedOrders: '0', unpausedMinutes: 0, complete: true, gaps: false }, points: [] },
    ],
  };
  const opts = {
    players: false,
    showPaused: true,
    cellBars: true,
    rankMarkers: false,
    playerNames: {},
    labels: { GER: [{ from: 0, name: 'Germany' }], ITA: [{ from: 0, name: 'Italy' }] },
  };
  const cols = apmTableColumns(false, true, true);
  const settings = tableSettingsOf(undefined, cols, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'apm', direction: 'descending' } });
  const fModel = apmTableProjection.project({ answers: new Map([['main', wire]]), panels: new Map(), live: new Map() }, 100, opts);
  const eModel = apmTableProjection.project({ answers: new Map(), panels: new Map([['main', reply]]), live: new Map() }, 100, opts);
  const fAudit = deriveTableAudit(resolveTableRows(fModel, settings), cols, 100, fModel.empty);
  const eAudit = deriveTableAudit(resolveTableRows(eModel, settings), cols, 100, eModel.empty);
  assert.equal(pictureError('table', fAudit, { values: new Map(eAudit.map(r => [r.key, r.value!])), ordered: true }), null);
  assert.ok(fAudit.find(r => r.key === 'GER:apm')?.value?.includes('partial:'));
  assert.ok(fAudit.find(r => r.key === 'ITA:apm')?.value?.startsWith('Not applicable:'));
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
  const occModel = occupiedStatesProjection.project({
    answers: new Map(),
    panels: new Map([['main', occReply]]),
    live: new Map([['main', { get: k => (k.endsWith('resistance') ? 2_500_000 : k.endsWith('compliance') ? 1_000_000 : k.endsWith('garrison') ? 500_000 : 100_000), missing: () => 'Not recorded' }]]),
  }, 50, {
    filter: { passing: null, unrecorded: [], bounds: [] },
    role: 'occupier',
    labels: { GER: [{ from: 0, name: 'Germany' }], POL: [{ from: 0, name: 'Poland' }] },
    lawAs: 'dot',
    filterAll: true,
  });
  const occSettings = tableSettingsOf(undefined, OCCUPIED_DEFAULTS, OCCUPIED_BASE);
  const occRows = resolveTableRows(occModel, occSettings);
  assert.equal(occRows.length, 1);
  assert.equal(deriveTableAudit(occRows, occModel.columns, 50).find(r => r.key === '1:resistance')?.value, '25%');

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
  const gsModel = gameScoreProjection.project({
    answers: new Map(),
    panels: new Map([['main', gsReply]]),
    live: new Map([['main', { get: k => (k.startsWith('GER') ? 300 : 200), missing: () => 'Not recorded' }]]),
  }, 50, {
    all: true,
    chosenTags: [],
    membership: null,
    top: null,
    by: 'total',
    factions: true,
    factionRuns: [
      { id: '1', tag: 'GER', firstHour: 0, lastHour: 100, names: [], fallbackName: 'Axis' },
      { id: '2', tag: 'ENG', firstHour: 0, lastHour: 100, names: [], fallbackName: 'Allies' },
    ],
    rankMarkers: false,
    cellBars: true,
    labels: { GER: [{ from: 0, name: 'Germany' }], ENG: [{ from: 0, name: 'United Kingdom' }] },
  });
  const gsCols = gameScoreColumns(undefined, true);
  const gsFlat = resolveTableRows(gsModel, tableSettingsOf(undefined, gsCols, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'total', direction: 'descending' } }));
  assert.deepEqual(gsFlat.filter(r => r.kind === 'group-header').map(r => r.label), ['Allies', 'Axis']);

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
  const dtCols = divisionTemplateColumns(true);
  const dtSettings = tableSettingsOf(undefined, dtCols, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'template', direction: 'ascending' } });
  const dtModel = divisionTemplateProjection.project({ answers: new Map(), panels: new Map([['main', dtReply]]), live: new Map([['main', repliedCells(dtReply).read()]]) }, 50, {
    countryName: 'Germany',
    cellBars: true,
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
  const settings = tableSettingsOf(undefined, columns, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'v1', direction: 'descending' } });
  const baseCols = buildColumnLayouts(columns, settings);
  const { ctx, counts, reset } = instrumentedContext();
  const fixedWidths = measureFixedColumnWidths(baseCols, settings, TEST_THEME, text => ctx.measureText(text).width);
  const layout = layoutFamilyTable(buildColumnLayouts(columns, settings, fixedWidths), 480, 'tooltip', { rankShown: false, rankWidth: 0 });
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
  const firstDraw = drawFamilyTable(ctx, { layout, rows: r1, settings, hour: 1, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 1, glideDuration: 0, glide: new Map(), markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, icons: new Map(), dpr: 1 }, picture);
  assert.equal(firstDraw.paintedCells, 15, 'initial paint draws all 5x3 visible cells');

  reset();
  const sameDraw = drawFamilyTable(ctx, { layout, rows: r1, settings, hour: 2, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 2, glideDuration: 0, glide: new Map(), markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, icons: new Map(), dpr: 1 }, picture);
  assert.equal(sameDraw.paintedCells, 0, 'unchanged frame paints zero cells');
  assert.equal(sameDraw.drew, false);
  assert.equal(counts().measureCalls, 0);

  const r2 = makeRows([[100, 1.5], [90, 2.5], [80, 0], [70, 0], [60, 9.9]]);
  reset();
  const oneCellDraw = drawFamilyTable(ctx, { layout, rows: r2, settings, hour: 3, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 3, glideDuration: 0, glide: new Map(), markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, icons: new Map(), dpr: 1 }, picture);
  assert.equal(oneCellDraw.paintedCells, 1, 'changing one cell without reordering repaints exactly 1 cell');

  const r3 = makeRows([[100, 1.5], [95, 2.5], [80, 0], [70, 0], [99, 9.9]]);
  reset();
  const swapDraw = drawFamilyTable(ctx, { layout, rows: r3, settings, hour: 4, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 4, glideDuration: 0, glide: new Map(), markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, icons: new Map(), dpr: 1 }, picture);
  assert.ok(swapDraw.paintedCells > 0 && swapDraw.paintedCells <= 15, `reorder repaints only dirty rows: ${swapDraw.paintedCells}`);

  reset();
  for (let frame = 0; frame < 60; frame++) {
    const stepRows = makeRows([[100, 1.5], [90, 2.5], [80, 0], [70, 0], [60, (frame % 5) + 0.5]]);
    drawFamilyTable(ctx, { layout, rows: stepRows, settings, hour: 10 + frame, width: 480, height: 220, scrollLeft: 0, scrollTop: 0, rankShown: false, rankWidth: 0, theme: TEST_THEME, now: 10 + frame, glideDuration: 0, glide: new Map(), markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, icons: new Map(), dpr: 1 }, picture);
  }
  assert.ok(counts().measureCalls <= 15, `text measurements stay bounded after warm-up: ${counts().measureCalls} across 60 frames`);
});

test('1,500 countries project, sort, audit identically, and paint only visible rows with bounded measurements', () => {
  const count = 1500;
  const tags = Array.from({ length: count }, (_, i) => `C${String(i).padStart(4, '0')}`);
  const looks = Object.fromEntries(tags.map(tag => [tag, [{ from: 0, name: `Country ${tag}` }]]));
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

  const options = { shown: columns, candidates: tags, top: null, rankMarkers: false, looks };
  const specs = scoreboardColumnsSpec(columns, null);
  const settings: TableSettings = { ...tableSettingsOf(undefined, specs, SCOREBOARD_BASE), order: specs.map(s => s.key), sort: { column: 'factories', direction: 'descending' } };
  const followedModel = scoreboardProjection.project({ answers: new Map([['factories', wire]]), panels: new Map(), live: new Map() }, 200, options);
  const exactModel = scoreboardProjection.project({ answers: new Map(), panels: new Map([['main', reply]]), live: new Map() }, 200, options);
  const followedRows = resolveTableRows(followedModel, settings);
  const exactRows = resolveTableRows(exactModel, settings);
  assert.equal(followedRows.length, count);

  const followedAudit = deriveTableAudit(followedRows, specs, 200);
  const exactAudit = deriveTableAudit(exactRows, specs, 200);
  assert.equal(pictureError('table', followedAudit, { values: new Map(exactAudit.map(r => [r.key, r.value!])), ordered: true }), null);

  const { ctx, counts, reset } = instrumentedContext();
  const baseCols = buildColumnLayouts(specs, settings);
  const fixedWidths = measureFixedColumnWidths(baseCols, settings, TEST_THEME, text => ctx.measureText(text).width);
  const layout = layoutFamilyTable(buildColumnLayouts(specs, settings, fixedWidths), 480, 'tooltip', { rankShown: true, rankWidth: 36 });
  const picture = createTablePicture();
  reset();
  const draw = drawFamilyTable(ctx, { layout, rows: followedRows, settings, hour: 200, width: 480, height: 320, scrollLeft: 0, scrollTop: 0, rankShown: true, rankWidth: 36, theme: TEST_THEME, now: 1, glideDuration: 0, glide: new Map(), markers: new Map(), inspectedKey: null, selectedKey: null, focusedKey: null, icons: new Map(), dpr: 1 }, picture);
  assert.ok(draw.paintedCells <= 30, `only visible viewport rows painted out of 1,500: ${draw.paintedCells} cells`);
  assert.ok(counts().measureCalls <= 50, `bounded text measurements for 1,500-country table: ${counts().measureCalls}`);
});

test('occupied-states projection carries country looks at hour, filter ranges, unrecorded membership, and live tooltip details', () => {
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
  const labels = {
    GER: [{ from: 0, name: 'Germany', flag: 'old.png' }, { from: 20, name: 'German Reich', flag: 'new.png' }],
    POL: [{ from: 0, name: 'Poland' }],
  };
  assert.deepEqual([nextLookChange(labels, 5), nextLookChange(labels, 19), nextLookChange(labels, 20)], [20, 20, Infinity]);

  const filter = { passing: { GER: [[10, 20]] as const }, unrecorded: [[30, 40, 'Membership not recorded']] as const, bounds: [10, 21, 30, 41] };
  const live = valuesOf(new Map([
    [liveKey('1', 'resistance'), 2_500_000],
    [liveKey('1', 'compliance'), 0],
    [liveKey('1', 'requiredManpower'), 1200],
    [liveKey('1', 'garrison'), 250_000],
    [liveKey('1', 'garrisonStrength'), 87_500],
  ]), NOT_NAMED);
  const run = (hour: number) => occupiedStatesProjection.project(
    { answers: new Map(), panels: new Map([['main', reply]]), live: new Map([['main', live]]) },
    hour,
    { filter, role: 'occupier', labels, lawAs: 'dot', filterAll: false },
  );
  const at15 = run(15);
  assert.deepEqual(at15.rows.map(r => r.key), ['1']);
  const settings = tableSettingsOf(undefined, OCCUPIED_DEFAULTS, OCCUPIED_BASE);
  const flat15 = resolveTableRows(at15, settings);
  const tip = deriveTableTooltip(flat15, '1', at15.columns, new Set(), 15);
  assert.ok(tip);
  assert.equal(tip.rows.find(r => r.label === 'Resistance')?.value, '25%');
  assert.equal(tip.rows.find(r => r.label === 'Assigned manpower')?.value, '0', 'known assigned with no live cell reads 0');
  assert.equal(tip.rows.find(r => r.label === 'Garrison strength')?.value, '87.5%');
  assert.equal(tip.rows.find(r => r.label === 'Occupied country')?.value, 'Poland');

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
  const settings = tableSettingsOf(undefined, specs, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'resistance', direction: 'descending' } });
  const cols = buildColumnLayouts(specs, settings, { state: 140, resistance: 130, owner: 140 });
  const hiddenLayout = layoutFamilyTable(cols, 370, 'tooltip');
  assert.deepEqual(hiddenLayout.columns.map(c => c.key), ['state', 'resistance']);
  assert.deepEqual(hiddenLayout.hidden, ['owner']);
  const scrollLayout = layoutFamilyTable(cols, 250, 'scroll');
  assert.equal(scrollLayout.scrollable, true);
  assert.equal(scrollLayout.pinnedWidth, 140);

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
  registerTableProjection(proj);
  const prevOffscreen = globalThis.OffscreenCanvas;
  class FakeOffscreenCanvas {
    width: number;
    height: number;
    constructor(w: number, h: number) { this.width = w; this.height = h; }
    getContext() { return { ...instrumentedContext().ctx, setTransform() {} }; }
  }
  (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeOffscreenCanvas;
  try {
    const view = familyTableLaneView(() => {});
    const settings = tableSettingsOf(undefined, [
      { key: 'name', label: 'Name', as: 'plain', offered: ['plain'] },
      { key: 'v', label: 'Val', as: 'plain', offered: ['plain'] },
    ], { density: 'comfortable', overflow: 'tooltip', sort: { column: 'v', direction: 'descending' } });
    const staticPanels = { main: { rows: [{ id: 'GER', v: 10 }] } };
    const options = {};
    const model = {
      family: 'table' as const,
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
    assert.equal(view.next?.(scene, 10), Infinity);
    assert.equal(projectCalls, 0, 'next() must never invoke projection.project()');

    const state = new Map<string, unknown>();
    let seq = 1;
    const content = () => `c${seq++}`;
    const mkInput = (now: number, speed: number, audit = false) => ({
      ...scene,
      model: audit ? { ...model, audit: true } : model,
      hour: 10,
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
    assert.ok(Array.isArray(withAudit.fields.audit?.value), 'audit built only when requested');
  } finally {
    (globalThis as unknown as { OffscreenCanvas: unknown }).OffscreenCanvas = prevOffscreen;
  }
});
