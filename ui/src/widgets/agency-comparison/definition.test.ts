import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import type { CountrySelection } from '../../dashboard/model';
import { sortKey } from '../../charts/table/options.ts';
import { OPTIONS } from './definition.ts';

test('agency ranking is shown only for Top N, independently of table sorting', () => {
  const sorting = sortKey({ column: 'agency', direction: 'ascending' });
  const options = resolveOptions(OPTIONS, { topMeasure: 'active', tableSort: sorting });
  for (const countries of [{ kind: 'all' }, { kind: 'tags', tags: ['mod country'] },
    { kind: 'group', group: { type: 'humans' }, aggregate: false, membership: 'hour' },
    { kind: 'group', group: { type: 'top', count: 10 }, aggregate: false, membership: 'hour' }] as CountrySelection[]) {
    const fields = sectionFields(OPTIONS, 'options', options, countries);
    assert.equal(fields.some(field => field.key === 'topMeasure'), countries.kind === 'group' && countries.group.type === 'top');
    assert.equal(options.topMeasure, 'active');
    assert.equal(options.tableSort, sorting);
  }
});
