import assert from 'node:assert/strict';
import { test } from 'node:test';
import { advanceTableState, carriedValues, cellBar, fitState, cellText, columnMaxima, drawTable, fitTableColumns, fitText, rankMarkers, sortRowsAtHour, tableReading, tablePicture, tableState, type TableSurfaceModel } from './surface.ts';
import { strengthBar, type TableColumnLayout } from './logic.ts';
import { tableDefaults } from './settings.ts';
import { sourceValues } from './sources.ts';
import { liveKey } from '../../charts/live.ts';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message, type Spec } from '../../dashboard/follow/messages.fixture.ts';

const NUMBERS = ['plain', 'faint', 'track', 'meterAfter', 'meterBefore', 'underline'] as const;
const OCCUPIED_DEFAULTS = tableDefaults([
  { key: 'state', label: 'State', as: 'plain', offered: ['plain'] }, { key: 'owner', label: 'Owner', as: 'flagName', offered: ['flagName', 'name', 'flag'] },
  { key: 'resistance', label: 'Resistance', as: 'meterAfter', tone: 'red', offered: NUMBERS }, { key: 'compliance', label: 'Compliance', as: 'meterAfter', offered: NUMBERS },
], { density: 'comfortable', overflow: 'tooltip', sort: { column: 'resistance', direction: 'descending' } });
const answer = (spec: Spec) => parseFollowed(message(100, [{ id: 1, answer: spec }])).answers[0]!.answer;

const fake = (cells: Record<string, number | undefined>) => ({ get: (key: string) => cells[key], missing: () => 'Not recorded' });

test('text columns sort by their text and number columns by value, with missing rows last', () => {
  const row = (key: string, name: string | null) => ({ key, cells: {
    state: name === null ? { missing: 'Not recorded' } : { text: name }, resistance: { valueKey: `r${key}`, format: 'fixed5Percent' as const } } });
  const rows = [row('1', 'Łódz'), row('2', 'Brittany'), row('3', null), row('4', 'alsace')];
  const values = fake({ r1: 10, r2: 30, r3: 20 });
  const by = (column: string, direction: 'ascending' | 'descending') =>
    sortRowsAtHour(rows, { ...OCCUPIED_DEFAULTS, sort: { column, direction } }, values, 1).map(item => item.key);
  assert.deepEqual(by('state', 'ascending'), ['4', '2', '1', '3'], 'names compare as text, case and accents aside; a missing name sorts last');
  assert.deepEqual(by('resistance', 'descending'), ['2', '3', '1', '4'], 'values compare as numbers; a missing value sorts last');
  const country = (key: string, looks: { from: number; name: string }[]) => ({ key, cells: { state: { looks }, resistance: { valueKey: key, format: { decimals: 0 } } } });
  const countries = [country('A', [{ from: 0, name: 'Zeta' }, { from: 10, name: 'Alpha' }]), country('B', [{ from: 0, name: 'Mid' }])];
  const exact = { get: (key: string) => key === 'A' ? 2n ** 60n : 2n ** 60n + 1n, missing: () => 'Not recorded' };
  const at = (column: string, hour: number) => sortRowsAtHour(countries, { ...OCCUPIED_DEFAULTS, sort: { column, direction: 'ascending' } }, exact, hour).map(item => item.key);
  assert.deepEqual([at('state', 5), at('state', 10)], [['B', 'A'], ['A', 'B']], 'a country sorts by its name at the hour');
  assert.equal(cellText({ looks: [{ from: 10, name: 'Alpha' }] }, null, 5), 'Country name unavailable', 'never a name from after the hour');
  const pending = [country('A', [{ from: 0, name: null, status: 'Loading name…' }] as never), country('B', [{ from: 0, name: 'Mid' }])];
  assert.deepEqual(sortRowsAtHour(pending, { ...OCCUPIED_DEFAULTS, sort: { column: 'state', direction: 'ascending' } }, exact, 5).map(item => item.key), ['B', 'A'],
    'a name not known sorts last, never by the text that says so');
  assert.equal(cellText(pending[0]!.cells.state, null, 5), 'Loading name…', 'and shows that text');
  assert.deepEqual([5, 9, 10].map(hour => tableState({ columns: [], settings: OCCUPIED_DEFAULTS, rows: countries, width: 0, height: 0, scrollLeft: 0, offset: 0 }, exact, hour).nextChange),
    [10, 10, undefined], 'the clock alone changes the picture where a country’s look changes');
  assert.deepEqual(at('resistance', 5), ['A', 'B'], 'values beyond a double’s precision compare exactly');
  assert.deepEqual(sortRowsAtHour(countries, { ...OCCUPIED_DEFAULTS, sort: { column: 'resistance', direction: 'descending' } }, exact, 5).map(item => item.key), ['B', 'A']);
});

