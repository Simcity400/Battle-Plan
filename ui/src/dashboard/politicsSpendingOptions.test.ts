import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizePoliticsSpendingOptions, NO_SPENDING_CATEGORIES, POLITICS_SPENDING_OPTIONS, SPENDING_DEFAULT_SORT, spendingSort } from './politicsSpendingOptions.ts';
import { contentSet, resolveOptions, sectionFields, setOption, setToken } from './options.ts';
import { OptionDraft } from './optionDraft.ts';

const known = ['law/mod', 'advisor', 'none'].map(key => ({ value: setToken(key) }));
test('saved hide filters wait for the catalogue, then migrate once through composed option saves', () => {
  const saved = { categories: setToken('law/mod'), filterMode: 'hide' };
  assert.deepEqual(normalizePoliticsSpendingOptions(saved, undefined), { categories: saved.categories, migrated: false });
  const migrated = normalizePoliticsSpendingOptions(saved, known);
  assert.deepEqual(contentSet(migrated.categories), ['advisor', 'none']);
  const draft = new OptionDraft(saved);
  draft.update(POLITICS_SPENDING_OPTIONS, 'categories', migrated.categories);
  const stored = draft.update(POLITICS_SPENDING_OPTIONS, 'filterMode', 'shown')!;
  assert.equal(stored.filterMode, 'shown');
  assert.deepEqual(normalizePoliticsSpendingOptions(stored, known), { categories: migrated.categories, migrated: false });
});

test('show selections, legacy implicit hide, all and none retain their meanings without a mod token collision', () => {
  assert.equal(normalizePoliticsSpendingOptions({ categories: 'advisor', filterMode: 'show' }, known).categories, 'advisor');
  assert.equal(normalizePoliticsSpendingOptions({ categories: 'advisor' }, known).categories, `${setToken('law/mod')} none`);
  assert.equal(normalizePoliticsSpendingOptions({}, known).categories, '');
  const allHidden = normalizePoliticsSpendingOptions({ categories: known.map(item => item.value).join(' ') }, known);
  assert.equal(allHidden.categories, NO_SPENDING_CATEGORIES);
  assert.equal(normalizePoliticsSpendingOptions({ categories: '', filterMode: 'show' }, known).categories, NO_SPENDING_CATEGORIES);
  assert.notEqual(NO_SPENDING_CATEGORIES, setToken('none'));
  assert.ok(setOption(POLITICS_SPENDING_OPTIONS, {}, 'categories', NO_SPENDING_CATEGORIES));
});

test('5000 known categories migrate without truncation', () => {
  const catalogue = Array.from({ length: 5000 }, (_, index) => ({ value: setToken(`mod category ${index}`) }));
  const normalized = normalizePoliticsSpendingOptions({ categories: catalogue[0].value, filterMode: 'hide' }, catalogue);
  assert.equal(contentSet(normalized.categories).length, 4999);
  assert.equal(contentSet(normalized.categories).at(-1), catalogue.at(-1)!.value);
});

test('spending tables offer table sort, summary and decimals settings, and keep a saved log order', () => {
  const fields = sectionFields(POLITICS_SPENDING_OPTIONS, 'options', resolveOptions(POLITICS_SPENDING_OPTIONS, {}));
  const keys = fields.map(field => field.key);
  for (const key of ['artwork', 'chips', 'categories', 'totals', 'decimals', 'cellBars', 'density', 'tableSort']) assert.ok(keys.includes(key), key);
  for (const key of ['order', 'months', 'rowAnimation', 'rankMarkers', 'filterMode']) assert.equal(keys.includes(key), false, key);
  assert.equal(fields.find(field => field.key === 'totals')!.label, 'Summary');
  assert.equal(fields.find(field => field.key === 'categories')!.label, 'Show categories');
  assert.equal(resolveOptions(POLITICS_SPENDING_OPTIONS, {}).decimals, 0, 'whole numbers by default');
  assert.deepEqual(spendingSort({}), SPENDING_DEFAULT_SORT);
  assert.deepEqual(spendingSort({ order: 'oldest' }), { column: 'date', direction: 'ascending' });
  assert.deepEqual(spendingSort({ order: 'cost' }), { column: 'amount', direction: 'descending' });
  assert.deepEqual(spendingSort({ order: 'cost', tableSort: JSON.stringify(['item', 'ascending']) }), { column: 'item', direction: 'ascending' });
});
