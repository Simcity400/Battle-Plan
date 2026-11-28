import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { artHandleKey, readyArt } from '../../families/art/types.ts';
import type { ArtHandle, IdentityResult } from '../../families/plan.ts';
import { cellBarFraction, resolveTableRows } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY, tableDefaults } from '../../families/table/settings.ts';
import type { LeaderRow, UnitLeadersReply } from '../../generated/dashboard';
import { liveKey } from '../../charts/live.ts';
import { catalogueEntries, widgetGroups } from '../../dashboard/widgetCategories.ts';
import { followedTitle, resolveOptions } from '../../dashboard/options.ts';
import type { FamilyPlanContext } from '../../families/plan.ts';
import '../definitions.ts';
import { familyViewOptionFields, presetWidget, type PresetWidget, resolveWidgetActiveView, resolveWidgetViewSwitch, widgetOptionFields } from '../manifest.ts';
import { COLUMNS, definition, TABLE_BASE } from './definition.ts';
import { generalsDeclaration, generalsProjection, leaderSelection, type GeneralsOptions } from './generalsProjection.ts';

const land = { owner: 'GER', kind: 1, category: 0, rank: 'General', attributeNames: ['Attack', 'Defense', 'Planning', 'Logistics'] as LeaderRow['role']['attributeNames'], maximumSkill: 9 };
const navy = { ...land, owner: 'ENG', kind: 2, rank: 'Admiral', attributeNames: ['Attack', 'Defense', 'Maneuvering', 'Coordination'] as LeaderRow['role']['attributeNames'] };

function leader(reference: string, role: LeaderRow['role'], extra: Partial<LeaderRow> = {}): LeaderRow {
  return { reference, name: `Leader ${reference}`, portrait: { image: `${reference}.png`, missing: null }, role, skill: 3, components: [4, 5, 6, 7], traits: [], traitDates: [], firstSeenHour: 10, fought: 4, won: 3, lost: 1, ...extra };
}

const reply = (leaders: LeaderRow[], partialFrom: number | null = null): UnitLeadersReply => ({
  leaders, traits: [], withheld: [], missing: null, battlesPartialFrom: partialFrom, resultsMissing: 'Results not recorded',
  skillIcons: {
    land: [{ key: 'GFX_mod_skill', frame: 0 }, { key: 'GFX_mod_attack', frame: 0 }, null, { key: 'GFX_mod_planning', frame: 0 }, { key: 'GFX_mod_logistics', frame: 0 }],
    navy: [{ key: 'GFX_mod_skill', frame: 0 }, { key: 'GFX_mod_naval_attack', frame: 0 }, null, null, null],
  },
  warnings: [],
});

const OPTIONS: GeneralsOptions = { filter: { passing: null, unrecorded: [], bounds: [] }, kind: 'land' };
const ADMIRALS: GeneralsOptions = { ...OPTIONS, kind: 'navy' };

function view(data: UnitLeadersReply, names: Record<string, [string, number]> = {}) {
  const art = (handle: ArtHandle) => readyArt(`art:${artHandleKey(handle)}`);
  const identity = (kind: IdentityResult['kind'], key: string, atHour: number): IdentityResult => ({
    kind, key, holder: { status: 'pending' },
    name: names[key] ? { status: 'ready', value: names[key]![0] } : { status: 'pending' },
    validInterval: [atHour, names[key]?.[1] ?? Infinity], nextChangeAt: names[key]?.[1] ?? Infinity,
  });
  return createBoundedReadView({
    hour: 100,
    panels: new Map<string, unknown>([['main', data]]),
    art: { resolve: art },
    identity: { resolve: identity },
  });
}

