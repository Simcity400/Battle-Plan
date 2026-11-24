import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { artHandleKey, readyArt } from '../../families/art/types.ts';
import type { ArtHandle, IdentityResult } from '../../families/plan.ts';
import { resolveTableRows } from '../../families/table/model.ts';
import { tableDefaults } from '../../families/table/settings.ts';
import type { LeaderRow, UnitLeadersReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';
import { generalsProjection, leaderSelection, type GeneralsOptions } from './generalsProjection.ts';

const land = { owner: 'GER', kind: 1, category: 0, rank: 'General', attributeNames: ['Attack', 'Defense', 'Planning', 'Logistics'] as LeaderRow['role']['attributeNames'], maximumSkill: 9 };
const navy = { ...land, owner: 'ENG', kind: 2, rank: 'Admiral', attributeNames: ['Attack', 'Defense', 'Maneuvering', 'Coordination'] as LeaderRow['role']['attributeNames'] };

function leader(reference: string, role: LeaderRow['role'], extra: Partial<LeaderRow> = {}): LeaderRow {
  return { reference, name: `Leader ${reference}`, role, skill: 3, components: [4, 5, 6, 7], traits: [], traitDates: [], firstSeenHour: 10, fought: 4, won: 3, lost: 1, ...extra };
}

const reply = (leaders: LeaderRow[], partialFrom: number | null = null): UnitLeadersReply => ({
  leaders, traits: [], withheld: [], missing: null, battlesPartialFrom: partialFrom, resultsMissing: 'Results not recorded',
  skillIcons: {
    land: [{ key: 'GFX_mod_skill', frame: 0 }, { key: 'GFX_mod_attack', frame: 0 }, null, { key: 'GFX_mod_planning', frame: 0 }, { key: 'GFX_mod_logistics', frame: 0 }],
    navy: [{ key: 'GFX_mod_skill', frame: 0 }, { key: 'GFX_mod_naval_attack', frame: 0 }, null, null, null],
  },
  warnings: [],
});

const OPTIONS: GeneralsOptions = { filter: { passing: null, unrecorded: [], bounds: [] }, kind: 'all' };

function view(data: UnitLeadersReply, names: Record<string, [string, number]> = {}) {
  const art = (handle: ArtHandle) => readyArt(`art:${artHandleKey(handle)}`);
  const identity = (kind: IdentityResult['kind'], key: string, atHour: number): IdentityResult => ({
    kind, key, atHour: atHour as IdentityResult['atHour'], holder: { status: 'pending' },
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

test('skill cells draw the product’s icons for each leader’s own window; an icon the interface lacks is no icon', () => {
  const model = generalsProjection.project(view(reply([leader('1', land), leader('2', navy)])), OPTIONS);
  const [general, admiral] = [model.rows.find(row => row.key === '1')!, model.rows.find(row => row.key === '2')!];
  assert.equal(general.cells.skill!.art?.status === 'ready' ? general.cells.skill!.art.url : undefined, `art:${artHandleKey({ kind: 'sprite', key: 'GFX_mod_skill', frame: 0 })}`);
  assert.equal(general.cells.attack!.art?.status === 'ready' ? general.cells.attack!.art.url : undefined, `art:${artHandleKey({ kind: 'sprite', key: 'GFX_mod_attack', frame: 0 })}`);
  assert.equal(general.cells.defense!.art, undefined);
  assert.equal(admiral.cells.attack!.art?.status === 'ready' ? admiral.cells.attack!.art.url : undefined, `art:${artHandleKey({ kind: 'sprite', key: 'GFX_mod_naval_attack', frame: 0 })}`);
  assert.equal(admiral.cells.fourth!.art, undefined);
  assert.deepEqual(
    model.columns.filter(column => column.key === 'third' || column.key === 'fourth').map(column => [column.label, column.unit]),
    [['Planning', '/ Maneuvering'], ['Logistics', '/ Coordination']],
  );
  const landOnly = generalsProjection.project(view(reply([leader('1', land)])), { ...OPTIONS, kind: 'land' });
  assert.deepEqual(
    landOnly.columns.filter(column => column.key === 'third' || column.key === 'fourth').map(column => [column.label, column.unit]),
    [['Planning', undefined], ['Logistics', undefined]],
  );
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

test('a commander is named as at the hour, the portrait is the art cache’s, and a rename is the next change', () => {
  const readView = view(reply([leader('1', land)]), { 'GER:1': ['Erwin Rommel', 250] });
  const model = generalsProjection.project(readView, OPTIONS);
  const row = model.rows[0]!;
  assert.deepEqual(row.cells.commander!.value, { kind: 'known', value: 'Erwin Rommel' });
  assert.equal(row.cells.commander!.subtext, 'General');
  assert.equal(row.cells.portrait!.art?.status === 'ready' ? row.cells.portrait!.art.url : undefined, `art:${artHandleKey({ kind: 'portrait', leader: '1', hour: 10 })}`);
  assert.equal(generalsProjection.nextChange!(readView, OPTIONS), 250);
});

test('battle counts from a partly recorded battle log are partial; the win rate splits won against lost', () => {
  const model = generalsProjection.project(view(reply([leader('1', land)], 50)), OPTIONS);
  const cells = model.rows[0]!.cells;
  assert.equal(cells.won!.value.kind, 'partial');
  assert.deepEqual([cells.winRate!.value, cells.winRate!.secondaryValue], [{ kind: 'known', value: 3 }, { kind: 'known', value: 1 }]);
});

test('the read asks only for the countries the filter ever passes', () => {
  assert.deepEqual(leaderSelection({ passing: null, unrecorded: [], bounds: [] }), { kind: 'all' });
  const asked = leaderSelection({ passing: { ITA: [[0, 5]], GER: [[3, 9]] }, unrecorded: [], bounds: [] });
  assert.equal(asked.kind, 'countries');
  assert.equal(asked.kind === 'countries' ? asked.countries.length : 0, 2);
});
