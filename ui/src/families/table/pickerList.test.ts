import assert from 'node:assert/strict';
import test from 'node:test';
import { appliedFacets, facetChoices, layoutEntries, type LogKindFilterItem } from './pickerList.ts';

const item = (value: string, group: string, extra: Partial<LogKindFilterItem> = {}): LogKindFilterItem => ({ value, label: value, group, checked: true, ...extra });

test('a filter list gives each group one heading, in the order the groups first appear, however the items interleave', () => {
  const laid = layoutEntries([item('war', 'Wars and peace'), item('coup', 'Politics'), item('peace', 'Wars and peace'), item('vote', 'Politics')], 32);
  assert.deepEqual(laid.entries.map(entry => entry.kind === 'group' ? `# ${entry.label}` : entry.item.value), ['# Wars and peace', 'war', 'peace', '# Politics', 'coup', 'vote']);
  assert.deepEqual(laid.options.map(entry => entry.kind === 'item' && entry.last), [false, true, false, true]);
});

test('a country or kind chosen before a seek no longer filters once the roster lacks it', () => {
  const choices = facetChoices([item('a', '', { country: 'BBB', countryName: 'Bravo', detail: 'Infantry' })]);
  assert.deepEqual(appliedFacets({ query: '', country: 'AAA', kind: 'Armour' }, choices), { country: '', kind: '' });
  assert.deepEqual(appliedFacets({ query: '', country: 'BBB', kind: 'Infantry' }, choices), { country: 'BBB', kind: 'Infantry' });
});
