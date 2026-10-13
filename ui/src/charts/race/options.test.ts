import test from 'node:test';
import assert from 'node:assert/strict';
import { raceViewOption, raceViewVariants } from './options.ts';

test('each race view other than the default is a catalogue card with a plain title', () => {
  const cards = (title: string, ...args: Parameters<typeof raceViewOption>) =>
    raceViewVariants(title, [raceViewOption(...args)]).map(item => [item.initial.title, item.initial.options]);
  assert.deepEqual(cards('Faction strength'), [['Faction strength over time', { view: 'lines' }]]);
  assert.deepEqual(cards('Factories', 'type'), [['Factories over time', { view: 'lines' }], ['Factories: One country by type', { view: 'mix' }]]);
  assert.deepEqual(cards('Occupation trend', 'measure', 'lines'), [['Occupation trend race', { view: 'race' }], ['Occupation trend: One country by measure', { view: 'mix' }]],
    'a widget drawing lines by default offers its race');
  assert.deepEqual(cards('Territory', undefined, 'race', [{ value: 'share', label: 'Share' }]), [['Territory over time', { view: 'lines' }], ['Territory share', { view: 'share' }]]);
  assert.deepEqual(raceViewVariants('Army size', [raceViewOption('unit group')], { mix: 'Army composition' }).map(item => item.initial.title),
    ['Army size over time', 'Army composition'], 'a widget can name its own views');
  assert.deepEqual(raceViewVariants('Plain', []), [], 'no View setting, no view cards');
});
