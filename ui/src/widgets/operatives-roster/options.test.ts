import assert from 'node:assert/strict';
import test from 'node:test';
import { OPTIONS } from './definition.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';

test('departed is a campaign preference and keeps its saved value in other modes', () => {
  const options = resolveOptions(OPTIONS, { departed: false, showRank: false });
  for (const mode of ['campaign', 'follow', 'pin', 'range'] as const) {
    const fields = sectionFields(OPTIONS, 'options', options, undefined, mode);
    assert.equal(fields.some(field => field.key === 'departed'), mode === 'campaign');
    assert.ok(fields.some(field => field.key === 'showRank'));
    assert.equal(resolveOptions(OPTIONS, options).departed, false);
    assert.equal(resolveOptions(OPTIONS, options).showRank, false);
  }
});