test('a garrison bar is its strength narrowed to whole percent: none, empty or red as the game shows it, never zero for missing', () => {
  const strength = (raw: number | undefined, alertBelow?: number) => cellBar({ valueKey: 'amount', bar: { key: 'strength', alertBelow } }, 1, undefined, fake({ strength: raw }));
  assert.deepEqual(strength(87_544), { fraction: 0.88, alert: false }, '87.544% draws as 88%');
  assert.deepEqual(strength(29_000, 30_000), { fraction: 0.29, alert: true }, 'below the content threshold shows red');
  assert.deepEqual(strength(undefined, 30_000), { fraction: null, alert: false }, 'a strength not recorded draws no bar, not an empty one');
  assert.deepEqual(cellBar({ bar: { fixed: 0 } }, null, undefined, null), { fraction: 0, alert: false }, 'an unmet requirement is an empty bar');
  assert.deepEqual(cellBar({ bar: null }, 5, 10, null), { fraction: null, alert: false }, 'nothing required: no bar');
  assert.deepEqual(cellBar({}, 2_500_000, 10_000_000, null), { fraction: 0.25, alert: false }, 'otherwise the value against its scale');
});

test('the worker fits a table to the rows it draws in view: a set width kept, a fitted one grown to them and held there', () => {
  const theme = { surface: '', text: '', muted: '', line: '', lineSoft: '', accent: '', blue: '', red: '', amber: '', green: '', grey: '', hatch: '',
    font: 'f', fontSize: { small: 10, body: 12 }, lineWidth: 1, hatchSpacing: 4 };
  const measure = (text: string) => text.length * 6;
  const model = (names: string[], width?: number): TableSurfaceModel => ({
    columns: [{ key: 'name', label: 'Name', priority: 0, width: 0, shown: true, display: 'plain' },
      { key: 'set', label: 'Set', priority: 1, width: 0, shown: true, display: 'plain' }],
    settings: { ...OCCUPIED_DEFAULTS, sort: { column: 'name', direction: 'ascending' }, columns: { ...OCCUPIED_DEFAULTS.columns, set: { show: true, as: 'plain', width: width ?? 77 } } },
    rows: names.map(name => ({ key: name, cells: { name: { text: name }, set: { text: 'x' } } })), width: 400, height: 56, scrollLeft: 0, offset: 0, theme });
  const floor = new Map<string, number>();
  const wide = tableState(model(['A', 'Bbbbbbbbbbbbbbbbbbbb', 'Hidden below the view and longer than either']), null, 1);
  assert.equal(fitState(wide, measure, floor), true);
  const width = (state: typeof wide, key: string) => state.layout.columns.find(column => column.key === key)?.width;
  assert.equal(width(wide, 'set'), 77, 'a set width');
  assert.ok(width(wide, 'name')! >= 20 * 6 && width(wide, 'name')! < 44 * 6, 'the rows in view, not those below');
  const narrow = tableState(model(['A']), null, 1);
  fitState(narrow, measure, floor);
  assert.equal(width(narrow, 'name'), width(wide, 'name'), 'a fitted width holds while its floor does');
  assert.equal(fitState(narrow, measure, floor), false, 'nothing moved');
  const grown = tableState(model(['A', 'Bbbbbbbbbbbbbbbbbbbbb']), null, 1);
  fitState(grown, measure, floor);
  assert.equal(width(grown, 'name'), width(wide, 'name'), 'a column grows with room to spare: a text a little wider moves nothing');
});

