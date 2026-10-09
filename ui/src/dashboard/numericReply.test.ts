import assert from 'node:assert/strict';
import test from 'node:test';
import { NumericReply } from './numericReply.ts';

test('numeric projection is retained through pending labels and compared once per new reply', () => {
  const held = new NumericReply<{ amount: string; name: string | null }>();
  let projected = 0;
  const project = (reply: { amount: string; name: string | null }) => { projected++; return { ...reply, name: null }; };
  const first = { amount: '9007199254740993', name: 'First language' };
  const numbers = held.update('source/branch/actor', first, project);
  for (let frame = 0; frame < 500; frame++) assert.equal(held.update('source/branch/actor', first, project), numbers);
  assert.equal(projected, 1);
  assert.equal(held.update('source/branch/actor', undefined, project), numbers);
  assert.equal(held.update('source/branch/actor', { ...first, name: 'Other language' }, project), numbers);
  assert.equal(projected, 2);
  assert.notEqual(held.update('source/branch/actor', { amount: '9007199254740994', name: 'Other language' }, project), numbers);
  assert.equal(held.update('other source/branch/actor', undefined, project), undefined);
  assert.equal(held.update('source/branch/actor', undefined, project), undefined);
});
