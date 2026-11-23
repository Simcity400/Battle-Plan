import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const PROBE_PATH = new URL('./probe.js', import.meta.url);
const PROBE_SOURCE = readFileSync(PROBE_PATH, 'utf8');

function loadProbe() {
  const rafs = new Map();
  let nextRaf = 0, rafCalls = 0, now = 0;
  const microtasks = [];
  class FakeChannel {
    constructor() {
      this.port1 = { onmessage: null };
      this.port2 = { postMessage: data => { microtasks.push(() => this.port1.onmessage?.({ data })); } };
    }
  }
  class FakeObserver {
    observe() {}
  }
  const sandbox = {
    window: null,
    performance: { now: () => now, timeOrigin: 1000 },
    requestAnimationFrame: cb => { rafCalls++; const id = ++nextRaf; rafs.set(id, cb); return id; },
    cancelAnimationFrame: id => { rafs.delete(id); },
    setTimeout: (fn) => { microtasks.push(fn); return 1; },
    MessageChannel: FakeChannel,
    PerformanceObserver: FakeObserver,
    addEventListener: () => {},
    document: {
      createElement: () => ({ width: 0, height: 0, getContext: () => ({ clearRect() {}, drawImage() {}, getImageData: () => ({ data: [] }) }) }),
      querySelector: () => null,
      querySelectorAll: () => [],
    },
    CSS: { escape: value => String(value) },
    innerWidth: 1280,
    innerHeight: 720,
    getComputedStyle: () => ({ overflowX: 'visible', overflowY: 'visible', contain: '' }),
  };
  sandbox.window = sandbox;
  vm.runInNewContext(PROBE_SOURCE, sandbox);
  const stepFrame = dt => {
    now += dt;
    const batch = [...rafs.entries()];
    rafs.clear();
    for (const [, cb] of batch) cb(now);
    while (microtasks.length) microtasks.shift()();
  };
  return {
    probe: sandbox.__e2e,
    pendingRafs: () => rafs.size,
    rafCalls: () => rafCalls,
    stepFrame,
  };
}

test('probe schedules no animation frames while idle, and runs only while tracking, watching, or recording frame gaps', async () => {
  const host = loadProbe();
  assert.equal(host.pendingRafs(), 0, 'idle probe schedules no animation frames on load');
  assert.equal(host.rafCalls(), 0);

  host.probe.watch();
  assert.equal(host.pendingRafs(), 1, 'watch starts the frame loop');
  host.stepFrame(16);
  assert.equal(host.pendingRafs(), 1, 'watch keeps running while active');
  host.probe.unwatch();
  assert.equal(host.pendingRafs(), 0, 'unwatch stops the frame loop');

  host.probe.mark(null);
  assert.equal(host.pendingRafs(), 1, 'mark starts the frame loop');
  const settled = host.probe.settle(1000, 0, null, ({ settled: done }) => done);
  host.stepFrame(16);
  host.stepFrame(16);
  host.stepFrame(16);
  await settled;
  assert.equal(host.pendingRafs(), 0, 'settle leaves no animation frames scheduled once done');

  const beforeDrag = host.rafCalls();
  host.probe.startFrames();
  assert.equal(host.pendingRafs(), 1, 'startFrames starts the frame loop for drag gap measurement');
  host.stepFrame(16);
  host.stepFrame(20);
  host.stepFrame(18);
  const gaps = host.probe.gaps(0, 1000);
  assert.ok(gaps.length >= 2, 'recorded frame intervals while active');
  host.probe.stopFrames();
  assert.equal(host.pendingRafs(), 0, 'stopFrames stops the frame loop');
  const afterDrag = host.rafCalls();
  assert.ok(afterDrag > beforeDrag);
  host.stepFrame(16);
  assert.equal(host.rafCalls(), afterDrag, 'no extra frames run after stopFrames');
});
