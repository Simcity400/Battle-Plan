import test from 'node:test';
import assert from 'node:assert/strict';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState.ts';
import { fuelExtentToken, fuelMoment, fuelOpen, fuelRecipientAtPick, fuelRecipientBasis, fuelRecipientForReply, fuelRecipientToken, readFuelExtent, readFuelRecipient, sameFuelActor } from './fuelDrill.ts';
import type { FuelRecipientSelection } from './fuelDrill';
import type { FuelRead } from '../../generated/dashboard';

const selected: FuelRecipientSelection = { campaign: 'c1', branch: 'b1', consumer: 1, key: 'opaque-native-name-key', continuityStartHour: 77,
  basis: { branchId: 'b1', tag: 'synthetic tag', hour: 100, country: '7', countryInstance: 0 } };

test('recipient pins preserve native supplying basis and refuse campaign, branch and same-instance reset reuse', () => {
  const token = fuelRecipientToken(selected);
  assert.deepEqual(readFuelRecipient(token, 'c1', 'b1'), selected);
  assert.equal(readFuelRecipient(token, 'c2', 'b1'), null);
  assert.equal(readFuelRecipient(token, 'c1', 'b2'), null);
  const read = { country: '7', countryInstance: 0, continuityStartHour: 77 } as FuelRead;
  assert.equal(sameFuelActor(selected, read), true);
  assert.equal(sameFuelActor(selected, { ...read, continuityStartHour: 101 }), false);
  assert.equal(sameFuelActor(selected, { ...read, countryInstance: 1 }), false);
  const extent = fuelExtentToken('c1', 'b1', [50, 500]);
  assert.deepEqual(readFuelExtent(extent, 'c1', 'b1'), [50, 500]);
  assert.equal(readFuelExtent(extent, 'c1', 'b2'), null);
});

test('recipient detail retains its native basis through a recorded alias, never a reused original tag', () => {
  const aliases = new Map([[selected.basis.tag, [{ firstHour: 100, lastHour: 123 }]], ['renamed', [{ firstHour: 124, lastHour: 200 }]]]);
  assert.equal(fuelRecipientForReply(selected, { branchId: 'b1', tag: 'renamed' }, 'c1', aliases), selected);
  assert.equal(fuelRecipientBasis({ branchId: 'b1', tag: 'renamed' }, null, selected, 'c1', aliases), selected.basis);
  assert.equal(fuelRecipientForReply({ ...selected, basis: { ...selected.basis, hour: 150 } }, { branchId: 'b1', tag: 'renamed' }, 'c1', aliases), null);
});

test('Campaign, Range and Pin picks keep the drawn day; Follow then tracks the actual clock until a day is picked', () => {
  for (const mode of ['Campaign', 'Range', 'Pin']) {
    const held = reduceDrill(CLOSED, fuelOpen('burn', 200, false, false, 100));
    assert.equal(fuelMoment(held, false, 200, 100), 200, mode);
    const followed = reduceDrill(held, { type: 'follow' });
    assert.equal(fuelMoment(followed, false, 200, 110), 110, mode);
    const picked = reduceDrill(followed, { type: 'pick', hour: 150 });
    assert.equal(fuelMoment(picked, false, 200, 120), 150, mode);
    assert.equal(fuelMoment(CLOSED, true, 200, 100), 200, mode);
  }
  assert.equal(fuelMoment(reduceDrill(CLOSED, fuelOpen('burn', 200, true, true, 100)), false, 200, 101), 101);
});

test('close A then reopen a drawn B line clears the selected A recipient before the actual lazy request seam', () => {
  const replyA = { branchId: 'b1', tag: 'synthetic tag' };
  const replyB = { branchId: 'b1', tag: 'other tag' };
  const readB: FuelRead = { firstHour: 124, lastHour: 147, startsSegment: true, country: '8', countryInstance: 1, continuityStartHour: 101,
    fuelRaw: '0', capacityRaw: '0', lendLeaseRaw: '0', leftOut: null, awardsReason: null,
    day: { gainedRaw: '0', burnedRaw: '0', receivedRaw: ['0', '0', '0'], otherRaw: '0',
      awards: { gainsRaw: ['0', '0', '0', '0'], ownGainRaw: '0', ownGainQuantityRaw: '0',
        transfersRaw: ['0', '0', '0', '0'], consumedRaw: '0', receivedRaw: ['0', '0', '0'] } } };
  let drill = reduceDrill(CLOSED, fuelOpen('burn', 100, false, false, null));
  let recipient: FuelRecipientSelection | null = selected;
  assert.deepEqual(fuelRecipientBasis(replyA, null, recipient, 'c1'), selected.basis);
  drill = reduceDrill(drill, { type: 'close' });
  recipient = fuelRecipientAtPick(recipient, replyB.tag); // Actual main-chart open() actor seam.
  drill = reduceDrill(drill, fuelOpen('burn', 124, false, false, null));
  assert.equal(recipient, null);
  assert.equal(fuelMoment(drill, false, 124, 100), 124);
  assert.deepEqual(fuelRecipientBasis(replyB, readB, recipient, 'c1'), { branchId: 'b1', tag: 'other tag',
    hour: 124, country: '8', countryInstance: 1 });
  // Even a retained/restored A token cannot request or render A under reply B.
  const restored = readFuelRecipient(fuelRecipientToken(selected), 'c1', 'b1');
  assert.equal(fuelRecipientForReply(restored, replyB, 'c1'), null);
  assert.equal(fuelRecipientBasis(replyB, readB, restored, 'c1'), null);
  assert.equal(fuelRecipientForReply(restored, replyA, 'c2'), null);
});
