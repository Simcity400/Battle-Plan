import assert from 'node:assert/strict';
import test from 'node:test';
import { encode } from '@msgpack/msgpack';
import { busy, busyKinds } from '../../shell/busy.ts';
import type { StripParams } from '../../generated/dashboard.ts';
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

function packet(whole: boolean, states: unknown[]) {
  const parts = whole ? [encode(states[0])] : [];
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
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: 0, lastHour: last });
  const branch = { id: '1', firstHour: 0, lastHour: last, gaps: gap ? [{ firstHour: gap[0], lastHour: gap[1], reason: 'Not recorded' }] : [] };
  const lattice = stripLattice(branch, 0);
  const opened = new OpenedSources();
  let frame: (() => void) | undefined, settled = true, reads = 0;
  const gate = new PrepareGate(() => true, { request: run => { frame = run; return 1; }, cancel: () => { frame = undefined; } });
  const groups: { source: string; hours: number[]; whole: boolean; signal: AbortSignal; busy: boolean }[] = [];
  const tiles: { signal: AbortSignal; busy: boolean }[] = [];
  let answer = (hour: number): unknown => ({ answer: { hour, value: hour * 3 } });
  let delay: ((hours: number[], signal: AbortSignal) => Promise<void> | void) | undefined;
  let delayTile: ((signal: AbortSignal) => Promise<void>) | undefined;
  const client = { product: async (request: ProductRequest, signal: AbortSignal) => {
    if (request.product === 'series-tiles') {
      tiles.push({ signal, busy: clock.getSnapshot().playing || clock.getScrubPosition() !== null || !settled || busyKinds().requests > 0 });
      await delayTile?.(signal);
      return { ...request, data: 'tile reply' };
    }
    assert.equal(request.product, 'strip');
    assert.equal(opened.refusal(request), null);
    const { product, asks, whole } = request.params as StripParams;
    assert.equal(product, 'country-fuel');
    const hours = asks.map(ask => Number(ask.hour));
    groups.push({ source: request.sourceId, hours, whole, signal, busy: clock.getSnapshot().playing || !settled || busyKinds().requests > 0 });
    await delay?.(hours, signal);
    return { ...request, data: { data: packet(whole, hours.map(answer)) } };
  } } as unknown as DashboardClient;
  const page = new StripPage(lattice, clock, client, { budget, gate });
  const stop = page.start();
  const until = async (done: () => boolean) => {
    for (let tick = 0; tick < 5000; tick++) {
      if (done()) return;
      const run = frame; frame = undefined; run?.();
      await Promise.resolve();
    }
    assert.fail(`Strip page did not settle (${page.status})`);
  };
  const mount = (id: string, after: number = 0, supported = true, receipt?: (change: StripUpdate) => Promise<void> | void, fixed = false) => {
    opened.add({ id, revision: 'r', metadata: { branches: [branch] } } as never);
    const held = new WidgetStrips();
    const widget: StripWidget = { key: id, supported, fixed, settled: () => settled,
      requests: hour => {
        if (fixed) return [];
        reads++;
        return [{ slot: 'source', request: hour < after ? null : productRequest({
          product: 'country-fuel', sourceId: id, revision: 'r', params: { branchId: '1', hour: gatedHour(hour) },
        }) }];
      },
      store: async (_build, change) => {
        const result = held.update(change);
        await receipt?.(change);
        return result;
      },
    };
    return { held, remove: page.register(id, widget) };
  };
  return { page, clock, groups, tiles, lattice, gate, mount, until, cache: tileCache(client), reads: () => reads,
    stop() { clock.setPlaying(false); stop(); },
    settle: (value: boolean) => { settled = value; },
    delay: (value: typeof delay) => { delay = value; },
    delayTile: (value: typeof delayTile) => { delayTile = value; },
    answer: (value: typeof answer) => { answer = value; },
  };
}

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

test('the shipped switch allocates no page and leaves zero requests and zero held bytes', () => {
  const held = new WidgetStrips();
  let groups = 0;
  const page = configuredStripPage(() => {
    groups++;
    const h = setup(200);
    h.page.register('chart', { key: 'chart', supported: true, requests: () => [], settled: () => true,
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
  const answer = h.mount('answer');
  const chart = h.page.register('chart', {
    key: 'chart', supported: true, requests: () => [], settled: () => true,
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
  chart();
  await h.until(() => h.page.status === 'ready');
  assert.equal(h.cache.stripBytes, 0);
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
