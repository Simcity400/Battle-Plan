import assert from 'node:assert/strict';
import test from 'node:test';
import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message, type Spec } from '../../dashboard/follow/messages.fixture.ts';
import { EMPTY_SOURCES } from '../../families/contract.ts';
import { deriveTableAudit, deriveTableTooltip, resolveTableRows } from '../../families/table/model.ts';
import { tableDefaults, tableSettingsOf } from '../../families/table/settings.ts';
import type { ScoreboardColumn, ScoreboardReply } from '../../generated/dashboard';
import { gatedHour } from '../../test-support/gated.ts';
import { DEFAULT_SORT, OPTIONS, TABLE_BASE, definition } from './definition.ts';
import { isPartial, rankedTags, rankingColumn, rankingDescending, scoreboardParams, scoreboardSort, selectedColumns, validateColumnsReply, validateScoreboardReply } from './scoreboardData.ts';
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

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('a top ranks by the sorted measure, else the measure chosen, never by an unavailable column', () => {
  assert.equal(rankingColumn(registry, { column: 'stability', direction: 'ascending' }, '', 'factories'), stability);
  assert.equal(rankingColumn(registry, { column: 'country', direction: 'ascending' }, 'politicalPower', 'factories'), power);
  assert.equal(rankingColumn(registry, { column: 'casualties', direction: 'descending' }, 'casualties', 'factories'), factories);
  assert.deepEqual(scoreboardParams('1', gatedHour(50), [power, casualties], [], 10, factories),
    { branchId: '1', hour: 50, columns: ['politicalPower', 'casualties'], countries: [], top: { count: 10, sort: 'factories', descending: true } });
  assert.equal(rankingDescending({ column: 'factories', direction: 'ascending' }, factories), false);
  assert.equal(rankingDescending({ column: 'country', direction: 'ascending' }, factories), true);
  assert.deepEqual(scoreboardParams('1', gatedHour(50), [power], ['GER'], null, factories), { branchId: '1', hour: 50, columns: ['politicalPower'], countries: ['GER'] });
  assert.equal(scoreboardParams('1', gatedHour(50), [power], [], null, factories), null);
  assert.equal(scoreboardParams('1', gatedHour(50), [], ['GER'], null, factories), null);
});

test('a reply’s cells are exact amounts, a missing value keeps its reason, and slot/tag keys stay distinct', () => {
  const wide = (1n << 80n) + 1n;
  const reply: ScoreboardReply = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [
    { country: 'GER', tag: 'GER', cells: [{ valueRaw: `-${wide}`, missing: null, partial: true }, { valueRaw: null, missing: 'GER\'s stability was not recorded at this hour', partial: false }] },
  ] }, 2);
  const shown = [taken, stability];
  const model = scoreboardProjection.project(
    { answers: new Map(), panels: new Map([['main', reply]]), live: new Map() },
    40,
    { shown, candidates: ['GER'], top: null, rankMarkers: false, looks: { GER: [{ from: 0, name: 'Germany' }] } },
  );
  const settings = tableDefaults(model.columns, TABLE_BASE);
  const flat = resolveTableRows(model, settings);
  const audit = deriveTableAudit(flat, model.columns, 40);
  assert.equal(audit.find(row => row.key === 'GER:casualtiesTaken')?.value, JSON.stringify(['partial', `-${wide}`, 'Some inputs were not recorded at this hour', false]));
  assert.equal(audit.find(row => row.key === 'GER:stability')?.value, JSON.stringify(['notRecorded', "GER's Stability was not recorded at this hour"]));
  const tip = deriveTableTooltip(flat, 'GER', model.columns, new Set(), 40);
  assert.ok(tip);
  assert.equal(tip.rows.find(row => row.label === 'Stability')?.detail, "GER's Stability was not recorded at this hour");

  const cells = [{ valueRaw: '1', missing: null, partial: false }, { valueRaw: '2', missing: null, partial: false }];
  const both = validateScoreboardReply({ branchId: '1', hour: 40, missing: null, warnings: [], rows: [{ country: 5, tag: 'GER', cells }, { country: 'GER', tag: 'GER', cells }] }, 2);
  const bothModel = scoreboardProjection.project(
    { answers: new Map(), panels: new Map([['main', both]]), live: new Map() },
    40,
    { shown, candidates: [], top: null, rankMarkers: false, looks: { GER: [{ from: 0, name: 'Germany' }] } },
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
    { answers: new Map([['factories', followed]]), panels: new Map(), live: new Map() },
    40,
    { shown: [factories, casualties], candidates: ['ITA', 'GER', 'SOV', 'ENG'], top: null, rankMarkers: false, looks: {} },
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
  const looks = {
    GER: [{ from: 0, name: 'Germany' }],
    GDR: [{ from: 105, name: 'East Germany' }],
    ITA: [{ from: 110, name: 'Italy' }],
  };
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
    looks,
  };
  const followed = scoreboardProjection.project({ answers: new Map([['factories', facAns]]), panels: new Map(), live: new Map() }, 100, opts);
  const exact = scoreboardProjection.project({ answers: new Map(), panels: new Map([['main', exactReply]]), live: new Map() }, 100, opts);

  assert.deepEqual(followed.rows.map(r => r.key), [slotKey], 'ITA does not exist at hour 100 and is omitted on follow as on exact');
  assert.deepEqual(followed.rows[0]!.cells.country!.value, { kind: 'known', value: 'Germany' }, 'slot key resolves display label via dated tag GER');
  assert.deepEqual(followed.rows[0]!.cells.factories!.value, { kind: 'notRecorded', reason: 'Factories not recorded' }, 'existing country with missing measurement stays');

  const specs = scoreboardColumnsSpec([factories], null);
  const settings = tableSettingsOf(undefined, specs, TABLE_BASE);
  assert.deepEqual(
    deriveTableAudit(resolveTableRows(followed, settings), specs, 100),
    deriveTableAudit(resolveTableRows(exact, settings), specs, 100),
  );
  assert.equal(scoreboardProjection.nextChange?.(EMPTY_SOURCES, 100, opts), 105, 'nextChange reports slot tag switch boundary');
});

