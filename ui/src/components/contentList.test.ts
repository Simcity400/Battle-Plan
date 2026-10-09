import assert from 'node:assert/strict';
import test from 'node:test';
import { contentChoices, contentWindow, filterContent } from './contentList.ts';
import { COUNTRY_ROW_HEIGHT, COUNTRY_ROW_OVERSCAN, moveIndex, revealRow } from './countryList.ts';
import { contentSet, toggleContentSet } from '../dashboard/options.ts';
import { OPTIONS } from '../widgets/division-log/definition.ts';

test('the division template setting uses the complete virtual catalogue at 5000 choices', () => {
  const field = OPTIONS.find(field => field.key === 'templates');
  assert.ok(field?.kind === 'content-set' && field.catalogue);
  const inventory = Array.from({ length: 5000 }, (_, index) => ({ value: `template-${index}`, label: `Recorded template ${index}` }));
  const choices = contentChoices(inventory, inventory.map(choice => choice.value));
  assert.equal(choices.length, 5000);
  const height = 340;
  const maximum = Math.ceil(height / COUNTRY_ROW_HEIGHT) + 1 + 2 * COUNTRY_ROW_OVERSCAN;
  for (const scrollTop of [0, 1000 * COUNTRY_ROW_HEIGHT, 5000 * COUNTRY_ROW_HEIGHT - height]) {
    const rendered = contentWindow(choices, scrollTop, height);
    assert.ok(rendered.rows.length > 0 && rendered.rows.length <= maximum);
    assert.equal(rendered.rows[0], choices[rendered.start]);
  }
  // Search and keyboard navigation can reach the last choice without mounting
  // the preceding choices, and selecting it stores its exact token.
  assert.deepEqual(filterContent(choices, 'template-4999').map(row => row.value), ['template-4999']);
  const last = moveIndex('End', 0, choices.length)!;
  const rendered = contentWindow(choices, revealRow(last, 0, height), height);
  assert.ok(rendered.rows.some(row => row.value === 'template-4999'));
  assert.deepEqual(contentSet(toggleContentSet('', 'template-4999', Number.MAX_SAFE_INTEGER)!), ['template-4999']);
});

test('catalogue search keeps saved absent choices removable and refuses unstorable choices', () => {
  const choices = contentChoices([{ value: 'present', label: 'Present' }, { value: 'bad token', label: 'Cannot save' }], ['absent']);
  const absent = filterContent(choices, 'absent')[0];
  assert.equal(absent.label, 'absent (not in this campaign)');
  assert.equal(absent.unavailable, undefined);
  assert.equal(toggleContentSet('absent', absent.value, Number.MAX_SAFE_INTEGER), '');
  assert.ok(choices.find(row => row.value === 'bad token')?.unavailable);
});
