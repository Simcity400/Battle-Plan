import assert from 'node:assert/strict';
import test from 'node:test';
import { placeName, stateNamesOf } from './construction.ts';

test('a place is named from the reply’s states, and a state the content does not name says so', () => {
  const names = stateNamesOf([{ id: 5, name: 'Metro' }, { id: 6, name: 'Metro' }, { id: 7, name: null }]);
  assert.equal(placeName({ state: 5, toState: 6, railway: true }, names).label, 'Metro to Metro');
  assert.equal(placeName({ state: 5, toState: 5, railway: true }, names).label, 'Metro');
  const unnamed = placeName({ state: 7, railway: false }, names);
  assert.equal(unnamed.label, 'State 7');
  assert.match(unnamed.missing ?? '', /no name in the recorded content/);
  assert.match(placeName({ state: 9000, railway: false }, names).missing ?? '', /not named by the reply/);
  assert.equal(placeName({ state: null, province: 5007, railway: false }, names).label, 'province 5007');
  assert.equal(placeName({ state: null, railway: true }, names).label, 'an unknown route');
});

