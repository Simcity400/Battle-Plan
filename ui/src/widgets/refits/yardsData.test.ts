import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { fixedText, hullShare, refitShare, refitsMissing, refitsStructure, refitsTexts, repairsStructure, repairsTexts, shipLabel, validateRefits, validateRepairs } from './yardsData.ts';

const ship = (handle: string, hull: string | null, name: string | null = null, nameRebuilt: string | null = null) => ({ handle, name, nameRebuilt, design: 'Destroyer I',
  class: 'Destroyer', hull, maximum: '200000', taskForce: null, policy: 'Repair when damaged' });
const repairs = validateRepairs({ ports: [{ province: 100, name: 'Kiel', priority: 2, policy: 'Highest priority', dockyards: 4, queue: [ship('5', '100000', 'Z1'), ship('6', null)] }],
  usedDockyards: 4, repairLimit: 10, queuedShips: 2, censusComplete: true, warnings: [] });
const refits = validateRefits({ lines: [{ handle: '70', target: 'Destroyer II', progress: '250000', cost: '1000000', dockyards: 3, remaining: '750000', ships: [ship('1', '200000')] }],
  ships: 1, dockyards: 3, costRead: 96, censusComplete: true, missing: null, warnings: [] });

const none = valuesOf(new Map<string, number>(), 'Not read yet');

test('a ship without its own recorded name shows its rebuilt name, else its design, else its class, each marked so; a hull without one is unknown, not zero', () => {
  assert.deepEqual(shipLabel(repairs.ports[0].queue[0]), { name: 'Z1', source: 'recorded' });
  assert.deepEqual(shipLabel(ship('7', null, null, 'Z 7 Hermann')), { name: 'Z 7 Hermann', source: 'rebuilt' });
  assert.deepEqual(shipLabel({ ...ship('8', null), design: null }), { name: 'Destroyer', source: 'design' });
  assert.deepEqual(shipLabel(repairs.ports[0].queue[1]), { name: 'Destroyer I', source: 'design' });
  assert.equal(hullShare(repairs.ports[0].queue[0]), 0.5);
  assert.equal(hullShare(repairs.ports[0].queue[1]), null);
  assert.equal(fixedText('123456'), '1.23');
});

test('a refit’s share needs a positive cost with a dated read', () => {
  assert.equal(refitShare(refits, refits.lines[0]), 0.25);
  assert.equal(refitShare({ ...refits, costRead: null }, refits.lines[0]), null);
  assert.equal(refitShare(refits, { ...refits.lines[0], cost: '0' }), null);
});

test('hourly progress and hulls are painted from their live cells; no value moves the structure, a row does', () => {
  const later = structuredClone(refits);
  later.lines[0].progress = '400000';
  assert.equal(refitsStructure(later), refitsStructure(refits), 'a value never renders');
  assert.equal(refitsTexts(refits, none).get('progress|70'), '25%', 'before a cell is read, the reply’s value');
  assert.equal(refitsTexts(refits, valuesOf(new Map([[liveKey('70', 'progress'), 500000]]), '')).get('progress|70'), '50%');
  const moved = structuredClone(repairs);
  moved.ports[0].queue[0].hull = '1';
  assert.equal(repairsStructure(moved), repairsStructure(repairs));
  assert.equal(repairsTexts(repairs, valuesOf(new Map([[liveKey('5', 'hull'), 150000]]), '')).get('hull|100|0'), '75%');
  assert.equal(repairsTexts(repairs, none).get('hull|100|1'), '—');
  moved.ports[0].queue.pop();
  assert.notEqual(repairsStructure(moved), repairsStructure(repairs));
});

test('refits not recorded at the hour carry their reason, which a render shows; a reply naming none is malformed', () => {
  const reason = 'Refit lines are not recorded at this hour: naval refit progress: not available (naval repairs could not be followed in the game)';
  const missing = validateRefits({ lines: [], ships: null, dockyards: null, costRead: null, censusComplete: false, missing: reason, warnings: [] });
  assert.equal(missing.missing, reason);
  assert.notEqual(refitsStructure(missing), refitsStructure({ ...missing, missing: null }), 'the reason renders');
  assert.deepEqual(refitsMissing(missing), { message: 'Refits were not recorded at this date.', reason }, 'plain words on screen, the reason in the tooltip');
  assert.equal(refitsMissing(refits), null);
  assert.throws(() => validateRefits({ ...refits, missing: undefined }), /malformed/);
});
