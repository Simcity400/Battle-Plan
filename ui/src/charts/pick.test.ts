import assert from 'node:assert/strict';
import test from 'node:test';
import { plotFrame } from './frame.ts';
import { runAt } from './series.ts';
import { ChartPicks } from './pick.ts';
import type { ChartPick } from './pick';

function pickLines(lines: readonly { key: string; runs: readonly { firstHour: number; lastHour: number; value: number }[] }[], hour: number, at: ChartPick): string[] {
  const y = at.point?.y;
  if (y === undefined || !at.frame) return lines.filter(line => runAt(line.runs, hour)).map(line => line.key);
  const scored = lines.flatMap(line => {
    const run = runAt(line.runs, hour);
    return run ? [{ key: line.key, d: Math.abs(at.frame.toY(run.value) - y) }] : [];
  });
  const best = Math.min(...scored.map(item => item.d));
  return [...new Set(scored.filter(item => item.d <= best + 2).map(item => item.key))].sort();
}

const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
const context = { frame, until: 50, point: { x: frame.toX(50), y: frame.toY(80) } };
const scope = (owner: unknown, onPick: (hour: number) => void) => ({ owner, onPick, extent: [0, 100] as const, branchStart: 0, active: true });

test('a 500ms pending click uses the drawn callback through 10Hz playback and label updates', () => {
  const picks = new ChartPicks();
  const owner = { source: 'one', branch: 'same', model: 'A/B' };
  const delivered: unknown[] = [];
  const lines = [{ key: 'A', runs: [{ firstHour: 0, lastHour: 100, value: 10 }] },
    { key: 'B', runs: [{ firstHour: 0, lastHour: 100, value: 80 }] }];
  const drawnCallback = (hour: number, at: ChartPick = context) => delivered.push([hour, at.until, pickLines(lines, hour, at)]);
  picks.update(scope(owner, drawnCallback));
  const drawn = picks.drawn(context, drawnCallback);
  const pending = { at: 500, run: () => picks.pick(drawn, 50.9) };
  for (let now = 100; now < pending.at; now += 100) {
    picks.update(scope(owner, () => delivered.push('latest callback must not replace drawn callback')));
    picks.drawn({ ...context, until: 50 + now / 100 }, () => delivered.push('later frame'));
  }
  assert.equal(pending.run(), true);
  assert.deepEqual(delivered, [[50, 50, ['B']]]);
});

test('new ceiling data, membership or source refuses a pending click and A-B-A stays cancelled', () => {
  for (const changed of ['ceiling arrived', 'hour membership to ever', 'different recording', 'different selection']) {
    const picks = new ChartPicks();
    let calls = 0;
    const callback = () => { calls++; };
    picks.update(scope('clicked owner', callback));
    const clicked = picks.drawn(context, callback);
    const pending = { at: 500, run: () => picks.pick(clicked, 50) };
    picks.update(scope(changed, callback));
    assert.equal(pending.run(), false);
    picks.update(scope('clicked owner', callback));
    assert.equal(pending.run(), false, 'restoring a model does not restore a cancelled gesture');
    assert.equal(calls, 0);
  }
});

test('a ceiling arriving during the click delay cannot add an actor to the witnessed selection', () => {
  const picks = new ChartPicks();
  const balances = [{ key: 'A', runs: [{ firstHour: 0, lastHour: 100, value: 10 }] },
    { key: 'B', runs: [{ firstHour: 0, lastHour: 100, value: 80 }] }];
  const results: string[][] = [];
  const clicked = (hour: number, at: ChartPick = context) => results.push(pickLines(balances, hour, at));
  picks.update(scope({ balances, ceilings: null }, clicked));
  const drawn = picks.drawn(context, clicked);
  const newCeilings = [{ key: 'A', runs: [{ firstHour: 0, lastHour: 100, value: 80 }] }];
  const current = (hour: number, at: ChartPick = context) => results.push(pickLines([...balances, ...newCeilings], hour, at));
  picks.update(scope({ balances, ceilings: newCeilings }, current));
  assert.equal(picks.pick(drawn, 50), false);
  assert.deepEqual(results, []);
  const newFrame = picks.drawn(context, current);
  assert.equal(picks.pick(newFrame, 50), true);
  assert.deepEqual(results, [['A', 'B']]);
});

test('keyboard picks require the last drawn owner, and inactive or changed ranges refuse it', () => {
  const picks = new ChartPicks();
  let calls = 0;
  const callback = () => { calls++; };
  const owner = scope('one', callback);
  picks.update(owner);
  const drawn = picks.drawn({ frame, until: 50 }, callback);
  picks.update({ ...owner, active: false });
  assert.equal(picks.pick(drawn, 50), false);
  picks.update(owner);
  assert.equal(picks.pick(drawn, 50), false);
  const resumed = picks.drawn({ frame, until: 50 }, callback);
  picks.update({ ...owner, extent: [25, 100] });
  assert.equal(picks.pick(resumed, 50), false);
  const redrawn = picks.drawn({ frame, until: 50 }, callback);
  assert.equal(picks.pick(redrawn, 50), true);
  assert.equal(calls, 1);
});
