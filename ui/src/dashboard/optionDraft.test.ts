import assert from 'node:assert/strict';
import test from 'node:test';
import { OptionDraft } from './optionDraft.ts';
import type { OptionField } from './options';

const fields: readonly OptionField[] = [
  { key: 'sort', label: 'Sort', section: 'options', kind: 'content-choice', default: '', automatic: 'Default' },
  { key: 'rank', label: 'Rank by', section: 'countries', kind: 'content-choice', default: '', automatic: 'Default' },
];

test('two saves from one header click retain both options before props arrive', () => {
  const supplied = { rank: 'factories', otherWidgetOption: true };
  const draft = new OptionDraft(supplied);
  assert.deepEqual(draft.update(fields, 'rank', 'territory'), { rank: 'territory', otherWidgetOption: true });
  draft.sync(supplied);
  assert.deepEqual(draft.update(fields, 'sort', 'country'), { rank: 'territory', sort: 'country', otherWidgetOption: true });
  assert.equal(draft.update(fields, 'rank', 1), null);
  assert.deepEqual(draft.update(fields, 'sort', ''), { rank: 'territory', otherWidgetOption: true });
});

test('a saved layout change or reset becomes the new option baseline', () => {
  const draft = new OptionDraft({ rank: 'factories' });
  draft.update(fields, 'sort', 'country');
  draft.sync({ rank: 'politicalPower' });
  assert.deepEqual(draft.update(fields, 'sort', 'territory'), { rank: 'politicalPower', sort: 'territory' });
  draft.sync(undefined);
  assert.deepEqual(draft.update(fields, 'rank', 'territory'), { rank: 'territory' });
});
