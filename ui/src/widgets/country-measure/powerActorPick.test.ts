import test from 'node:test';
import assert from 'node:assert/strict';
import { ChartPicks } from '../../charts/pick.ts';
import type { ChartPick } from '../../charts/pick';
import { plotFrame } from '../../charts/frame.ts';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState.ts';
import { choosePowerActor, pickPowerActor, powerActorDrillAction } from './powerActorPick.ts';

const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
const runs = (value: number) => [{ firstHour: 0, lastHour: 100, value }];
const pointer = (value: number) => ({ frame, until: null, point: { x: frame.toX(49), y: frame.toY(value) } });

test('comparison picks the drawn balance curve and deduplicates that actor\'s visible maximum', () => {
  const lines = [{ key: 'raw-nearest', runs: runs(0) }, { key: 'drawn-nearest', runs: [
    { firstHour: 0, lastHour: 49, value: 0 }, { firstHour: 50, lastHour: 100, value: 100 }] }];
  assert.equal(pickPowerActor(lines, 49, pointer(40))?.tag, 'drawn-nearest');
  assert.deepEqual(pickPowerActor([...lines, { key: 'drawn-nearest', runs: runs(40) }], 49, pointer(40))?.candidates,
    ['drawn-nearest']);
});

test('overlap and keyboard picks hold the hour until one witnessed actor is chosen', () => {
  const lines = [{ key: 'a', runs: runs(80) }, { key: 'b', runs: runs(80) }];
  for (const context of [pointer(80), { frame, until: null }]) {
    const pick = pickPowerActor(lines, 49, context)!;
    assert.equal(pick.tag, null);
    assert.deepEqual(pick.candidates, ['a', 'b']);
    const drill = reduceDrill(CLOSED, powerActorDrillAction(pick, false, true, 50));
    assert.equal(drill.follow, false);
    assert.equal(drillHour(drill, 75), 49);
    assert.equal(choosePowerActor(pick, 'hidden'), pick);
    const chosen = choosePowerActor(pick, 'b');
    assert.equal(chosen.hour, 49);
    assert.equal(chosen.tag, 'b');
    assert.equal(drillHour(reduceDrill(drill, powerActorDrillAction(chosen, true, true, 75)), 75), 49);
  }
});

test('missing, clipped and future lines are excluded, and missing pick context cannot select a comparison', () => {
  const lines = [{ key: 'shown', runs: runs(50) }, { key: 'gap', runs: [{ firstHour: 0, lastHour: 10, value: 50 }] },
    { key: 'missing', runs: runs(NaN) }, { key: 'clipped', runs: runs(200) }];
  assert.deepEqual(pickPowerActor(lines, 49, { frame, until: 70 })?.candidates, ['shown']);
  assert.equal(pickPowerActor(lines, 80, { frame, until: 70 }), null);
  assert.equal(pickPowerActor(lines, 49), null);
  assert.equal(pickPowerActor(lines.slice(0, 1), 49), null, 'even one actor needs a drawn frame');
});

const scope = (owner: unknown, onPick: (hour: number, context?: ChartPick) => void) => ({
  owner, onPick, extent: [0, 100] as const, branchStart: 0, active: true,
});

test('PP/CP delayed caller retains its drawn actor callback through 500ms of 10Hz playback and label refreshes', () => {
  for (const currency of ['Political', 'Command']) {
    const picks = new ChartPicks();
    const balances = [{ key: 'a', runs: runs(10) }, { key: 'b', runs: runs(80) }];
    const owner = { source: 'recording', branch: 'branch', currency, balances, membership: 'same', maxima: 'same', ledger: 'same' };
    let selected: ReturnType<typeof pickPowerActor> = null;
    let currentHead = 49;
    const drawn = (hour: number, context?: ChartPick) => { selected = pickPowerActor(balances, hour, context); };
    picks.update(scope(owner, drawn));
    const frameAtClick = picks.drawn({ ...pointer(80), until: 49 }, drawn);
    const pending = { at: 500, run: () => picks.pick(frameAtClick, 49.9) };
    for (let now = 100; now < pending.at; now += 100) {
      currentHead++;
      picks.update(scope(owner, () => { throw new Error('A later label/playback callback replaced the drawn actor callback'); }));
      picks.drawn({ ...pointer(10), until: currentHead }, () => { throw new Error('A later frame replaced the clicked frame'); });
    }
    assert.equal(pending.run(), true);
    assert.deepEqual(selected, { hour: 49, candidates: ['b'], tag: 'b' });
  }
});

