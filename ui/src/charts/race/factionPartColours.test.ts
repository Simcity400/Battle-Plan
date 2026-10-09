import assert from 'node:assert/strict';
import { test } from 'node:test';
import { FAILED_FACT, HourStore, nameKey, planRequests } from '../../dashboard/countryFacts.ts';
import type { CountryStyle } from '../../generated/dashboard';
import type { FactionCountry, FactionPart, FactionParts } from './factionParts';
import { factionColourKey, factionColourPlan, factionCountryColours, heldFactionColours } from './factionPartColours.ts';

const branch = { firstHour: 0, lastHour: 9, gaps: [] };
const census: FactionCountry[] = [
  { id: '[7,41]', witnesses: [{ tag: 'A', firstHour: 0, lastHour: 3, slot: 7, countryId: 41 },
    { tag: 'B', firstHour: 4, lastHour: 9, slot: 7, countryId: 41 }] },
  { id: '[8,42]', witnesses: [{ tag: 'A', firstHour: 4, lastHour: 9, slot: 8, countryId: 42 }] },
  { id: '[9,43]', witnesses: [{ tag: 'same name', firstHour: 0, lastHour: 9, slot: 9, countryId: 43 }] },
];
const model = (countries: FactionCountry[]): FactionParts => ({ census, parts: countries.map(country =>
  ({ ...country, runs: [], counted: [] } as FactionPart)), total: [], remainder: [], unidentified: 0 });
const style = (tag: string, color: CountryStyle['color']): CountryStyle => ({ tag, color, name: 'Same loaded name' });

test('native map colours and full-census correction survive faction filtering, aliases and duplicated names', () => {
  const full = factionColourPlan(model(census), branch, 5, true);
  const filtered = factionColourPlan(model([census[1]]), branch, 5, true);
  assert.deepEqual(full.selection, filtered.selection);
  const styles = new Map([[nameKey(4, 'B'), style('B', [20, 40, 60])],
    [nameKey(4, 'A'), style('A', [20, 40, 60])], [nameKey(0, 'same name'), style('same name', null)]]);
  const key = factionColourKey(full, styles)!;
  assert.equal(factionColourKey(filtered, styles), key);
  assert.deepEqual(factionCountryColours(key, '#ffffff', true), factionCountryColours(factionColourKey(filtered, styles)!, '#ffffff', true));
  assert.equal(factionCountryColours(key, '#ffffff', false).get('[7,41]'), '#14283c');
  assert.equal(factionCountryColours(key, '#ffffff', true).has('[9,43]'), false);
  const raw = factionColourPlan(model([census[0]]), branch, 5, false);
  assert.deepEqual(raw.wanted, [{ tag: 'B', hour: 4, stretch: 4 }]);
  assert.equal(factionColourPlan(model([census[1]]), branch, 2, false).wanted.length, 0,
    'the reused tag is never read at its former holder date');
});

test('pending fixed chunks retain only the same native alias/stretch; answered unknown colours stay neutral', () => {
  const before = factionColourPlan(model(census), branch, 3, true);
  const next = factionColourPlan(model(census), branch, 5, true);
  const colours = new Map([['[7,41]', '#14283c'], ['[8,42]', '#28465a'], ['[9,43]', '#7799aa']]);
  assert.equal(factionColourKey(next, new Map()), null);
  assert.deepEqual([...heldFactionColours(colours, before.selection, next.selection, new Map()).keys()], ['[9,43]']);
  assert.equal(heldFactionColours(colours, next.selection, next.selection,
    new Map([[nameKey(4, 'B'), FAILED_FACT]])).has('[7,41]'), false);
  assert.equal(heldFactionColours(colours, next.selection, next.selection,
    new Map([[nameKey(4, 'A'), style('A', null)]])).has('[8,42]'), false);
});

test('five thousand native holders use bounded style chunks while the full correction order stays fixed', () => {
  const many = Array.from({ length: 5000 }, (_, index): FactionCountry => ({ id: JSON.stringify([index, index]),
    witnesses: [{ tag: `mod ${index}`, firstHour: 0, lastHour: 9, slot: index, countryId: index }] }));
  const full = { ...model([]), census: many };
  const plan = factionColourPlan(full, branch, 5, true);
  assert.equal(plan.selection.size, 5000);
  assert.equal(plan.wanted.length, 5000);
  const chunks = planRequests(new HourStore<CountryStyle>(plan.maxHours), plan.wanted, plan.maxHours, 64);
  assert.equal(chunks.reduce((count, [, tags]) => count + tags.length, 0), 5000);
  assert.ok(chunks.every(([, tags]) => tags.length <= 64));
  assert.deepEqual([...factionColourPlan({ ...full, parts: [{ ...many[4999], runs: [], counted: [] }] }, branch, 5, true).selection.keys()],
    [...plan.selection.keys()]);
});
