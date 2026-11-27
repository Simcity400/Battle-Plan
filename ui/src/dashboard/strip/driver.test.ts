import assert from 'node:assert/strict';
import test from 'node:test';
import { encode } from '@msgpack/msgpack';
import { Lane } from '../../draw/lane.ts';
import { identityBrokerFor, type StripIdentitySource } from '../../families/identity/broker.ts';
import { identityKeyString, stateKey, type IdentitySnapshot } from '../../families/identity/types.ts';
import { busy, busyKinds } from '../../shell/busy.ts';
import type { PlaceNameRunsParams, StripParams } from '../../generated/dashboard.ts';
import { FrameEngine } from '../../engine/frame.ts';
import { measureWith, type PerfRecorder } from '../../shell/perf.ts';
import { gatedHour } from '../../test-support/gated.ts';
import type { DashboardClient, ProductRequest } from '../client.ts';
import { OpenedSources } from '../openedSources.ts';
import { PlaybackController } from '../playback.ts';
import { PrepareGate } from '../prepare.ts';
import { productRequest } from '../request.ts';
import { tileCache } from '../tiles/view.ts';
import { configuredStripPage, StripPage, type StripWidget } from './driver.ts';
import { stripLattice } from './lattice.ts';
import { WidgetStrips, type StripUpdate } from './store.ts';
import type { StripFrame } from './scrub.ts';

function packet(whole: boolean, states: unknown[]) {
  const parts = whole && states.length ? [encode(states[0])] : [];
  for (let i = 1; i < states.length; i++) {
    parts.push(encode([{ replace: [[], states[i]] }]), encode([{ replace: [[], states[i - 1]] }]));
  }
  const bytes = Buffer.alloc(8 + parts.reduce((sum, part) => sum + 4 + part.length, 0));
  bytes.set([66, 80, 83, 84, 1, Number(whole), states.length, 0]);
  let at = 8;
  for (const part of parts) {
    bytes.writeUInt32LE(part.length, at);
    bytes.set(part, at + 4);
    at += 4 + part.length;
  }
  return bytes.toString('base64');
}

