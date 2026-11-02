import assert from 'node:assert/strict';
import test from 'node:test';
import { CountryLabelIndex } from './countryLabelData.ts';
import { nameReadFrom, type NameRead } from './nameReads.ts';
import { deepFreeze } from '../testFrozen.ts';

test('dated label tables are prepared once per immutable source and lookup reuses their identities', () => {
  let reads = 0;
  const image = deepFreeze({ get url() { reads++; return 'synthetic flag'; } });
  const names: ReadonlyMap<string, NameRead> = deepFreeze(new Map([[JSON.stringify([100, 'country']), nameReadFrom('Name', true)], [JSON.stringify([101, 'country']), nameReadFrom('Name', true)]]));
  const flags = deepFreeze(new Map([[JSON.stringify([100, 'country']), image], [JSON.stringify([101, 'country']), image]]));
  const index = new CountryLabelIndex();
  const labels = index.at(names, flags);
  assert.equal(labels(100), labels(101));
  const before = reads;
  for (let lookup = 0; lookup < 100; lookup++) {
    assert.equal(index.at(names, flags), labels);
    assert.equal(labels(100)('country'), labels(101)('country'));
  }
  assert.equal(reads, before);
  assert.throws(() => (names as Map<string, NameRead>).set(JSON.stringify([100, 'country']), nameReadFrom('Changed', true)), TypeError);
  const changed = deepFreeze(new Map([[JSON.stringify([100, 'country']), nameReadFrom('Renamed', true)]]));
  assert.equal(index.at(changed, flags)(100)('country').name, 'Renamed');
  assert.equal(labels(100)('country').name, 'Name');
  assert.equal(index.at(deepFreeze(new Map()), deepFreeze(new Map()))(1)('country').flagPending, false);
});