test('fitted widths come from the drawn texts and the header, never shrink below the floor, and a set width wins', () => {
  const theme = { surface: '', text: '', muted: '', line: '', lineSoft: '', accent: '', blue: '', red: '', amber: '', green: '', grey: '', hatch: '',
    font: 'f', fontSize: { small: 10, body: 12 }, lineWidth: 1, hatchSpacing: 4 };
  const measure = (text: string) => text.length * 7;
  const column = (key: string, display: TableColumnLayout['display'], numeric = false) => ({ key, label: key, priority: 0, width: 0, shown: true, display, numeric });
  const cols = [column('state', 'plain'), column('owner', 'flag'), column('resistance', 'meterAfter', true)];
  const texts = { state: ['A much longer state name'], owner: ['A very long country name indeed'], resistance: ['100%'] };
  const [state, owner, resistance] = fitTableColumns(cols, texts, OCCUPIED_DEFAULTS, theme, measure);
  assert.equal(state!.width, Math.ceil(8 + 24 * 7 + 8), 'inset + widest text + inset');
  assert.equal(owner!.width, Math.ceil(8 + 5 * 7 + 14 + 28), 'a flag-only column fits its header, not the names it hides');
  assert.equal(resistance!.width, Math.ceil(8 + Math.max(4 * 7 + 52 + 28, 10 * 7 + 14 + 28)), 'number, meter and caret lane, or the header');
  const held = fitTableColumns(cols, { state: ['Short'] }, OCCUPIED_DEFAULTS, theme, measure, new Map([['state', 300]]));
  assert.equal(held[0]!.width, 300, 'a fitted column keeps its floor while the clock plays');
  const set = fitTableColumns(cols, texts, { ...OCCUPIED_DEFAULTS, columns: { ...OCCUPIED_DEFAULTS.columns, state: { ...OCCUPIED_DEFAULTS.columns.state!, width: 90 } } }, theme, measure);
  assert.equal(set[0]!.width, 90, 'a width the user set wins');
});

