import assert from 'node:assert/strict';
import { test } from 'node:test';
import { drawnViews } from './drawn.ts';
import type { Scene } from '../lane.ts';
import type { FamilyTableBinding } from '../../families/table/laneView.ts';

const scene = (model: unknown): Scene => ({ widget: 1, subs: [], pictures: [], model });
const table = drawnViews('table', () => {})!, race = drawnViews('race', () => {})!;
const reply = { missing: null, countries: [{ tag: 'GER', countryId: 1, total: 4, counts: [] }], types: [], warnings: [] };

test('astra 7: a race breaks at its membership change hour, not at midnight', () => {
  const model = { top: null, tags: ['GER'], split: false, overlay: false, offset: 0, reply, membership: new Map([['GER', [{ firstHour: 109, lastHour: 500 }]]]) };
  assert.equal(race.next!(scene(model), 100), 109, 'GER joins the selection at 109');
  assert.equal(race.next!(scene(model), 109), 501);
});

test('astra 7: a table breaks at its filter bounds and at the hours its country names and flags change', () => {
  const base = { family: 'table', kind: 'occupied-states', settings: {}, sources: {}, width: 100, height: 100, scrollLeft: 0, scrollTop: 0 };
  const filtered = {
    ...base,
    options: { filter: { passing: null, unrecorded: [], bounds: [0, 109, 300] }, role: 'occupier', labels: {}, lawAs: 'dot', filterAll: true },
  };
  assert.equal(table.next!(scene(filtered as unknown as FamilyTableBinding), 100), 109, 'the filter changes at 109, never sampled a day later');
  const named = {
    ...base,
    options: { filter: { passing: null, unrecorded: [], bounds: [] }, role: 'occupier', labels: { GER: [{ from: 0, name: 'German Reich' }, { from: 130, name: 'Germany' }] }, lawAs: 'dot', filterAll: true },
  };
  assert.equal(table.next!(scene(named as unknown as FamilyTableBinding), 100), 130, 'the name changes at 130');
});
