import assert from 'node:assert/strict';
import test from 'node:test';
import { ABSENT_NAME, LOADING_NAME, nameReadFrom, nameReadFailure, nameReadMetadata, nameReadText, sameNameRead } from './nameReads.ts';
import { RefusedError } from './queryError.ts';

test('dated names distinguish an unrequested lookup, active read, absence, empty value and failure', () => {
  assert.equal(nameReadText(undefined), 'Country name unavailable');
  assert.equal(nameReadFrom(undefined, false), ABSENT_NAME);
  assert.equal(nameReadFrom(undefined, true), LOADING_NAME);
  assert.equal(nameReadFrom(null, true), ABSENT_NAME);
  assert.equal(nameReadText(LOADING_NAME), 'Loading name…');
  assert.equal(nameReadText(nameReadFrom('', true)), '');
  assert.equal(nameReadText(nameReadFrom('Recorded name', true)), 'Recorded name');
  assert.equal(nameReadText(undefined, 'Visible resolved name'), 'Visible resolved name');
  for (const read of [ABSENT_NAME, LOADING_NAME, nameReadFrom(new Error('Synthetic decode error'), true)]) {
    assert.notEqual(nameReadText(read, 'Visible resolved name'), 'Visible resolved name');
  }
  const refused = new RefusedError('Synthetic country styles were not recorded');
  for (const failure of [refused, new Error('Synthetic styles decoder failure')]) {
    const read = nameReadFrom(failure, true), shown = nameReadMetadata(read);
    assert.equal(nameReadFailure(read), failure);
    assert.equal(shown.nameFailure, failure);
    assert.equal(shown.nameDetail, failure.message);
    assert.equal(shown.name, failure === refused ? refused.text : 'Couldn’t load country name.');
    assert.equal(shown.known, false);
    assert.equal(shown.nameFailed, true);
    assert.equal(sameNameRead(read, nameReadFrom(failure, true)), true);
  }
  assert.equal(nameReadMetadata(LOADING_NAME).known, undefined);
  assert.equal(nameReadMetadata(ABSENT_NAME).known, false);
  const unnamed = nameReadFrom({ reason: 'localisation AAA_fascism: scripted trigger unavailable' }, true);
  assert.equal(unnamed.kind, 'absent');
  assert.equal(nameReadMetadata(unnamed).nameDetail, 'localisation AAA_fascism: scripted trigger unavailable');
  assert.equal(sameNameRead(unnamed, nameReadFrom({ reason: 'localisation AAA_fascism: scripted trigger unavailable' }, true)), true);
  assert.equal(sameNameRead(unnamed, ABSENT_NAME), false);
});