function setup(last: number, budget?: number, gap?: readonly [number, number]) {
  const frames = new FrameEngine({ request() {} });
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: last });
  const branch = { id: '1', firstHour: 0, lastHour: last, gaps: gap ? [{ firstHour: gap[0], lastHour: gap[1], reason: 'Not recorded' }] : [] };
  const lattice = stripLattice(branch, 0);
  const opened = new OpenedSources();
  let frame: (() => void) | undefined, settled = true, reads = 0;
  const gate = new PrepareGate(() => true, { request: run => { frame = run; return 1; }, cancel: () => { frame = undefined; } });
  const groups: { source: string; hours: number[]; whole: boolean; signal: AbortSignal; busy: boolean }[] = [];
  const tiles: { signal: AbortSignal; busy: boolean }[] = [];
  const identities: { from: number; signal: AbortSignal; busy: boolean }[] = [];
  let identityDelay: ((signal: AbortSignal) => Promise<void>) | undefined;
  let nameOf = (_id: number, from: number) => `Dated state ${from}`;
  let answer = (hour: number): unknown => ({ answer: { hour, value: hour * 3 } });
  let replyCount = (hours: number[], _whole: boolean) => hours.length;
  let delay: ((hours: number[], signal: AbortSignal) => Promise<void> | void) | undefined;
  let delayTile: ((signal: AbortSignal) => Promise<void>) | undefined;
  const client = { product: async (request: ProductRequest, signal: AbortSignal) => {
    if (request.product === 'series-tiles') {
      tiles.push({ signal, busy: clock.getSnapshot().playing || clock.getScrubPosition() !== null || !settled || busyKinds().requests > 0 });
      await delayTile?.(signal);
      return { ...request, data: 'tile reply' };
    }
    if (request.product === 'place-name-runs') {
      const { from, to, states } = request.params as unknown as PlaceNameRunsParams;
      identities.push({ from, signal, busy: clock.getSnapshot().playing || !settled || busyKinds().requests > 0 });
      await identityDelay?.(signal);
      return { ...request, data: { from, to, unknown: [], states: states.map(id => ({
        id, runs: [{ firstHour: from, lastHour: to, name: nameOf(id, from), missing: null, presence: 'present' }],
      })), provinces: [], warnings: [] } };
    }
    assert.equal(request.product, 'strip');
    assert.equal(opened.refusal(request), null);
    const { product, asks, whole } = request.params as StripParams;
    assert.equal(product, 'country-fuel');
    const hours = asks.map(ask => Number(ask.hour));
    groups.push({ source: request.sourceId, hours, whole, signal, busy: clock.getSnapshot().playing || !settled || busyKinds().requests > 0 });
    await delay?.(hours, signal);
    return { ...request, data: { data: packet(whole, hours.slice(0, replyCount(hours, whole)).map(answer)) } };
  } } as unknown as DashboardClient;
  const page = new StripPage(lattice, clock, client, { budget, gate, frames });
  const stop = page.start();
  const until = async (done: () => boolean) => {
    for (let tick = 0; tick < 5000; tick++) {
      if (done()) return;
      const run = frame; frame = undefined; run?.();
      await Promise.resolve();
    }
    assert.fail(`Strip page did not settle (${page.status})`);
  };
  const mount = (id: string, after: number = 0, supported = true, receipt?: (change: StripUpdate, build: number) => Promise<void> | void, fixed = false,
    scrub?: StripWidget['scrub'], track?: StripWidget['track'], identity?: StripIdentitySource,
    stateAt = (_hour: number) => 7, lookupAt = (hour: number) => hour) => {
    opened.add({ id, revision: 'r', metadata: { branches: [branch] } } as never);
    const held = new WidgetStrips();
    let scan: IdentitySnapshot | undefined;
    const widget: StripWidget = { key: id, supported, fixed, scrub, track, identity, settled: () => settled,
      keyframe: hour => {
        if (fixed) return { asks: [] };
        reads++;
        return { asks: [{ slot: 'source', request: hour < after ? null : productRequest({
          product: 'country-fuel', sourceId: id, revision: 'r', params: { branchId: '1', hour: gatedHour(hour) },
        }) }] };
      },
      store: async (build, change) => {
        const result = held.update(change);
        await receipt?.(change, build);
        if (change.kind === 'identities' && change.snapshot !== undefined) scan = change.snapshot as IdentitySnapshot;
        if (change.kind === 'discover') {
          const hour = lookupAt(change.hour), key = stateKey(stateAt(change.hour));
          return { ...result, identities: [{ key, hour }], unresolved:
            !scan?.entities?.[identityKeyString(key)]?.some(run => run.firstHour <= hour && hour <= run.lastHour) };
        }
        return result;
      },
    };
    return { held, remove: page.register(id, widget) };
  };
  return { page, clock, groups, tiles, identities, client, lattice, gate, frames, mount, until, cache: tileCache(client), reads: () => reads,
    stop() { clock.setPlaying(false); stop(); },
    settle: (value: boolean) => { settled = value; },
    delay: (value: typeof delay) => { delay = value; },
    delayTile: (value: typeof delayTile) => { delayTile = value; },
    identityDelay: (value: typeof identityDelay) => { identityDelay = value; },
    nameOf: (value: typeof nameOf) => { nameOf = value; },
    answer: (value: typeof answer) => { answer = value; },
    replyCount: (value: typeof replyCount) => { replyCount = value; },
  };
}

test('identity windows share strip cancellation and the lane acknowledgment gates readiness', async t => {
  const h = setup(2000);
  t.after(h.stop);
  const scope = { sourceId: 'state', branchId: '1', revision: 'r', content: 0 };
  let finish: (() => void) | undefined, install: (() => void) | undefined;
  h.identityDelay(signal => new Promise((resolve, reject) => {
    finish = () => signal.aborted ? reject(signal.reason) : resolve();
  }));
  h.mount('state', 0, true, change => change.kind === 'identities' && change.snapshot === undefined && !change.fine
    ? new Promise<void>(resolve => { install = resolve; }) : undefined, false, undefined, undefined, { scope, bounds: [0, 2000] });
  await h.until(() => h.identities.length === 1);
  h.clock.setPlaying(true);
  assert.equal(h.identities[0]!.signal.aborted, true);
  finish!();
  await h.until(() => h.gate.pending === 1);
  assert.equal(h.identities.length, 1);
  assert.equal(h.page.status, 'not ready');
  h.identityDelay(undefined);
  h.clock.setPlaying(false);
  await h.until(() => !!install);
  assert.equal(h.page.status, 'not ready');
  install!();
  await h.until(() => h.page.status === 'ready');
  assert.deepEqual(h.identities.map(ask => ask.from), [0, 0, 720, 1440]);
  assert.ok(h.identities.every(ask => !ask.busy));
});