test('one kind of leader per table: Generals shows only army leaders and Admirals only navy leaders, each with its own recorded skill names on one line and icons', () => {
  const mixed: UnitLeadersReply = {
    ...reply([leader('1', land), leader('2', navy)]),
    withheld: [{ reference: '8', owner: 'GER', kind: 1, name: null, reason: 'not decided' }, { reference: '9', owner: 'ENG', kind: 2, name: null, reason: 'not decided' }, { reference: '7', owner: 'ENG', kind: 2, name: null, reason: 'not decided' }],
  };
  const headers = (model: ReturnType<typeof generalsProjection.project>) =>
    model.columns.filter(column => ['attack', 'defense', 'third', 'fourth'].includes(column.key)).map(column => [column.label, column.unit]);
  const url = (art: { readonly status: string; readonly url?: string } | undefined) => (art?.status === 'ready' ? art.url : undefined);
  const generals = generalsProjection.project(view(mixed), OPTIONS);
  assert.deepEqual(generals.rows.map(row => row.key), ['1']);
  assert.deepEqual(headers(generals), [['Attack', undefined], ['Defense', undefined], ['Planning', undefined], ['Logistics', undefined]]);
  assert.equal(url(generals.rows[0]!.cells.skill!.art), `art:${artHandleKey({ kind: 'sprite', key: 'GFX_mod_skill', frame: 0 })}`);
  assert.equal(url(generals.rows[0]!.cells.attack!.art), `art:${artHandleKey({ kind: 'sprite', key: 'GFX_mod_attack', frame: 0 })}`);
  assert.equal(generals.rows[0]!.cells.defense!.art, undefined, 'an icon the interface lacks is no icon');
  assert.equal(generals.titleSummary, '1 not shown', 'the header marker counts only generals');
  assert.equal(generals.titleSummaryPartial, '1 leader of these countries can’t be shown; Details lists why.\nLeader 8: not decided', 'the header marker carries the full sentence and per-leader reason');
  assert.ok(generals.warnings?.includes('Leader 8: not decided'), 'Details still lists the per-leader reason');
  assert.equal('notes' in generals, false, 'no body note row is emitted');
  const admirals = generalsProjection.project(view(mixed), ADMIRALS);
  assert.deepEqual(admirals.rows.map(row => row.key), ['2']);
  assert.deepEqual(headers(admirals), [['Attack', undefined], ['Defense', undefined], ['Maneuvering', undefined], ['Coordination', undefined]]);
  assert.equal(url(admirals.rows[0]!.cells.attack!.art), `art:${artHandleKey({ kind: 'sprite', key: 'GFX_mod_naval_attack', frame: 0 })}`);
  assert.equal(admirals.rows[0]!.cells.fourth!.art, undefined);
  assert.equal(admirals.titleSummary, '2 not shown', 'the header marker counts only admirals');
  assert.equal(admirals.titleSummaryPartial, '2 leaders of these countries can’t be shown; Details lists why.\nLeader 9: not decided\nLeader 7: not decided', 'the header marker carries every withheld admiral reason');
  assert.equal('notes' in admirals, false, 'no body note row is emitted');
  const modded = generalsProjection.project(view(reply([leader('3', { ...navy, attributeNames: ['Gunnery', 'Screening', 'Manoeuvre', 'Command'] })])), ADMIRALS);
  assert.deepEqual(headers(modded).map(([label]) => label), ['Gunnery', 'Screening', 'Manoeuvre', 'Command'], 'a mod’s recorded names are the headers');
});

test('the header toggle is the manifest view switch, Generals | Admirals, and the stored view is the kind the read asks for', () => {
  assert.deepEqual(resolveWidgetViewSwitch('generals')?.choices, [{ value: 'land', label: 'Generals' }, { value: 'navy', label: 'Admirals' }]);
  const asked = (stored: Record<string, string>) => {
    const shown = resolveWidgetActiveView('generals', stored);
    const ctx = {
      options: resolveOptions(familyViewOptionFields('generals', shown), stored),
      branch: { id: '1' },
      countries: { filter: { passing: null, unrecorded: [], bounds: [] } },
      track: { source: { id: 's', revision: '1' }, axisOffset: 0 },
    } as unknown as FamilyPlanContext<unknown>;
    const read = generalsDeclaration.sources(ctx, undefined as never).entries[0]!.read as unknown as { readonly params: { readonly leaders: string } };
    return [shown, read.params.leaders, generalsDeclaration.projectionOptions(ctx as never).kind];
  };
  assert.deepEqual(asked({}), ['land', 'land', 'land']);
  assert.deepEqual(asked({ leaders: 'navy' }), ['navy', 'navy', 'navy']);
  assert.deepEqual(asked({ leaders: 'land' }), ['land', 'land', 'land']);
  assert.deepEqual(asked({ leaders: 'all' }), ['land', 'land', 'land'], 'a layout saved with the old mixed table opens as Generals');
  assert.equal(widgetOptionFields('generals', { leaders: 'navy' }).find(field => field.key === TABLE_SETTINGS_KEY)?.storeAs, `${TABLE_SETTINGS_KEY}@navy`, 'each view keeps its own table settings');
});

