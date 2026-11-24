import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { CELL, parseAnswer, parseFollowed } from '../../dashboard/follow/message.ts';
import { message, type Spec } from '../../dashboard/follow/messages.fixture.ts';
import { createBoundedReadView, emptyReadView } from '../../families/boundedView.ts';
import { compileFollowPlan } from '../../families/broker.ts';
import type { IdentityProvider } from '../../families/contract.ts';
import { buildColumnLayouts } from '../../families/table/layout.ts';
import { deriveTableAudit, deriveTableTooltip, resolveTableRows } from '../../families/table/model.ts';
import { tableDefaults, tableSettingsOf } from '../../families/table/settings.ts';
import type { ScoreboardColumn, ScoreboardReply } from '../../generated/dashboard';
import { gatedHour } from '../../test-support/gated.ts';
import { DEFAULT_SORT, OPTIONS, TABLE_BASE, definition } from './definition.ts';
import { isPartial, rankedTags, scoreboardParams, scoreboardSort, selectedColumns, validateColumnsReply, validateScoreboardReply } from './scoreboardData.ts';
import { scoreboardColumnsSpec, scoreboardDeclaration, scoreboardProjection } from './scoreboardProjection.ts';

const family = (kind: string, measure: string) => ({ kind, measure });
const factories: ScoreboardColumn = { key: 'factories', label: 'Factories', unit: 'factories', decimals: 0, available: true, reason: null,
  family: family('country-factories', 'free7'), partial: null, noneAtZero: false };
const casualties: ScoreboardColumn = { key: 'casualties', label: 'Casualties', unit: 'people', decimals: 0, available: false,
  reason: 'Choose Casualties dealt or Casualties taken in settings.', family: null, partial: null, noneAtZero: false };
const power: ScoreboardColumn = { key: 'politicalPower', label: 'Content political label', unit: 'points', decimals: 5, available: true, reason: null,
  family: family('country-measure', 'politicalPower'), partial: null, noneAtZero: false };
const stability: ScoreboardColumn = { key: 'stability', label: 'Stability', unit: 'percent', decimals: 5, available: true, reason: null,
  family: family('country-stability', 'stability'), partial: null, noneAtZero: false };
const taken: ScoreboardColumn = { key: 'casualtiesTaken', label: 'Casualties taken', unit: 'people', decimals: 0, available: true, reason: null,
  note: 'Losses without a recorded attacker are excluded.', family: family('casualty-totals', 'taken'), partial: family('casualty-totals', 'partial'), noneAtZero: true };
const registry = [factories, casualties, power, stability, taken];
const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

const identityOf = (names: Readonly<Record<string, string>>, colour?: string, nextChangeAt = Infinity): IdentityProvider => ({
  resolve: (kind, key, atHour) => ({
    kind,
    key,
    atHour,
    holder: { status: 'ready', value: key },
    name: names[key] ? { status: 'ready', value: names[key]! } : { status: 'absent', reason: 'Name not recorded' },
    colour: colour ? { status: 'ready', value: colour } : { status: 'absent', reason: 'Colour not recorded' },
    flagChoice: { status: 'absent', reason: 'Flag not recorded' },
    validInterval: [0, 1000],
    nextChangeAt,
  }),
});

test('the scoreboard follows playback over the registry defaults with the top ten, and its settings keep their meaning', () => {
  assert.equal(definition.defaultTime, 'follow');
  assert.equal(definition.defaultCountries.kind, 'group');
  assert.equal(OPTIONS.find(option => option.key === 'columns')?.kind, 'content-set');
  assert.deepEqual(DEFAULT_SORT, { column: '', direction: 'descending' });
  assert.deepEqual(selectedColumns(registry, '', ['factories', 'casualties']).map(column => column.key), ['factories', 'casualties']);
  assert.deepEqual(selectedColumns(registry, 'stability factories', ['factories']).map(column => column.key), ['stability', 'factories']);
  assert.deepEqual(scoreboardSort(registry, [casualties], { column: 'factories', direction: 'descending' }, 'factories'), { column: 'country', direction: 'descending' });
  assert.deepEqual(tableSortOf(tableSortToken({ column: 'politicalPower', direction: 'ascending' }), DEFAULT_SORT), { column: 'politicalPower', direction: 'ascending' });
});