test('fine identity bytes lower the whole page density and release fine-only keys from shared windows', async t => {
  const h = setup(191, 16_000);
  t.after(h.stop);
  const scope = { sourceId: 'state', branchId: '1', revision: 'r', content: 0 };
  h.nameOf(id => id === 8 ? 'x'.repeat(20_000) : 'Coarse name');
  const named = h.mount('state', 0, true, undefined, false, undefined, undefined, { scope, bounds: [0, 191] },
    hour => hour === 5 ? 8 : 7);
  const other = h.mount('other');
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.page.density, 8);
  const keys = h.lattice.hours(8).length;
  assert.equal(named.held.slots.get('source')!.counts.keys, keys);
  assert.equal(other.held.slots.get('source')!.counts.keys, keys);
  assert.equal(identityBrokerFor(h.client).resolve(scope, stateKey(8), 5).kind, 'pending');
});

test('changing visible rows reuses held groups and requests only the additional dated window', async t => {
  const h = setup(1919);
  t.after(h.stop);
  const scope = { sourceId: 'state', branchId: '1', revision: 'r', content: 0 };
  let lookup = 5;
  h.mount('state', 0, true, undefined, false, undefined, undefined, { scope, bounds: [0, 1919] }, undefined, hour => Math.min(hour, lookup));
  await h.until(() => h.page.status === 'ready');
  const groupCount = h.groups.length;
  const first = { first: 0, count: 20 };
  h.page.rowWindowChanged(first, { ...first });
  assert.equal(h.page.status, 'ready');
  lookup = 725;
  h.page.rowWindowChanged(first, { first: 10, count: 20 });
  assert.equal(h.page.status, 'not ready');
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.groups.length, groupCount);
  assert.deepEqual(h.identities.map(ask => ask.from), [0, 720]);
});

test('coarse identity admission drops fine strips left by a previously ready widget', async t => {
  const budget = 6000, h = setup(191, budget);
  t.after(h.stop);
  const lane = new Lane({
    lane: 0, lanes: 1, views: () => assert.fail('Unexpected view'), crop: async () => [],
    present() {}, release() {}, fail: assert.fail, later() {},
  });
  const initialScope = { sourceId: 'initial', branchId: '1', revision: 'r', content: 0 };
  let droppedFine = false, committedCoarse = false;
  const previous = h.mount('initial', 0, true, (change, build) => {
    if (change.kind === 'identities' || change.kind === 'drop') lane.strip(1, build, change);
    if (change.kind === 'drop' && change.fine) droppedFine = true;
    if (change.kind === 'identities' && !change.fine && !change.snapshot && droppedFine) committedCoarse = true;
  }, false, undefined, undefined, { scope: initialScope, bounds: [0, 191] });
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.page.density, 16);
  const more = budget - previous.held.slots.get('source')!.bytes + 128;
  h.nameOf(id => id === 8 ? 'x'.repeat(more) : 'Earlier campaign');
  const scope = { sourceId: 'state', branchId: '1', revision: 'r', content: 0 };
  h.mount('state', 0, true, undefined, false, undefined, undefined, { scope, bounds: [0, 191] }, () => 8);
  await h.until(() => h.page.status === 'coarse' || h.page.status === 'ready' || h.page.status === 'will not follow');
  assert.notEqual(h.page.status, 'will not follow');
  assert.equal(committedCoarse, true, 'the earlier campaign commits its coarse snapshot after the fine drop');
  assert.equal(previous.held.slots.get('source')!.counts.keys, h.lattice.hours(1).length);
});