test('a frame repaints only the cells whose values changed, and every cell of a row that moved', () => {
  const theme = { surface: '', text: '', muted: '', line: '', lineSoft: '', accent: '', blue: '', red: '', amber: '', green: '', grey: '', hatch: '',
    font: 'f', fontSize: { small: 10, body: 12 }, lineWidth: 1, hatchSpacing: 4 };
  const painted: string[] = [];
  const ctx = new Proxy({}, { set: () => true, get: (_, name) => name === 'measureText' ? (text: string) => ({ width: text.length * 6 })
    : name === 'fillText' ? (text: string) => { painted.push(text); } : name === 'createLinearGradient' ? () => ({ addColorStop() {} }) : () => {} }) as unknown as CanvasRenderingContext2D;
  const model: TableSurfaceModel = {
    columns: [{ key: 'name', label: 'Name', priority: 0, width: 100, shown: true, display: 'plain' },
      { key: 'factories', label: 'Factories', priority: 1, width: 100, shown: true, display: 'plain', numeric: true }],
    settings: { ...OCCUPIED_DEFAULTS, sort: { column: 'factories', direction: 'descending' } },
    rows: ['GER', 'ITA', 'SOV'].map(tag => ({ key: tag, cells: { name: { text: tag }, factories: { valueKey: tag, format: { decimals: 0 } } } })),
    width: 200, height: 200, scrollLeft: 0, offset: 0, theme,
  };
  const picture = tablePicture();
  const frame = (cells: Record<string, number>) => { painted.length = 0; drawTable(ctx, tableState(model, fake(cells), 1), 0, 0, undefined, picture); return [...painted]; };
  assert.deepEqual(frame({ GER: 30, ITA: 20, SOV: 10 }), ['GER', 'ITA', 'SOV', '30', '20', '10'], 'the first frame paints every cell');
  assert.deepEqual(frame({ GER: 30, ITA: 21, SOV: 10 }), ['21'], 'one value moved: only its cell');
  assert.deepEqual(frame({ GER: 30, ITA: 21, SOV: 10 }), [], 'nothing moved: nothing painted');
  assert.deepEqual(frame({ GER: 30, ITA: 21.3, SOV: 10 }), [], 'a value moving below the shown precision paints nothing');
  assert.deepEqual(frame({ GER: 30, ITA: 21, SOV: 40 }), ['SOV', 'GER', 'ITA', '40', '30', '21'], 'the order moved: every cell of each row that moved');
  const named: TableSurfaceModel = { ...model, rows: [{ key: 'GER', cells: { name: { looks: [{ from: 0, name: 'Germany' }, { from: 10, name: 'German Reich' }] }, factories: { valueKey: 'GER', format: { decimals: 0 } } } }] };
  const renamed = tablePicture(), state = tableState(named, fake({ GER: 30 }), 5);
  drawTable(ctx, state, 0, 0, undefined, renamed);
  painted.length = 0;
  advanceTableState(state, 10);
  drawTable(ctx, state, 0, 0, undefined, renamed);
  assert.deepEqual(painted, ['German Reich'], 'a look the clock changed: only its cell');
  const shown = (columns: TableSurfaceModel['columns']) => { painted.length = 0; drawTable(ctx, tableState({ ...model, columns }, fake({ GER: 30, ITA: 21, SOV: 40 }), 1), 0, 0, undefined, picture); return [...painted]; };
  assert.deepEqual(shown(model.columns.map(column => column.numeric ? { ...column, display: 'meterBefore' } : column)), ['SOV', 'GER', 'ITA', '40', '30', '21'],
    'a column shown another way paints again');
  const gutter = (rankWidth: number) => { painted.length = 0; drawTable(ctx, tableState({ ...model, width: 400, rankShown: true, rankWidth }, fake({ GER: 30, ITA: 21, SOV: 40 }), 1), 0, 0, undefined, picture); return [...painted]; };
  gutter(36);
  assert.deepEqual(gutter(64).filter(text => !/^\d$/.test(text)), ['SOV', 'GER', 'ITA', '40', '30', '21'], 'a rank gutter of another width moves every cell');
  const empty = (reason: string) => { painted.length = 0; drawTable(ctx, tableState({ ...model, rows: [], empty: reason }, fake({}), 1), 0, 0, undefined, picture); return [...painted]; };
  assert.deepEqual(empty('No rows here'), ['No rows here'], 'no rows: the reason in their place');
  assert.deepEqual(empty('Nothing recorded'), ['Nothing recorded'], 'another reason replaces it');
});

test('a pinned table carries its cells’ values and those its tooltips read, each missing one with its reason', () => {
  const rows = [{ key: 'GER', cells: { factories: { valueKey: 'GER:f', bar: { key: 'GER:b' } } } }];
  const now = { get: (key: string) => key === 'GER:f' ? 3 : key === 'GER:speed' ? 2n : key === 'GER:b' ? 'Not read yet' : undefined, missing: (key: string) => `${key} not recorded` };
  assert.deepEqual(carriedValues(rows, now, row => [`${row}:speed`, `${row}:assigned`]), {
    values: { 'GER:f': 3, 'GER:b': undefined, 'GER:speed': 2n, 'GER:assigned': undefined },
    missing: { 'GER:b': 'Not read yet', 'GER:assigned': 'GER:assigned not recorded' } });
});

test('clock-bounded rows keep future values out of the bar maximum', () => {
  const model: TableSurfaceModel = {
    columns: [{ key: 'resistance', label: 'Resistance', priority: 1, width: 140, shown: true, display: 'meterAfter', numeric: true }],
    settings: OCCUPIED_DEFAULTS,
    rows: [
      { key: 'old', revealAt: 1, cells: { resistance: { valueKey: 'old', format: 'fixed5Percent' } } },
      { key: 'future', revealAt: 10, cells: { resistance: { valueKey: 'future', format: 'fixed5Percent' } } },
    ], width: 240, height: 120, scrollLeft: 0, offset: 0,
  };
  const values = { get: (key: string) => key === 'old' ? 50 : 100, missing: () => 'Not recorded' };
  const state = tableState(model, values, 1);
  assert.deepEqual(state.rows.map(row => row.key), ['old']);
  assert.equal(state.maxima.get('resistance'), 50);
});

