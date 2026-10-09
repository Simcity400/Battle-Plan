import assert from 'node:assert/strict';
import test from 'node:test';
import { catalogueWidth } from './labelCatalogue.ts';

test('campaign measurement uses glyph widths, all styles and their actual label space', () => {
  const labels = [
    { text: 'long narrow name', space: 38, weight: 600 },
    { text: 'WWW', space: 64, weight: 500 },
    { text: 'SIDE', space: 38, weight: 600, size: 10 },
  ];
  const measure = (label: typeof labels[number]) => label.text === 'WWW' ? 140 : 40;
  assert.equal(catalogueWidth(labels, measure), 204);
  assert.equal(catalogueWidth([], measure), 120);
});
