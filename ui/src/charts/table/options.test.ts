import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions, setOption, validOption } from '../../dashboard/options.ts';
import { tableSortChoices, tableSortOf, tableSortOptions, tableSortToken } from './options.ts';

test('content-dependent table sorts persist through settings and header tokens', () => {
  const fallback = { column: 'factories', direction: 'descending' } as const;
  const columns = [{ key: 'factories', label: 'Recorded factory label' }, { key: 'mod column', label: 'Recorded measure' },
    { key: 'portrait', label: 'Portrait', sortable: false }];
  const fields = tableSortOptions(columns, fallback, true);
  const choices = tableSortChoices(columns);
  assert.equal(choices.length, 4);
  assert.equal(choices[0].label, 'Recorded factory label · descending');
  const next = { column: 'mod column', direction: 'ascending' } as const;
  const token = tableSortToken(next);
  assert.ok(validOption(fields[0], token));
  const saved = setOption(fields, undefined, 'tableSort', token);
  assert.ok(saved);
  assert.deepEqual(tableSortOf(resolveOptions(fields, saved).tableSort, fallback), next);
  assert.deepEqual(tableSortOf(resolveOptions(fields).tableSort, fallback), fallback);
  assert.deepEqual(tableSortOf(JSON.stringify([next.column, next.direction]), fallback), next);
  const ascending = tableSortOptions(columns, { column: 'mod column', direction: 'ascending' }, true)[0];
  assert.ok(ascending.kind === 'content-choice');
  assert.equal(ascending.automatic, 'Default column · ascending');
});

test('malformed sort tokens keep the declared fallback', () => {
  const fallback = { column: 'factories', direction: 'descending' } as const;
  for (const value of [null, '', 'broken', '["factories","sideways"]', '["factories"]', '[1,"ascending"]']) {
    assert.deepEqual(tableSortOf(value, fallback), fallback);
  }
});

test('static and content sort settings use the same choices and labels', () => {
  const columns = [{ key: 'country', label: 'Country' }, { key: 'amount', label: 'Recorded amount' },
    { key: 'portrait', label: 'Portrait', sortable: false }];
  const fallback = { column: 'amount', direction: 'descending' } as const;
  const field = tableSortOptions(columns, fallback)[0];
  assert.ok(field.kind === 'choice');
  const contentChoices = tableSortChoices(columns);
  assert.deepEqual(field.choices.map(choice => choice.label), contentChoices.map(choice => choice.label));
  assert.deepEqual(field.choices.map(choice => tableSortOf(choice.value, fallback)),
    contentChoices.map(choice => tableSortOf(choice.value, fallback)));
});