test('a reply’s cells are exact amounts, a missing value keeps its reason, and slot/tag keys stay distinct', () => {
  const wide = (1n << 80n) + 1n;
  const reply: ScoreboardReply = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [
    { country: 'GER', tag: 'GER', cells: [{ valueRaw: `-${wide}`, missing: null, partial: true }, { valueRaw: null, missing: 'GER\'s stability was not recorded at this hour', partial: false }] },
  ] }, 2);
  const shown = [taken, stability];
  const model = scoreboardProjection.project(
    createBoundedReadView({ hour: 40, panels: new Map([['main', reply]]) }),
    { shown, candidates: ['GER'], top: null, rankMarkers: false },
  );
  const settings = tableDefaults(model.columns, TABLE_BASE);
  const flat = resolveTableRows(model, settings);
  const audit = deriveTableAudit({ model, flatRows: flat, visibleColumns: buildColumnLayouts(model.columns, settings) });
  assert.ok(audit.find(row => row.key === 'GER:casualtiesTaken')?.value?.includes(`["partial",["int","-${wide}"],"Some inputs were not recorded at this hour",false]`));
  assert.ok(audit.find(row => row.key === 'GER:stability')?.value?.includes(`["notRecorded","GER's stability was not recorded at this hour"]`));
  const tip = deriveTableTooltip(flat, 'GER', model.columns);
  assert.ok(tip);
  assert.equal(tip.rows.find(row => row.label === 'Stability')?.detail, "GER's stability was not recorded at this hour");

  const cells = [{ valueRaw: '1', missing: null, partial: false }, { valueRaw: '2', missing: null, partial: false }];
  const both = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 5, tag: 'GER', cells }, { country: 'GER', tag: 'GER', cells }] }, 2);
  const bothModel = scoreboardProjection.project(
    createBoundedReadView({ hour: 40, panels: new Map([['main', both]]) }),
    { shown, candidates: [], top: null, rankMarkers: false },
  );
  assert.deepEqual(bothModel.rows.map(row => row.key), [countryKey(5), 'GER']);
  assert.throws(() => validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 5, tag: 'GER', cells }, { country: 5, tag: 'GER', cells }] }, 2), /Invalid Scoreboard row/);
  assert.throws(() => validateScoreboardReply({ ...reply, rows: [{ tag: 'GER', cells: [{ valueRaw: null, missing: null, partial: false }] }] }, 1), /row/);
  assert.throws(() => validateScoreboardReply(reply, 3), /row/);
});

test('followed values and rankings preserve exact precision, partial markers and missing reasons', () => {
  const V = CELL.value, U = CELL.unrecorded;
  const followed = answer({ keys: [7n, 8n, 0n], columns: 1, names: ['ITA', 'GER', 'SOV'], values: [(1n << 60n) + 3n, 0n, 0n], states: [V, U, CELL.noRow],
    reasons: [null, null, 'SOV is held by no country'] });
  assert.deepEqual(rankedTags(followed), ['ITA']);
  assert.equal(isPartial(1n), true);
  assert.equal(isPartial(0n), false);
  assert.equal(isPartial(undefined), false);

  const model = scoreboardProjection.project(
    createBoundedReadView({ hour: 40, answers: new Map([['factories', followed]]) }),
    { shown: [factories, casualties], candidates: ['ITA', 'GER', 'SOV', 'ENG'], top: null, rankMarkers: false },
  );
  const byKey = new Map(model.rows.map(row => [row.key, row]));
  assert.deepEqual(byKey.get('ITA')!.cells.factories!.value, { kind: 'known', value: (1n << 60n) + 3n });
  assert.equal(byKey.get('GER')!.cells.factories!.value.kind, 'notRecorded');
  assert.deepEqual(byKey.get('SOV')!.cells.factories!.value, { kind: 'notRecorded', reason: 'SOV is held by no country' });
  assert.deepEqual(byKey.get('ITA')!.cells.casualties!.value, { kind: 'notRecorded', reason: casualties.reason });
});

test('the registry names the measure behind every available column', () => {
  const reply = { branchId: '1', defaultColumns: ['factories'], defaultSort: 'factories', columns: registry, warnings: [] };
  assert.equal(validateColumnsReply(reply).columns.length, registry.length);
  assert.throws(() => validateColumnsReply({ ...reply, columns: [{ ...factories, family: null }] }), /column/);
  assert.throws(() => validateColumnsReply({ ...reply, defaultSort: 'ships' }), /defaults/);
});