test('changing visible rows releases retired identities when they would put coarse readiness over budget', async t => {
  const h = setup(191, 16_000);
  t.after(h.stop);
  const scope = { sourceId: 'state', branchId: '1', revision: 'r', content: 0 };
  h.nameOf(id => 'x'.repeat(id === 8 ? 8_000 : id === 9 ? 9_000 : 20));
  let scrolled = false;
  h.mount('state', 0, true, undefined, false, undefined, undefined, { scope, bounds: [0, 191] },
    hour => scrolled ? 9 : hour === 5 ? 8 : 7);
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.page.density, 16);
  const broker = identityBrokerFor(h.client);
  assert.equal(broker.resolve(scope, stateKey(8), 5).kind, 'known');

  scrolled = true;
  h.page.rowWindowChanged({ first: 0, count: 20 }, { first: 10, count: 20 });
  await h.until(() => h.page.status === 'coarse' || h.page.status === 'will not follow');
  assert.equal(h.page.status, 'coarse');
  h.clock.scrub(5);
  assert.equal(broker.resolve(scope, stateKey(9), 5).kind, 'known');
  assert.equal(broker.resolve(scope, stateKey(8), 5).kind, 'pending');
});

test('the shipped switch leaves ordinary broker requests and resolved identities identical', async () => {
  const trace = async (configured: boolean) => {
    const h = setup(2000);
    const scope = { sourceId: 'state', branchId: '1', revision: 'r', content: 0 };
    const key = stateKey(7);
    const broker = identityBrokerFor(h.client), owner = {};
    if (configured) assert.equal(configuredStripPage(() => h.mount('state', 0, true, undefined, false, undefined, undefined, { scope })), null);
    broker.setDemand(owner, scope, { asks: [{ key, hour: 1600 }] }, undefined, [0, 2000]);
    await h.until(() => broker.resolve(scope, key, 1600).kind === 'known');
    const result = { asks: h.identities.map(({ from, busy }) => [from, busy]), answer: broker.snapshotOf(scope), groups: h.groups.length };
    h.stop();
    return result;
  };
  const before = await trace(false), shipped = await trace(true);
  assert.deepEqual({ ...shipped, answer: { ...shipped.answer, epoch: 0 } }, { ...before, answer: { ...before.answer, epoch: 0 } });
  assert.deepEqual(shipped.asks, [[1440, false]]);
  assert.equal(shipped.answer.entities?.['state:7']?.[0]?.name, 'Dated state 1440');
  assert.equal(shipped.groups, 0);
});

test('the driver yields to playback, seeks and foreground reads and resumes after its committed anchor', async t => {
  const h = setup(3200);
  t.after(h.stop);
  h.clock.setPlaying(true);
  h.mount('state');
  await h.until(() => h.gate.pending === 1);
  assert.equal(h.groups.length, 0);
  assert.equal(h.reads(), 0, 'the full-density request scan waits for idle too');
  h.settle(false);
  h.clock.setPlaying(false);
  await h.until(() => h.gate.pending === 1);
  assert.equal(h.groups.length, 0);
  const lateAsks: number[] = [];
  let finishAsk!: () => void;
  h.delay((hours, signal) => h.groups.length >= 2 && h.groups.length <= 4 ? new Promise((resolve, reject) => {
    finishAsk = () => {
      if (signal.aborted) { lateAsks.push(hours[0]!); reject(signal.reason); }
      else resolve();
    };
  }) : undefined);
  h.settle(true);
  await h.until(() => h.groups.length === 2);
  const [held, firstInterrupt] = h.groups;
  const endRead = busy('requests');
  t.after(endRead);
  assert.ok(firstInterrupt!.signal.aborted, 'a foreground read aborts synchronously');
  finishAsk();
  await h.until(() => h.gate.pending === 1 && h.groups.length === 2);
  endRead();
  await h.until(() => h.groups.length === 3);
  const secondInterrupt = h.groups[2]!;
  h.settle(false);
  h.clock.seek(900);
  assert.ok(secondInterrupt.signal.aborted);
  finishAsk();
  await h.until(() => h.gate.pending === 1 && h.groups.length === 3);
  h.settle(true);
  await h.until(() => h.groups.length === 4);
  const thirdInterrupt = h.groups[3]!;
  h.clock.setPlaying(true);
  assert.ok(thirdInterrupt.signal.aborted);
  finishAsk();
  await h.until(() => h.gate.pending === 1 && h.groups.length === 4);
  h.clock.setPlaying(false);
  await h.until(() => h.page.status === 'ready');
  for (const group of h.groups.slice(2, 5)) assert.deepEqual(group.hours, firstInterrupt!.hours);
  assert.equal(firstInterrupt!.hours[0], held!.hours.at(-1));
  assert.equal(h.groups.filter(group => group.whole).length, 1);
  assert.equal(h.groups.filter(group => group.busy).length, 0);
  assert.ok(h.groups.every(group => group.hours.length <= 17 && group.hours.every((hour, i) => i === 0 || hour > group.hours[i - 1]!)));
  assert.deepEqual(lateAsks, [firstInterrupt!.hours[0], secondInterrupt.hours[0], thirdInterrupt.hours[0]], 'only the current ask of each cancelled group finishes');
  const stats = h.page['counts'];
  assert.deepEqual([stats.startedWhilePlaying, stats.startedWhileSeeking, stats.startedWhileForeground], [0, 0, 0]);
  assert.equal(stats.groupsResumed, 3);
  assert.equal(stats.asksDiscarded, 3 * firstInterrupt!.hours.length);
  assert.equal(stats.anchorsAsked - stats.anchorsCommitted, 3);
  t.diagnostic(`${h.groups.length} groups, 3 cancelled and resumed, ${lateAsks.length} late asks, ${stats.asksDiscarded} discarded group asks, 0 groups during playback, seek or foreground reads`);
});

