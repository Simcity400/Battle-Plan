import assert from 'node:assert/strict';
import test from 'node:test';
import { OPTIONS } from './definition.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';

test('shading settings count Automatic as one and retain preferences when hidden', () => {
  for (const mode of ['campaign', 'follow', 'pin', 'range'] as const) {
    for (const [stored, visible] of [
      [{ view: 'time', resources: '' }, true],
      [{ view: 'time', resources: 'mod_resource' }, true],
      [{ view: 'time', resources: 'mod_resource other_resource' }, false],
      [{ view: 'time', show: 'resource', resources: 'mod_resource' }, false],
      [{ view: 'bars', resources: 'mod_resource' }, false],
    ] as const) {
      const options = resolveOptions(OPTIONS, { ...stored, gap: false });
      const fields = sectionFields(OPTIONS, 'options', options, undefined, mode);
      assert.equal(fields.some(field => field.key === 'gap'), visible);
      assert.equal(resolveOptions(OPTIONS, options).gap, false);
      assert.equal(resolveOptions(OPTIONS, options).resources, stored.resources);
    }
  }
});