test('the bar’s Number arithmetic matches the game’s strength narrowing', () => {
  for (let seed = 1; seed < 300; seed++) {
    const raw = Math.round(Math.sin(seed) * 10 ** (seed % 9));
    const exact = Math.max(0, Math.min(1, Number(strengthBar(BigInt(raw))) / 100000));
    assert.equal(cellBar({ bar: { key: 's' } }, null, undefined, fake({ s: raw })).fraction, exact, `strength ${raw}`);
  }
});

test('several followed sources feed one table: a cell reads its source at the row’s name, exactly, and missing keeps its reason', () => {
  const V = CELL.value, U = CELL.unrecorded;
  const wide = (1n << 60n) + 7n;
  const factories = answer({ keys: [1n, 2n, 3n], columns: 1, names: ['GER', 'ITA', 'SOV'], values: [120n, wide, 0n], states: [V, V, U], reasons: [null, null, null] });
  const partial = answer({ keys: [1n, 2n], columns: 1, names: ['GER', 'ITA'], values: [0n, 3n], states: [V, V] });
  const values = sourceValues(new Map([['factories', factories], ['partial', partial], ['stability', answer({ notRecorded: 'Stability is not recorded' })], ['waiting', undefined]]));
  assert.equal(values.get(liveKey('GER', 'factories')), 120, 'a value a Number holds exactly is a Number');
  assert.equal(values.get(liveKey('ITA', 'factories')), wide, 'a wider one stays exact');
  assert.equal(values.get(liveKey('SOV', 'factories')), undefined, 'a value not recorded is never zero');
  assert.equal(values.missing(liveKey('SOV', 'factories')), 'Not recorded at this hour');
  assert.equal(values.missing(liveKey('ENG', 'factories')), 'Not in the answer');
  assert.equal(values.missing(liveKey('GER', 'stability')), 'Stability is not recorded');
  assert.equal(values.missing(liveKey('GER', 'waiting')), 'Not read yet');
  assert.equal(values.missing(liveKey('GER', 'ships')), 'Not followed');

  const cell = (tag: string) => ({ valueKey: liveKey(tag, 'factories'), format: { decimals: 0 }, prefix: { key: liveKey(tag, 'partial'), text: '† ' } });
  assert.equal(cellText(cell('GER'), values), '120', 'a zero mark is no partial total');
  assert.equal(cellText(cell('ITA'), values), '† 1,152,921,504,606,846,983');
  assert.equal(cellText(cell('SOV'), values), 'Not recorded at this hour');
  const rows = ['GER', 'ITA', 'SOV'].map(tag => ({ key: tag, cells: { factories: cell(tag) } }));
  assert.deepEqual(sortRowsAtHour(rows, { ...OCCUPIED_DEFAULTS, sort: { column: 'factories', direction: 'descending' } }, values, 1).map(row => row.key),
    ['ITA', 'GER', 'SOV'], 'exact values sort as numbers, missing last');
  const named = [...rows, { key: 'ENG', cells: { factories: cell('ENG') } }];
  assert.deepEqual(sortRowsAtHour(named, { ...OCCUPIED_DEFAULTS, sort: { column: 'factories', direction: 'descending' } }, values, 1, 'factories').map(row => row.key),
    ['ITA', 'GER'], 'members are the rows the source names with a value at the hour');
});

test('only a column whose display measures against the column keeps a maximum', () => {
  const column = (key: string, display: TableColumnLayout['display'], tone?: TableColumnLayout['tone']) => ({ key, label: key, priority: 0, width: 80, shown: true, display, tone, numeric: true });
  const rows = [{ key: 'a', cells: { plain: { valueKey: 'a' }, faint: { valueKey: 'a' }, graded: { valueKey: 'a' } } }];
  const maxima = columnMaxima(rows, [column('plain', 'plain'), column('faint', 'faint'), column('graded', 'plain', 'value-bad')], fake({ a: 5 }));
  assert.deepEqual([...maxima.keys()], ['faint', 'graded']);
});