test('a strip reply that gives way commits its prefix and resumes at the held anchor', async t => {
  const h = setup(3200);
  t.after(h.stop);
  h.replyCount((hours, whole) => {
    switch (h.groups.length) {
      case 1: assert.ok(whole); h.settle(false); return 0;
      case 2: assert.ok(whole); return 4;
      case 3: assert.ok(!whole); return 1;
      default: return hours.length;
    }
  });
  const { held } = h.mount('state');
  await h.until(() => h.page['counts'].groupsGaveWay === 1 && h.gate.pending === 1);
  assert.equal(h.groups.length, 1);
  assert.equal(h.page['counts'].yieldedToForeign, 0);
  h.settle(true);
  await h.until(() => h.page['counts'].yieldedToForeign === 1);
  h.frames.tick(0);
  await h.until(() => h.page.status === 'ready');
  const [empty, prefix, heldOnly, resumed] = h.groups;
  assert.deepEqual(empty!.hours, prefix!.hours);
  assert.equal(heldOnly!.hours[0], prefix!.hours[3]);
  assert.deepEqual(heldOnly!.hours, resumed!.hours);
  assert.deepEqual(held.slots.get('source')?.at(prefix!.hours[2]!),
    { answer: { hour: prefix!.hours[2]!, value: prefix!.hours[2]! * 3 } });
  assert.equal(h.page['counts'].groupsGaveWay, 3);
  assert.equal(h.page['counts'].groupsCancelled, 0);
});

test('consecutive zero-progress replies back off for at most 32 frames and progress resets the wait', async t => {
  const h = setup(3200);
  t.after(h.stop);
  h.replyCount(hours => h.groups.length <= 7 ? 0 : h.groups.length === 8 ? 4 : h.groups.length === 9 ? 1 : hours.length);
  h.mount('state');
  let frame = 0;
  for (const [index, wait] of [1, 2, 4, 8, 16, 32, 32, 1].entries()) {
    const count = index + 1;
    await h.until(() => h.page['counts'].yieldedToForeign === count);
    const asks = h.groups.length;
    for (let i = 1; i < wait; i++) h.frames.tick(++frame);
    await Promise.resolve();
    assert.equal(h.groups.length, asks);
    h.frames.tick(++frame);
    await h.until(() => h.groups.length > asks);
  }
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.page['counts'].foreignYieldFrames, frame);
  assert.equal(h.page['counts'].groupsCancelled, 0);
});

test('the shipped switch allocates no page and leaves zero requests and zero held bytes', () => {
  const held = new WidgetStrips();
  let groups = 0;
  const page = configuredStripPage(() => {
    groups++;
    const h = setup(200);
    h.page.register('chart', { key: 'chart', supported: true, keyframe: () => ({ asks: [] }), settled: () => true,
      tiles: async () => { groups++; }, store: async (_build, change) => held.update(change) });
    return h.page;
  });
  assert.equal(page, null);
  assert.equal(groups, 0);
  assert.equal(held.slots.size, 0);
  assert.equal(held.update({ kind: 'drop', fine: false }).bytes, 0);
});