test('the catalogue offers Generals under Army and Admirals under Navy, each placing its own view, and the title follows the view', () => {
  const groups = widgetGroups([definition]);
  assert.deepEqual(groups.map(group => group.label), ['Army', 'Navy']);
  const [army, navy] = catalogueEntries(groups, new Map());
  assert.deepEqual(army!.map(entry => [entry.title, entry.group.label, entry.match]), [['Generals', 'Army', { leaders: 'land' }]]);
  assert.deepEqual(navy!.map(entry => [entry.title, entry.group.label, entry.match]), [['Admirals', 'Navy', { leaders: 'navy' }]]);
  const placed = presetWidget('generals', navy![0]!.initial as PresetWidget);
  assert.equal(placed.title, 'Admirals');
  assert.equal(resolveWidgetActiveView('generals', placed.options), 'navy');
  const field = definition.options.find(item => item.key === 'leaders');
  assert.equal(followedTitle(field, 'Generals', definition.title, 'land', 'navy'), 'Admirals');
  assert.equal(followedTitle(field, 'Admirals', definition.title, 'navy', 'land'), 'Generals');
  assert.equal(followedTitle(field, 'My navy', definition.title, 'navy', 'land'), 'My navy', 'a title the user wrote is kept');
});

test('equal-skill leaders keep the product’s order rather than alphabetical tie-breaking', () => {
  const model = generalsProjection.project(view(reply([
    leader('1', land, { name: 'Tomoyuki Yamashita', skill: 7, won: 1071 }),
    leader('2', land, { name: 'Hisaichi Terauchi', skill: 7, won: 831 }),
  ])), OPTIONS);
  const settings = tableDefaults(COLUMNS, TABLE_BASE);
  const flat = resolveTableRows(model, settings).filter(row => row.kind === 'data');
  assert.deepEqual(flat.map(row => row.key), ['1', '2']);
});

test('commander names and portraits advance with the followed roster', () => {
  const readView = view(reply([
    leader('1', land, { name: 'Erwin Rommel' }),
    leader('2', land, { name: null, portrait: { image: null, missing: 'Portrait holder not recorded' } }),
  ]), { 'GER:1': ['Older name', 250] });
  const model = generalsProjection.project(readView, OPTIONS);
  const [row, unnamed] = model.rows;
  assert.deepEqual(row!.cells.commander!.value, { kind: 'known', value: 'Erwin Rommel' });
  assert.deepEqual(row!.cells.country!.value, { kind: 'pending', reason: 'Name is loading' });
  assert.deepEqual(unnamed!.cells.commander!.value, { kind: 'notRecorded', reason: 'Character name unavailable' });
  assert.deepEqual(unnamed!.cells.portrait!.art, { status: 'unavailable', reason: 'Portrait holder not recorded' });
  assert.equal(row!.cells.commander!.subtext, 'General');
  assert.equal(row!.cells.portrait!.art?.status === 'ready' ? row!.cells.portrait!.art.url : undefined, `art:${artHandleKey({ kind: 'url', url: '1.png' })}`);
  assert.equal(generalsProjection.nextChange!(readView, OPTIONS), Infinity);
  const renamed = generalsProjection.project(view(reply([leader('1', land, { name: 'New recorded name', portrait: { image: 'new.png', missing: null } })])), OPTIONS);
  assert.deepEqual(renamed.rows[0]!.cells.commander!.value, { kind: 'known', value: 'New recorded name' });
  const nextPortrait = renamed.rows[0]!.cells.portrait!.art;
  assert.equal(nextPortrait?.status === 'ready' ? nextPortrait.url : undefined, `art:${artHandleKey({ kind: 'url', url: 'new.png' })}`);
});

