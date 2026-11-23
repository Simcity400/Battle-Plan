import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isPlaybackActive, preloadAllWidgetRenderers, setPlaybackActive } from './rendererLoaders.ts';

test('idle preloading never runs while playback is active, respects idle budget, and prefers modulepreload', () => {
  const clock = {};
  const queued: ((deadline: { didTimeout: boolean; timeRemaining(): number }) => void)[] = [];
  const preloaded: string[] = [];
  const loaded: string[] = [];
  const stop = preloadAllWidgetRenderers({
    kinds: ['a', 'b', 'c'],
    chunks: { a: './assets/widget-a.js', b: './assets/widget-b.js', c: './assets/widget-c.js' },
    schedule: run => queued.push(run),
    cancel: () => {},
    preload: url => preloaded.push(url),
    load: async kind => { loaded.push(kind); },
  });
  try {
    assert.equal(queued.length, 1);
    setPlaybackActive(clock, true);
    assert.equal(isPlaybackActive(), true);
    queued.shift()!({ didTimeout: false, timeRemaining: () => 40 });
    assert.deepEqual([preloaded, loaded], [[], []], 'nothing preloads while playing');
    assert.equal(queued.length, 0, 'no idle timer spins while playing');
    setPlaybackActive(clock, false);
    assert.equal(isPlaybackActive(), false);
    assert.equal(queued.length, 1, 'preloading resumes once playback pauses');
    queued.shift()!({ didTimeout: true, timeRemaining: () => 0 });
    assert.deepEqual(preloaded, [], 'a timed-out deadline is skipped');
    queued.shift()!({ didTimeout: false, timeRemaining: () => 4 });
    assert.deepEqual(preloaded, [], 'an insufficient idle slice is skipped');
    queued.shift()!({ didTimeout: false, timeRemaining: () => 30 });
    assert.deepEqual(preloaded, ['./assets/widget-a.js'], 'one chunk preloaded per idle slice via modulepreload');
    assert.deepEqual(loaded, [], 'modulepreload avoids main-thread module evaluation');
    queued.shift()!({ didTimeout: false, timeRemaining: () => 30 });
    queued.shift()!({ didTimeout: false, timeRemaining: () => 30 });
    assert.deepEqual(preloaded, ['./assets/widget-a.js', './assets/widget-b.js', './assets/widget-c.js']);
    assert.equal(queued.length, 0);
  } finally {
    setPlaybackActive(clock, false);
    stop();
  }
});