test('a mixed page waits for every claimed tile, including after a scrub cancels its request', async t => {
  const h = setup(200);
  t.after(h.stop);
  const frames: (StripFrame | null)[] = [];
  const holds: string[] = [];
  measureWith(new Proxy({ strip: (sample: { reason?: string }) => { if (sample.reason) holds.push(sample.reason); } },
    { get: (target, key) => Reflect.get(target, key) ?? (() => {}) }) as unknown as PerfRecorder);
  t.after(() => measureWith(null));
  const answer = h.mount('answer', 0, true, undefined, false, (_page, frame) => frames.push(frame));
  const chart = h.page.register('chart', {
    key: 'chart', supported: true, keyframe: () => ({ asks: [] }), settled: () => true,
    scrub: () => assert.fail('A widget with tiles cannot scrub yet'),
    store: async () => assert.fail('Tile-only chart does not store strip slots'),
    async tiles(claim, read) {
      const request = productRequest({ sourceId: 'chart', revision: 'r', product: 'series-tiles',
        params: { branchId: '1', family: { kind: 'autonomy-lines' }, binHours: 6, index: 0 } });
      const reply = await read<string>(request);
      claim.put({ key: 'chart|tile', value: reply.data, bytes: reply.data.length });
    },
  });
  let settleTile!: (value?: undefined) => void;
  h.delayTile(async signal => {
    await new Promise<void>(resolve => { settleTile = resolve; });
    signal.throwIfAborted();
  });
  await h.until(() => h.tiles.length === 1);
  assert.equal(h.page.status, 'not ready');
  h.clock.scrub(100);
  assert.ok(h.tiles[0]!.signal.aborted);
  settleTile();
  await h.until(() => h.gate.pending === 1);
  assert.equal(h.tiles.length, 1);
  assert.equal(h.cache.stripBytes, 0);
  h.clock.endScrub();
  await h.until(() => h.tiles.length === 2);
  settleTile();
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.cache.stripBytes, 'tile reply'.length);
  assert.ok(answer.held.slots.size > 0);
  assert.deepEqual(h.tiles.map(tile => tile.busy), [false, false]);
  assert.equal(h.page['counts'].tilesCancelled, 1);
  h.clock.scrub(100);
  h.frames.tick(16);
  h.clock.scrub(110);
  h.frames.tick(32);
  assert.equal(frames.length, 0);
  assert.deepEqual(holds, ['tile widgets follow from 3C-ii', 'tile widgets follow from 3C-ii']);
  h.clock.endScrub();
  chart();
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.cache.stripBytes, 0);
  h.clock.scrub(110);
  h.frames.tick(48);
  assert.equal(frames[0]?.hour, h.lattice.scrub(110, h.page.density)?.hour);
  h.clock.endScrub();
});

test('a ready page sends one selection per frame and a change in readiness holds the rest of the drag', async t => {
  const h = setup(410, undefined, [110, 130]);
  t.after(h.stop);
  const frames: (StripFrame | null)[] = [];
  const scrub: StripWidget['scrub'] = (_page, frame) => frames.push(frame);
  h.mount('first', 0, true, undefined, false, scrub);
  h.mount('second', 0, true, undefined, false, scrub);
  h.clock.scrub(10);
  h.frames.tick(16);
  assert.equal(frames.length, 0);
  h.clock.endScrub();
  await h.until(() => h.page.status === 'coarse');
  const groups = h.groups.length;
  h.clock.scrub(98);
  h.clock.scrub(104.5);
  h.frames.tick(32);
  assert.deepEqual(frames.map(frame => frame && [frame.frame, frame.hour, frame.density]), [[2, 95, 1]]);
  h.clock.scrub(120);
  h.frames.tick(48);
  assert.equal(frames.at(-1)?.hour, 120);
  assert.equal(frames.at(-1)?.gap?.kind, 'not-recorded');
  h.clock.scrub(131);
  h.frames.tick(64);
  assert.equal(frames.at(-1)?.hour, 131, 'the first recorded hour after the gap is a keyframe');
  h.clock.setBounds({ firstHour: 0, lastHour: 500 });
  const seek = h.clock.getSnapshot().seekSequence;
  h.clock.scrub(420);
  h.frames.tick(80);
  assert.equal(frames.length, 4);
  assert.equal(frames.at(-1), null);
  h.clock.scrub(131);
  h.frames.tick(96);
  assert.equal(frames.length, 4, 'returning to the built range during the same drag still holds');
  h.clock.endScrub();
  h.clock.scrub(131);
  h.frames.tick(112);
  assert.equal(frames.at(-1)?.hour, 131);
  h.mount('third', 0, false);
  h.frames.tick(128);
  assert.equal(frames.length, 6);
  assert.equal(frames.at(-1), null);
  assert.equal(h.groups.length, groups);
  assert.equal(h.clock.getSnapshot().seekSequence, seek);
  h.clock.seek(131);
  h.clock.endScrub();
  assert.equal(h.clock.getSnapshot().seekSequence, seek + 1);
});

