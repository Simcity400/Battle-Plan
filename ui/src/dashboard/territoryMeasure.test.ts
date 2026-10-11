import assert from 'node:assert/strict';
import test from 'node:test';
import { MEASURES, territoryNote } from './territoryMeasure.ts';

test('the map-area caveat stays concise and belongs only to map area', () => {
  for (const { value } of MEASURES) {
    assert.equal(territoryNote(value), value === 'area' ? 'Map area exaggerates territory near the poles.' : undefined);
  }
});