test('new PP/CP source, selection, native balances, maximum or ledger cancels delayed clicks including A to B to A', () => {
  for (const change of ['source', 'revision', 'branch', 'selection', 'membership', 'balances', 'maxima', 'ledger', 'showCeiling']) {
    const picks = new ChartPicks();
    const owner = { source: 'one', revision: 'one', branch: 'one', selection: 'a/b', membership: 'hour',
      balances: 'native balances', maxima: 'native ceiling', ledger: 'native applications', showCeiling: true };
    let calls = 0;
    const callback = (hour: number, context?: ChartPick) => {
      if (pickPowerActor([{ key: 'a', runs: runs(80) }], hour, context)) calls++;
    };
    picks.update(scope(owner, callback));
    const clicked = picks.drawn(pointer(80), callback);
    picks.update(scope({ ...owner, [change]: 'changed' }, callback));
    assert.equal(picks.pick(clicked, 49), false);
    picks.update(scope(owner, callback));
    assert.equal(picks.pick(clicked, 49), false);
    assert.equal(calls, 0);
  }
});

test('keyboard overlap chooser holds its witnessed hour and a stale frame cannot create a chooser', () => {
  const picks = new ChartPicks();
  const lines = [{ key: 'a', runs: runs(80) }, { key: 'b', runs: runs(80) }];
  let choice: ReturnType<typeof pickPowerActor> = null;
  let drill = CLOSED;
  const callback = (hour: number, context?: ChartPick) => {
    choice = pickPowerActor(lines, hour, context);
    if (choice) drill = reduceDrill(drill, powerActorDrillAction(choice, false, true, 75));
  };
  const owner = scope('owned comparison', callback);
  picks.update(owner);
  const keyboard = picks.drawn({ frame, until: 49 }, callback);
  picks.update({ ...owner, active: false });
  assert.equal(picks.pick(keyboard, 49), false);
  assert.equal(choice, null);
  picks.update(owner);
  const current = picks.drawn({ frame, until: 49 }, callback);
  assert.equal(picks.pick(current, 49), true);
  assert.equal(drill.follow, false);
  assert.equal(drillHour(drill, 99), 49);
  const chosen = choosePowerActor(choice!, 'b');
  assert.equal(chosen.tag, 'b');
  drill = reduceDrill(drill, powerActorDrillAction(chosen, true, true, 99));
  assert.equal(drillHour(drill, 99), 49);
  const rangeFrame = picks.drawn({ frame, until: 49 }, callback);
  picks.update({ ...owner, extent: [25, 100] });
  assert.equal(picks.pick(rangeFrame, 49), false);
});

test('drill actor, currency, mode and native history changes reject a pending day pick', () => {
  for (const changed of ['actor', 'currency', 'mode', 'model', 'membership', 'identities']) {
    const picks = new ChartPicks();
    const owner = { parent: 'source model', actor: 'a', currency: 'Political', mode: 'income', model: 'native ledger',
      membership: 'shown spans', identities: 'country stretches' };
    let picked: number | null = null;
    const callback = (hour: number, context?: ChartPick) => { if (context?.frame) picked = hour; };
    picks.update(scope(owner, callback));
    const clicked = picks.drawn({ frame, until: 49 }, callback);
    picks.update(scope({ ...owner, [changed]: 'changed' }, callback));
    assert.equal(picks.pick(clicked, 49), false);
    assert.equal(picked, null);
  }
});