test('the whole page holds a reference gap recorded by another track and resumes when coverage agrees', async t => {
  const h = setup(410, undefined, [110, 130]);
  t.after(h.stop);
  const frames: (StripFrame | null)[] = [];
  const scrub: StripWidget['scrub'] = (_page, frame) => frames.push(frame);
  h.mount('reference', 0, true, undefined, false, scrub);
  h.mount('other', 0, true, undefined, false, scrub, {
    axisOffset: 500,
    branch: { firstHour: 500, lastHour: 910, gaps: [{ firstHour: 620, lastHour: 625, reason: 'Other gap' }] },
  });
  await h.until(() => h.page.status === 'ready');
  const holds: unknown[] = [];
  measureWith(new Proxy({ strip: (sample: { reason?: string }) => { if (sample.reason) holds.push(sample.reason); } },
    { get: (target, key) => Reflect.get(target, key) ?? (() => {}) }) as unknown as PerfRecorder);
  t.after(() => measureWith(null));
  const move = (hour: number) => { h.clock.scrub(hour); h.frames.tick(hour); };
  move(105);
  assert.equal(frames.length, 1);
  move(112);
  move(116);
  assert.deepEqual(frames.slice(1), [null]);
  move(121);
  assert.equal(frames.at(-1)?.gap?.kind, 'not-recorded');
  move(128);
  assert.equal(frames.at(-1), null);
  assert.deepEqual(holds, ['A widget recorded the reference gap']);
  move(131);
  assert.equal(frames.at(-1)?.hour, 131);
  h.clock.endScrub();
  move(112);
  assert.equal(holds.length, 2, 'the next drag logs its own hold');
});

test('late sources hold local markers and a fine-only first anchor, and give exact cursor states at every sampled hour', async t => {
  const h = setup(410, undefined, [110, 130]);
  t.after(h.stop);
  const exact = (hour: number) => hour === 101 ? { refused: 'Not recorded in this mod' } : { answer: { hour, value: hour * 3 } };
  h.answer(exact);
  const widget = h.mount('late', 5);
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.page.density, 16);
  assert.deepEqual(h.groups.filter(group => group.whole).map(group => group.hours[0]), [5]);
  assert.ok(!h.lattice.hours(1).includes(5));
  const store = widget.held.slots.get('source')!;
  const hours = h.lattice.hours(h.page.density);
  for (const hour of [...hours, ...[...hours].reverse()]) assert.deepEqual(store.at(hour), hour < 5 ? null : exact(hour));
  assert.equal(store.at(115), undefined);
  assert.ok(h.groups.every(group => group.hours.every(hour => hour >= 5 && (hour < 110 || hour > 130))));
  t.diagnostic(`${hours.length} keyframes, ${store.patchesApplied} cursor patches, ${store.bytes} bytes, ${hours.length * 2} comparisons, 0 differences`);
});

