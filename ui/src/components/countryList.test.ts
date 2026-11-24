import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryMetadata } from '../shell/api';
import { allowedCountryChoices, COUNTRY_ROW_HEIGHT, COUNTRY_ROW_OVERSCAN, countryChoices, filterChoices, moveIndex, revealRow, visibleRange } from './countryList.ts';

const country = (index: number, branch = '1'): CountryMetadata =>
  ({ id: String(index), tag: `T${index}`, name: `Free Province of Westmarch-${index}`, observedBranchId: branch, observedHour: 100 });
const LIST = 340;
const windowRows = Math.ceil(LIST / COUNTRY_ROW_HEIGHT) + 1 + 2 * COUNTRY_ROW_OVERSCAN;

test('a picker over thousands of countries renders only the rows in view', () => {
  for (const count of [5_000, 65_535]) {
    const countries = Array.from({ length: count }, (_, index) => country(index));
    const choices = countryChoices([...countries, ...countries.map(entry => ({ ...entry, observedBranchId: '2' }))]);
    assert.equal(choices.length, count);
    assert.ok(choices.every(({ country }) => country.observedBranchId === '1'));
    for (const scrollTop of [0, 1_234 * COUNTRY_ROW_HEIGHT + 17, count * COUNTRY_ROW_HEIGHT - LIST]) {
      const { start, end } = visibleRange(choices.length, scrollTop, LIST);
      const rendered = choices.slice(start, end);
      assert.ok(rendered.length > 0 && rendered.length <= windowRows, `${count} rows at ${scrollTop}: ${rendered.length}`);
      const first = Math.floor(scrollTop / COUNTRY_ROW_HEIGHT);
      const last = Math.min(count - 1, Math.floor((scrollTop + LIST - 1) / COUNTRY_ROW_HEIGHT));
      assert.ok(start <= first && last < end, 'every row in view is rendered');
    }
    assert.deepEqual(visibleRange(count, count * COUNTRY_ROW_HEIGHT * 2, LIST), { start: count, end: count });
  }
});

test('choices keep the first entry per tag, sort by name and filter by a precomputed key', () => {
  const choices = countryChoices([
    { id: '1', tag: 'GER', name: 'Germany', observedBranchId: '1', observedHour: 5 },
    { id: '1', tag: 'GER', name: 'Germany', observedBranchId: '2', observedHour: 1 },
    { id: '2', tag: 'ENG', name: 'United Kingdom', observedBranchId: '1', observedHour: 5 },
    { id: '3', tag: 'D01', name: '', observedBranchId: '1', observedHour: 5 },
    { id: '4', tag: 'TUR', name: '', observedBranchId: '1', observedHour: 5 },
  ]);
  assert.deepEqual(choices.map(({ country }) => country.tag), ['D01', 'GER', 'TUR', 'ENG'], 'a country with no recorded name sorts by its tag');
  assert.equal(choices[1].country.observedBranchId, '1');
  assert.deepEqual(filterChoices(choices, '  kingdom ').map(({ country }) => country.tag), ['ENG']);
  assert.deepEqual(filterChoices(choices, 'd01').map(({ country }) => country.tag), ['D01']);
  assert.equal(filterChoices(choices, ''), choices);
  const shown = ({ country }: { country: { tag: string } }) => country.tag === 'GER' ? 'German Reich' : '';
  assert.deepEqual(filterChoices(choices, 'reich', shown).map(({ country }) => country.tag), ['GER'], 'the name shown at the playback date is searchable too');
  const many = countryChoices(Array.from({ length: 5_000 }, (_, index) => country(index)));
  assert.deepEqual(filterChoices(many, 'westmarch-4999').map(({ country }) => country.tag), ['T4999']);
});

test('keyboard navigation moves by index and reveals rows that are not rendered', () => {
  assert.equal(moveIndex('ArrowDown', -1, 5_000), 0);
  assert.equal(moveIndex('ArrowDown', 4_999, 5_000), 0);
  assert.equal(moveIndex('ArrowUp', 0, 5_000), 4_999);
  assert.equal(moveIndex('End', 3, 5_000), 4_999);
  assert.equal(moveIndex('Home', 3, 5_000), 0);
  assert.equal(moveIndex('Enter', 3, 5_000), null);
  assert.equal(moveIndex('ArrowDown', -1, 0), null);
  assert.equal(revealRow(4_999, 0, LIST), 5_000 * COUNTRY_ROW_HEIGHT - LIST);
  assert.equal(revealRow(0, 1_000, LIST), 0);
  assert.equal(revealRow(31, 1_000, LIST), 1_000);
});

test('drill choices retain later aliases absent from the first canonical-slot roster', () => {
  const metadata = [{ id: '1', tag: 'OLD', name: 'First holder', observedBranchId: '1', observedHour: 100 },
    { id: '2', tag: 'ITA', name: 'Second holder', observedBranchId: '1', observedHour: 100 }];
  const choices = allowedCountryChoices(metadata, ['GER', 'ITA', 'GER'], { branchId: '2', hour: 150 });
  assert.deepEqual(choices.map(choice => choice.country.tag).sort(), ['GER', 'ITA']);
  const renamed = choices.find(choice => choice.country.tag === 'GER')!.country;
  assert.equal(renamed.name, 'Country name unavailable');
  assert.equal(renamed.id, undefined);
  assert.equal(renamed.observedBranchId, '2');
  assert.equal(renamed.observedHour, 150);
  assert.deepEqual(filterChoices(choices, 'ger').map(choice => choice.country.tag), ['GER']);
  assert.deepEqual(countryChoices(metadata).map(choice => choice.country.tag), ['OLD', 'ITA']);
  const tags = Array.from({ length: 5000 }, (_, index) => `LATER${index}`);
  const many = allowedCountryChoices(metadata, tags, { branchId: '2', hour: 150 });
  assert.equal(many.length, 5000);
  const range = visibleRange(many.length, revealRow(4999, 0, LIST), LIST);
  assert.ok(range.end - range.start <= windowRows);
  assert.ok(filterChoices(many, 'later4999').some(choice => choice.country.tag === 'LATER4999'));
});

test('restricted chooser searches the historical name it displays, with an absent title before requesting and tag search retained', () => {
  const metadata = [{ id: '1', tag: 'ONE', name: 'New Republic', observedBranchId: '1', observedHour: 200 }];
  const at = { branchId: '1', hour: 100 };
  const before = allowedCountryChoices(metadata, ['ONE', 'LATER'], at);
  assert.equal(before.find(choice => choice.country.tag === 'ONE')!.country.name, 'Country name unavailable');
  assert.deepEqual(filterChoices(before, 'New Republic'), []);
  const seen = new Map([['ONE', { kind: 'value' as const, value: 'Old Empire' }]]);
  const choices = allowedCountryChoices(metadata, ['ONE', 'LATER'], at, seen);
  assert.deepEqual(filterChoices(choices, 'Old Empire').map(choice => choice.country.tag), ['ONE']);
  assert.equal(filterChoices(choices, 'Old Empire')[0].country.name, 'Old Empire');
  assert.deepEqual(filterChoices(choices, 'New Republic'), []);
  assert.equal(filterChoices(choices, 'LATER')[0].country.name, 'Country name unavailable');
  assert.deepEqual(choices.map(choice => choice.country.tag), before.map(choice => choice.country.tag), 'loaded names do not move rows and start more reads');
  assert.deepEqual(filterChoices(allowedCountryChoices(metadata, ['ONE'], { ...at, hour: 200 }), 'Old Empire'), []);
});