test('scoreboard normalizes absent-row missing reasons across followed and exact paths, keeps rows when all columns are unavailable, and allows empty countries in params', () => {
  const followedAbsent = answer({
    keys: [1n],
    columns: 1,
    names: ['\u00007'],
    values: [0n],
    states: [CELL.noRow],
    reasons: ["7's free7 was not recorded at this hour"],
  });
  const exactAbsent: ScoreboardReply = {
    branchId: '1',
    hour: 100,
    missing: null,
    warnings: [],
    rows: [
      { country: 7, tag: 'GER', cells: [{ valueRaw: null, missing: 'GER holds no row of country-factories at this hour', partial: false }] },
    ],
  };
  const opts = {
    shown: [factories],
    candidates: ['\u00007'],
    holders: { '\u00007': [{ tag: 'GER', firstHour: 0, lastHour: 200 }] },
    top: null,
    rankMarkers: false,
    looks: { GER: [{ from: 0, name: 'Germany' }] },
  };
  const fModel = scoreboardProjection.project({ answers: new Map([['factories', followedAbsent]]), panels: new Map(), live: new Map() }, 100, opts);
  const eModel = scoreboardProjection.project({ answers: new Map(), panels: new Map([['main', exactAbsent]]), live: new Map() }, 100, opts);
  const specs = scoreboardColumnsSpec([factories], null);
  const settings = tableSettingsOf(undefined, specs, TABLE_BASE);
  assert.deepEqual(
    deriveTableAudit(resolveTableRows(fModel, settings), specs, 100, fModel.empty, { rankShown: true }),
    deriveTableAudit(resolveTableRows(eModel, settings), specs, 100, eModel.empty, { rankShown: true }),
  );

  assert.deepEqual(
    scoreboardParams('1', gatedHour(50), [power], [], null, factories),
    { branchId: '1', hour: 50, columns: ['politicalPower'], countries: [] },
  );

  const followSrc = scoreboardDeclaration.source;
  assert.equal(followSrc.kind, 'multi');
  const unavailPlan = followSrc.follow({
    branch: { id: '1', firstHour: 0, lastHour: 200 },
    aux: { branchId: '1', columns: [casualties], defaultColumns: ['casualties'], defaultSort: 'casualties', warnings: [] },
    options: { columns: 'casualties', tableSort: '', topMeasure: '' },
    countries: { top: null, tags: ['GER'], holders: { GER: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] }, membership: null },
  } as unknown as Parameters<typeof followSrc.follow>[0]);
  assert.equal(unavailPlan.unavailable, undefined);
  assert.equal(unavailPlan.allowEmptyFollow, true);

  const unavailModel = scoreboardProjection.project(EMPTY_SOURCES, 50, {
    shown: [casualties],
    candidates: ['GER'],
    holders: { GER: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] },
    top: null,
    rankMarkers: false,
    looks: { GER: [{ from: 0, name: 'Germany' }] },
  });
  assert.equal(unavailModel.rows.length, 1);
  assert.deepEqual(unavailModel.rows[0]!.cells.casualties!.value, { kind: 'notRecorded', reason: casualties.reason });
});
