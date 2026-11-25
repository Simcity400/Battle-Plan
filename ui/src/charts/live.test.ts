import assert from 'node:assert/strict';
import test from 'node:test';
import { LiveCell, liveAt, liveKey, liveTexts } from './live.ts';
import { valuesOf } from '../test-support/liveValues.ts';

test('a live value is made again only when the clock moves to another hour, kept while it is the same, and wakes at frames and seeks', () => {
  let position = 10.2, made = 0;
  const frames = new Set<() => void>(), seeks = new Set<() => void>();
  const clock = { getSnapshot: () => ({ position }),
    subscribeFrame: (listener: () => void) => { frames.add(listener); return () => frames.delete(listener); },
    subscribe: (listener: () => void) => { seeks.add(listener); return () => seeks.delete(listener); } };
  const live = liveAt(clock, at => Math.floor(at), hour => { made++; return [hour >= 12 ? 'later' : 'early']; },
    (a, b) => a.length === b.length && a.every((v, i) => v === b[i]));
  const first = live.read();
  position = 10.9;
  assert.equal(live.read(), first);
  assert.equal(made, 1, 'the same hour reads what it made');
  position = 11;
  assert.equal(live.read(), first, 'another hour with the same entries keeps the array');
  assert.equal(made, 2);
  position = 12;
  assert.deepEqual(live.read(), ['later']);
  assert.equal(live.hour(), 12);
  const stop = live.subscribe(() => {});
  assert.equal(frames.size + seeks.size, 2, 'a frame of playback and a seek both wake the chart');
  stop();
  assert.equal(frames.size + seeks.size, 0);
});

test('a key the values do not hold is missing with its reason, and a set cell tells its readers only when it changes', () => {
  const values = valuesOf(new Map([[liveKey('GER', 'opinion'), 12]]), key => `no ${key.split('\u0000')[0]}`);
  assert.equal(values.get(liveKey('GER', 'opinion')), 12);
  assert.equal(values.get(liveKey('SOV', 'opinion')), undefined);
  assert.equal(values.missing(liveKey('SOV', 'opinion')), 'no SOV');
  const cell = new LiveCell(1);
  let told = 0;
  const stop = cell.subscribe(() => { told++; });
  cell.set(1);
  cell.set(2);
  assert.equal(told, 1);
  assert.equal(cell.read(), 2);
  stop();
  cell.set(3);
  assert.equal(told, 1);
});

test('live texts are read again when what they read lands though the hour stands still, and wake when it may have', () => {
  const frames = new Set<() => void>(), seeks = new Set<() => void>(), landed = new Set<() => void>();
  const clock = { subscribeFrame: (listener: () => void) => { frames.add(listener); return () => frames.delete(listener); },
    subscribe: (listener: () => void) => { seeks.add(listener); return () => seeks.delete(listener); } };
  let answer: number | undefined, version = 0, reads = 0;
  const texts = liveTexts(clock, () => 100, hour => { reads++; return new Map([['share', answer === undefined ? '' : `${hour}: ${answer}%`]]); },
    { version: () => [version], subscribe: changed => { landed.add(changed); return () => landed.delete(changed); } });
  assert.equal(texts.text('share'), '', 'nothing before the answer');
  assert.equal(texts.text('share'), '');
  assert.equal(reads, 1, 'the same hour and version read once');
  let woken = 0;
  const stop = texts.subscribe(() => { woken++; });
  answer = 41.2; version++;
  for (const changed of landed) changed();
  assert.equal(woken, 1, 'an answer landing wakes the painter while paused');
  assert.equal(texts.text('share'), '100: 41.2%', 'never the empty text read before the answer at the same hour');
  stop();
  assert.equal(frames.size + seeks.size + landed.size, 0);
  const plain = liveTexts(clock, () => 7, hour => new Map([['hour', String(hour)]]));
  assert.equal(plain.text('hour'), '7');
  assert.equal(plain.text('other'), '');
});