test('a new mounted body drops readiness, preserves the other strip, and releases its bytes when it leaves', async t => {
  const h = setup(200);
  t.after(h.stop);
  const first = h.mount('first');
  await h.until(() => h.page.status === 'ready');
  const original = h.groups.length;
  const second = h.mount('second');
  assert.equal(h.page.status, 'not ready');
  await h.until(() => h.page.status === 'ready');
  assert.ok(second.held.slots.get('source')!.bytes > 0);
  assert.ok(h.groups.slice(original).every(group => group.source === 'second'));
  second.remove();
  await h.until(() => h.page.status === 'ready');
  assert.equal(second.held.slots.size, 0);
  assert.ok(first.held.slots.get('source')!.bytes > 0);
  const beforeChart = h.groups.length;
  h.mount('chart', 0, false);
  await h.until(() => first.held.slots.size === 0);
  assert.equal(h.groups.length, beforeChart);
  assert.equal(h.page.status, 'not ready');
});

test('a body mounting mid-build keeps the existing body’s acknowledged coarse groups', async t => {
  const h = setup(3200);
  t.after(h.stop);
  h.mount('first');
  h.delay((_, signal) => h.groups.length === 2 ? new Promise((_resolve, reject) => {
    signal.addEventListener('abort', () => reject(signal.reason), { once: true });
  }) : undefined);
  await h.until(() => h.groups.length === 2);
  h.mount('second');
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.groups.filter(group => group.source === 'first' && group.whole).length, 1);
  assert.deepEqual(h.groups[2]?.hours, h.groups[1]?.hours, 'resume at the held anchor of the interrupted group');
});

test('a final fine receipt cannot make a superseded page ready', async t => {
  const h = setup(191);
  t.after(h.stop);
  let deliver: (() => void) | undefined;
  const first = h.mount('first', 0, true, change => change.kind === 'append' && change.fine && change.hours.at(-1) === 191
    ? new Promise(resolve => { deliver = resolve; }) : undefined);
  await h.until(() => deliver !== undefined);
  assert.equal(h.page.status, 'coarse');
  deliver!();
  h.mount('chart', 0, false);
  await h.until(() => first.held.slots.size === 0);
  assert.equal(h.page.status, 'not ready');
});

test('a late over-budget receipt lowers density after a fixed body joins', async t => {
  const budget = 6200;
  const h = setup(191, budget);
  t.after(h.stop);
  h.answer(hour => ({ answer: { hour, large: hour > 95 && hour % 12 === 5 ? 'x'.repeat(1000) : '' } }));
  let deliver: (() => void) | undefined;
  const first = h.mount('first', 0, true, change => change.kind === 'append' && change.fine && change.hours.at(-1) === 191 && !deliver
    ? new Promise(resolve => { deliver = resolve; }) : undefined);
  const strip = () => first.held.slots.get('source')!;
  await h.until(() => deliver !== undefined);
  assert.ok(strip().bytes > budget, 'the final receipt crosses the budget');
  h.mount('fixed', 0, true, undefined, true);
  deliver!();
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.page.density, 8);
  assert.ok(strip().bytes <= budget);
  assert.equal(h.groups.filter(group => group.whole).length, 1);
  assert.equal(h.groups.filter(group => group.hours.includes(101)).length, 1, 'the over-budget density is not tried again');
});

test('a failed group is isolated to single new keyframes and a remaining failure settles readiness', async t => {
  const h = setup(349);
  t.after(h.stop);
  h.delay(hours => { if (hours.includes(191)) throw new Error('Failed read'); });
  const widget = h.mount('broken');
  await h.until(() => h.page.status === 'will not follow');
  assert.deepEqual(h.groups.map(group => group.hours), [[0, 95, 191, 287, 349], [0], [0, 95], [95, 191]]);
  await h.until(() => widget.held.slots.size === 0);
});

test('overflow drops fine chains across the page and rebuilds at the same lower density for each body', async t => {
  const h = setup(191, 6200);
  t.after(h.stop);
  h.answer(hour => ({ answer: { hour, large: hour % 12 === 5 ? 'x'.repeat(330) : '' } }));
  const a = h.mount('a'), b = h.mount('b');
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.page.density, 8);
  assert.equal(h.groups.filter(group => group.whole).length, 2);
  assert.ok(h.groups.some(group => group.hours.includes(5)));
  const retained = [a, b].map(widget => widget.held.slots.get('source')!);
  assert.ok(retained.reduce((sum, store) => sum + store.bytes, 0) <= 6200);
  for (const store of retained) {
    assert.equal(store.at(5), undefined);
    for (const hour of h.lattice.hours(8)) assert.deepEqual(store.at(hour), { answer: { hour, large: '' } });
  }
});
