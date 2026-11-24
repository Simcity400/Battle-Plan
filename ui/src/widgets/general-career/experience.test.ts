import test from 'node:test';
import assert from 'node:assert/strict';
import { experienceText, experienceFactorText, experienceWhole } from './experience.ts';
test('commander experience retains signed fixed5 precision beyond safe integers', () => {
  assert.equal(experienceText('9223372036854775807', null), '92233720368547.75807');
  assert.equal(experienceText('-9223372036854775808', null), '-92233720368547.75808');
  assert.equal(experienceText('0', '300000'), '0.0 / 3.0');
  assert.equal(experienceWhole('666364027', '750000000'), '6,663 / 7,500', 'whole points never round up to what is needed');
});

test('training factor overrides retain sign and full i64 precision', () => {
  assert.equal(experienceFactorText('25000'), '+25.0%');
  assert.equal(experienceFactorText('-1'), '-0.001%');
  assert.equal(experienceFactorText('9223372036854775807'), '+9223372036854775.807%');
});