test('followed scoreboard filters selected countries by holder existence at hour, resolves slot keys to dated tags, and matches exact scoreboard audit', () => {
  const slotKey = '\u00007';
  const holders = {
    [slotKey]: [{ tag: 'GER', firstHour: 0, lastHour: 104 }, { tag: 'GDR', firstHour: 105, lastHour: 200 }],
    ITA: [{ tag: 'ITA', firstHour: 110, lastHour: 200 }],
  };
  const identity = identityOf({ GER: 'Germany', GDR: 'East Germany', ITA: 'Italy' });
  const facAns = parseFollowed(message(100, [{
    id: 1,
    answer: {
      keys: [1n, 2n],
      columns: 1,
      names: [slotKey, 'ITA'],
      values: [0n, 0n],
      states: [CELL.unrecorded, CELL.noRow],
      reasons: ['Factories not recorded', 'ITA is held by no country'],
    },
  }])).answers[0]!.answer;
  const exactReply: ScoreboardReply = {
    branchId: '1',
    hour: 100,
    missing: null,
    warnings: [],
    rows: [
      { country: 7, tag: 'GER', cells: [{ valueRaw: null, missing: 'Factories not recorded', partial: false }] },
    ],
  };
  const opts = {
    shown: [factories],
    candidates: [slotKey, 'ITA'],
    holders,
    top: null,
    rankMarkers: false,
  };
  const followed = scoreboardProjection.project(createBoundedReadView({ hour: 100, answers: new Map([['factories', facAns]]), identity }), opts);
  const exact = scoreboardProjection.project(createBoundedReadView({ hour: 100, panels: new Map([['main', exactReply]]), identity }), opts);

  assert.deepEqual(followed.rows.map(r => r.key), [slotKey], 'ITA does not exist at hour 100 and is omitted on follow as on exact');
  assert.deepEqual(followed.rows[0]!.cells.country!.value, { kind: 'known', value: 'Germany' }, 'slot key resolves display label via dated tag GER');
  assert.deepEqual(followed.rows[0]!.cells.factories!.value, { kind: 'notRecorded', reason: 'Factories not recorded' }, 'existing country with missing measurement stays');

  const specs = scoreboardColumnsSpec([factories], null);
  const settings = tableSettingsOf(undefined, specs, TABLE_BASE);
  const layouts = buildColumnLayouts(specs, settings);
  assert.deepEqual(
    deriveTableAudit({ model: followed, flatRows: resolveTableRows(followed, settings), visibleColumns: layouts }),
    deriveTableAudit({ model: exact, flatRows: resolveTableRows(exact, settings), visibleColumns: layouts }),
  );
  assert.equal(scoreboardProjection.nextChange?.(emptyReadView(100), opts), 105, 'nextChange reports slot tag switch boundary');
});