test('battle counters reorder equal-skill leaders without a new roster reply', () => {
  const data = reply([
    leader('1', land, { won: 0, fought: 0, lost: 0, battleOffset: [5, 2, 1] }),
    leader('2', land, { won: 0, fought: 0, lost: 0, battleOffset: [0, 0, 0] }),
  ]);
  const scores = new Map<string, number>();
  const read = () => createBoundedReadView({ hour: 100, panels: new Map([['main', data]]), live: new Map([['main', { get: key => scores.get(key), missing: () => 'Battles not recorded at this hour' }]]) });
  for (const [key, fought, won, lost] of [['1', 1, 0, 1], ['2', 3, 1, 1]] as const) {
    for (const [column, count] of [['fought', fought], ['won', won], ['lost', lost]] as const) scores.set(liveKey(key, column), count);
  }
  const ranked = () => generalsProjection.project(read(), OPTIONS).rows;
  assert.deepEqual(ranked().map(row => row.key), ['1', '2']);
  assert.deepEqual(ranked()[0]?.cells.won?.value, { kind: 'known', value: 2 });
  scores.set(liveKey('2', 'won'), 3);
  assert.deepEqual(ranked().map(row => row.key), ['2', '1']);
  scores.delete(liveKey('2', 'won'));
  assert.deepEqual(ranked().find(row => row.key === '2')?.cells.won?.value, { kind: 'notRecorded', reason: 'Battles not recorded at this hour' });
});

test('battle counts from a partly recorded battle log are partial and withhold win rate and its rank; exact logs split won against lost', () => {
  const exact = generalsProjection.project(view(reply([leader('1', land)], 150)), OPTIONS);
  const exactCells = exact.rows[0]!.cells;
  assert.equal(exactCells.won!.value.kind, 'known');
  assert.deepEqual([exactCells.winRate!.value, exactCells.winRate!.secondaryValue], [{ kind: 'known', value: 3 }, { kind: 'known', value: 1 }]);
  assert.deepEqual(exactCells.winRate!.sortValue, { kind: 'known', value: 75 });

  const partial = generalsProjection.project(view(reply([leader('2', land, { won: 4, lost: 0 })], 50)), OPTIONS);
  const cells = partial.rows[0]!.cells;
  assert.equal(cells.won!.value.kind, 'partial');
  assert.equal(cells.lost!.value.kind, 'partial');
  assert.equal(cells.winRate!.value.kind, 'notRecorded', 'partial battle counts withhold the derived win rate');
  assert.equal(cells.winRate!.secondaryValue?.kind, 'notRecorded');
  assert.equal(cells.winRate!.sortValue?.kind, 'notRecorded', 'partial battle counts withhold the win rate rank');
  for (const key of ['won', 'lost', 'fought', 'unresolved']) {
    const cell = cells[key]!;
    assert.equal(cellBarFraction(cell, cell.value, 1), null, `${key} does not scale a partly recorded count into a bar`);
    assert.equal(cell.value.kind, 'partial');
    assert.equal(cell.value.kind === 'partial' && cell.value.reason, 'Battle counts are incomplete: some battles were not recorded.');
    assert.equal(exactCells[key]!.bar, undefined, `${key} keeps its usual complete-data bar`);
  }

  const mixedModel = {
    ...exact,
    rows: [partial.rows[0]!, exact.rows[0]!],
  };
  const byWinRate = tableDefaults(COLUMNS, { ...TABLE_BASE, sort: { column: 'winRate', direction: 'descending' } });
  const ranked = resolveTableRows(mixedModel, byWinRate).filter(row => row.kind === 'data');
  assert.deepEqual(ranked.map(row => row.key), ['1', '2'], 'exact win rate ranks ahead of withheld partial win rate');
});

test('the read asks only for the countries the filter ever passes', () => {
  assert.deepEqual(leaderSelection({ passing: null, unrecorded: [], bounds: [] }), { kind: 'all' });
  const asked = leaderSelection({ passing: { ITA: [[0, 5]], GER: [[3, 9]] }, unrecorded: [], bounds: [] });
  assert.equal(asked.kind, 'countries');
  assert.equal(asked.kind === 'countries' ? asked.countries.length : 0, 2);
});