test('a row that moves rank is marked by how far and which way until the mark ends; a new row or a held one is not marked again', () => {
  const first = rankMarkers(['GER', 'ITA', 'SOV'], ['SOV', 'GER', 'ITA', 'ENG'], 1000, new Map(), 2500);
  assert.deepEqual(Object.fromEntries(first), { SOV: { delta: 2, until: 3500 }, GER: { delta: -1, until: 3500 }, ITA: { delta: -1, until: 3500 } });
  const later = rankMarkers(['SOV', 'GER', 'ITA', 'ENG'], ['SOV', 'GER', 'ENG', 'ITA'], 2000, first, 2500);
  assert.deepEqual(Object.fromEntries(later), { SOV: { delta: 2, until: 3500 }, GER: { delta: -1, until: 3500 }, ENG: { delta: 1, until: 4500 }, ITA: { delta: -1, until: 4500 } });
  assert.deepEqual(Object.fromEntries(rankMarkers(['SOV', 'GER', 'ENG', 'ITA'], ['SOV', 'GER', 'ENG', 'ITA'], 4000, later, 2500)), { ENG: { delta: 1, until: 4500 }, ITA: { delta: -1, until: 4500 } });
});

test('words that do not fit end in an ellipsis inside the width; text that fits is untouched', () => {
  const ctx = { font: 'f', measureText: (text: string) => ({ width: text.length * 7 }) } as never;
  assert.equal(fitText(ctx, 'Short', 100), 'Short');
  const cut = fitText(ctx, 'SWE holds no row of casualty totals at this hour', 140);
  assert.ok(cut.endsWith('…') && cut.length * 7 <= 140, cut);
  assert.equal(cut, 'SWE holds no row of…');
  assert.equal(fitText(ctx, 'Anything', 3), '…');
});

test('a reading pairs the values the table drew with the hour it drew them at: never newer values, never a newer hour', () => {
  const resistance = liveKey('1', 'resistance'), speed = liveKey('1', 'resistanceSpeed'), garrison = liveKey('1', 'garrison');
  const model: TableSurfaceModel = {
    columns: [{ key: 'resistance', label: 'Resistance', priority: 1, width: 140, shown: true, display: 'plain', numeric: true, format: 'fixed5Percent' }],
    settings: OCCUPIED_DEFAULTS, rows: [{ key: '1', cells: { resistance: { valueKey: resistance, format: 'fixed5Percent' } } }],
    width: 240, height: 120, scrollLeft: 0, offset: 0,
  };
  const live: Record<string, number | undefined> = { [resistance]: 4_000_000, [speed]: -25_000 };
  const state = tableState(model, { get: key => live[key], missing: () => 'Not recorded' }, 40, 40);
  const keys = [resistance, speed, garrison];
  assert.deepEqual(tableReading(state, keys), { hour: 40, values: { [resistance]: 4_000_000, [speed]: -25_000 }, missing: { [garrison]: 'Not recorded' } },
    'a tooltip-only cell is read from the drawn values too');

  advanceTableState(state, 44);
  assert.deepEqual(tableReading(state, keys), { hour: 44, values: { [resistance]: 4_000_000, [speed]: -25_000 }, missing: { [garrison]: 'Not recorded' } });

  const landed = { [resistance]: 9_000_000 };
  const redrawn = tableState(model, { get: key => landed[key as keyof typeof landed], missing: () => 'Not recorded' }, 45, 45);
  assert.equal(tableReading(state, keys).values[resistance], 4_000_000);
  assert.deepEqual(tableReading(redrawn, [resistance]), { hour: 45, values: { [resistance]: 9_000_000 }, missing: {} });

  assert.equal(tableReading(tableState(model, state.values, 44, 60), keys).hour, 60);

  assert.deepEqual(tableReading(state, [], { first: 0, count: 5 }).rows, [{ key: '1', cells: { resistance: '40%' } }]);
});