test('scoreboard decodes checked-in native FOL3 fixtures for slot+tag queries, matches Rust missing reasons without TS rewriting, enables country colours, and keeps rows when all columns are unavailable', () => {
  const bin525 = readFileSync(new URL('../../../../crates/dashboard-follow/fixtures/series_slot_tag_525.bin', import.meta.url));
  const bin800 = readFileSync(new URL('../../../../crates/dashboard-follow/fixtures/series_slot_tag_800.bin', import.meta.url));
  const ans525 = parseAnswer(new Uint8Array(bin525));
  const ans800 = parseAnswer(new Uint8Array(bin800));
  const opts = {
    shown: [power],
    candidates: ['\u00007', 'ITA'],
    holders: {
      '\u00007': [{ tag: 'GER', firstHour: 500, lastHour: 1000 }],
      ITA: [{ tag: 'ITA', firstHour: 800, lastHour: 1000 }],
    },
    top: null,
    rankMarkers: false,
  };
  const exact525: ScoreboardReply = {
    branchId: '1',
    hour: 525,
    missing: null,
    warnings: [],
    rows: [
      { country: 7, tag: 'GER', cells: [{ valueRaw: null, missing: "GER's politicalPower was not recorded at this hour", partial: false }] },
    ],
  };
  const exact800: ScoreboardReply = {
    branchId: '1',
    hour: 800,
    missing: null,
    warnings: [],
    rows: [
      { country: 7, tag: 'GER', cells: [{ valueRaw: '8225', missing: null, partial: false }] },
      { country: 'ITA', tag: 'ITA', cells: [{ valueRaw: '3225', missing: null, partial: false }] },
    ],
  };
  const specs = scoreboardColumnsSpec([power], null);
  const settings = tableSettingsOf(undefined, specs, TABLE_BASE);
  const layouts = buildColumnLayouts(specs, settings);

  const f525 = scoreboardProjection.project(createBoundedReadView({ hour: 525, answers: new Map([['politicalPower', ans525]]) }), opts);
  const e525 = scoreboardProjection.project(createBoundedReadView({ hour: 525, panels: new Map([['main', exact525]]) }), opts);
  assert.deepEqual(
    deriveTableAudit({ model: f525, flatRows: resolveTableRows(f525, settings), visibleColumns: layouts, rankShown: true }),
    deriveTableAudit({ model: e525, flatRows: resolveTableRows(e525, settings), visibleColumns: layouts, rankShown: true }),
    'S2+N3: real native FOL3 bytes for [Slot(7), Tag(ITA)] at hour 525 match exact board audit verbatim',
  );

  const f800 = scoreboardProjection.project(createBoundedReadView({ hour: 800, answers: new Map([['politicalPower', ans800]]) }), opts);
  const e800 = scoreboardProjection.project(createBoundedReadView({ hour: 800, panels: new Map([['main', exact800]]) }), opts);
  assert.deepEqual(
    deriveTableAudit({ model: f800, flatRows: resolveTableRows(f800, settings), visibleColumns: layouts, rankShown: true }),
    deriveTableAudit({ model: e800, flatRows: resolveTableRows(e800, settings), visibleColumns: layouts, rankShown: true }),
    'S2: real native FOL3 bytes for [Slot(7), Tag(ITA)] at hour 800 match exact board audit verbatim',
  );

  assert.equal(scoreboardDeclaration.countries.mode, 'multi');
  const colouredReply = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [
    { country: 'GER', tag: 'GER', cells: [{ valueRaw: '5', missing: null, partial: false }] },
  ] }, 1);
  const coloured = scoreboardProjection.project(
    createBoundedReadView({ hour: 40, panels: new Map([['main', colouredReply]]), identity: identityOf({ GER: 'Germany' }, 'rgb(10, 20, 30)', 120) }),
    { shown: [power], candidates: ['GER'], top: null, rankMarkers: false },
  );
  assert.deepEqual(Object.values(coloured.rows[0]!.cells).map(cell => cell.countryColor), ['rgb(10, 20, 30)', 'rgb(10, 20, 30)'], 'M6: every cell of a country row carries the colour the recording gives that country at the hour');
  assert.equal(coloured.rows[0]!.name, 'Germany');
  assert.equal(coloured.nextChangeAt, 120, 'the name and colour hold until the identity changes');

  assert.deepEqual(
    scoreboardParams('1', gatedHour(50), [power], [], null, factories),
    { branchId: '1', hour: 50, columns: ['politicalPower'], countries: [] },
  );

  const track = { source: { id: 's1', revision: 'r1' }, axisOffset: 0 };
  const followView = { following: true, staticSpan: null } as const;
  const unavailPlan = scoreboardDeclaration.sources({
    track,
    branch: { id: '1', firstHour: 0, lastHour: 200 },
    aux: { branchId: '1', columns: [casualties], defaultColumns: ['casualties'], defaultSort: 'casualties', warnings: [] },
    options: { columns: 'casualties', tableSort: '', topMeasure: '' },
    countries: { top: null, tags: ['GER'], holders: { GER: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] }, membership: null },
  } as unknown as Parameters<typeof scoreboardDeclaration.sources>[0], followView);
  assert.equal(unavailPlan.unavailable, undefined);
  assert.equal(unavailPlan.allowEmptyFollow, true);

  const unavailModel = scoreboardProjection.project(emptyReadView(50), {
    shown: [casualties],
    candidates: ['GER'],
    holders: { GER: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] },
    top: null,
    rankMarkers: false,
  });
  assert.equal(unavailModel.rows.length, 1);
  assert.deepEqual(unavailModel.rows[0]!.cells.casualties!.value, { kind: 'notRecorded', reason: casualties.reason });

  const selectedPlan = scoreboardDeclaration.sources({
    track,
    branch: { id: '1', firstHour: 0, lastHour: 200 },
    aux: { branchId: '1', columns: [power, taken], defaultColumns: ['politicalPower', 'casualtiesTaken'], defaultSort: 'politicalPower', warnings: [] },
    options: { columns: 'politicalPower casualtiesTaken', tableSort: '', topMeasure: '' },
    countries: { top: null, tags: ['GER'], holders: { GER: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] }, membership: null },
  } as unknown as Parameters<typeof scoreboardDeclaration.sources>[0], followView);
  const allowFollow = (query: Readonly<Record<string, unknown>> | null) => ({ query: query as never, refused: null });
  const selectedSubs = compileFollowPlan(selectedPlan, allowFollow).subscriptions;
  assert.equal(selectedSubs.length, 3);
  assert.ok(selectedSubs.every(sub => (sub.query as { coverage?: string }).coverage === 'row'), 'selected scoreboard follows opt into coverage: row');

  const topPlan = scoreboardDeclaration.sources({
    track,
    branch: { id: '1', firstHour: 0, lastHour: 200 },
    aux: { branchId: '1', columns: [power, taken], defaultColumns: ['politicalPower', 'casualtiesTaken'], defaultSort: 'politicalPower', warnings: [] },
    options: { columns: 'politicalPower casualtiesTaken', tableSort: '', topMeasure: '' },
    countries: { top: 5, tags: [], holders: {}, membership: null },
  } as unknown as Parameters<typeof scoreboardDeclaration.sources>[0], followView);
  const topSubs = compileFollowPlan(topPlan, allowFollow).subscriptions;
  assert.equal(topSubs.length, 4);
  assert.ok(topSubs.every(sub => (sub.query as { coverage?: string }).coverage === undefined), 'top-N scoreboard follows stay on default Table coverage');
});
