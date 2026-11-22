import type { DashboardClient, DashboardSource } from '../client';
import type { FromWorker, ToWorker } from '../../draw/kit';
import type { PlaybackController, PlaybackSnapshot } from '../playback';
import { FollowStore, queryBranch } from './store.ts';
import type { Availability, FollowTransport } from './store.ts';
import { baseClient, PerClient } from '../prepare.ts';
import { pageClock } from '../widgetClock.ts';
import { drawClock, drawPost, openFollow, onDraw, playbackRate, onPlaybackRate } from '../pulled.ts';
import { LatestReadings, type Show } from './latest.ts';

let nextId = 1;
const ids = () => nextId++;

const clocks = new WeakMap<PlaybackController, PerClient<Map<string, FollowStore>>>();

const NO_CHANNEL: Availability = { available: false, reason: 'This connection has no channel for pushed answers' };

function availability(data: unknown): Availability {
  const reply = data as { available?: unknown; reason?: unknown } | null;
  if (reply?.available === true) return { available: true };
  return { available: false, reason: typeof reply?.reason === 'string' ? reply.reason : 'The service did not say whether it follows this' };
}

export interface FollowLanes {
  post(message: ToWorker): void;
  listen(listener: (message: FromWorker) => void): () => void;
  open(client: DashboardClient): Promise<void>;
  clock(clock: PlaybackController): () => void;
  rate(): number;
  onRate(changed: () => void): () => void;
}

const drawingLanes: FollowLanes = { post: message => drawPost(message), listen: onDraw, open: openFollow, clock: drawClock, rate: playbackRate, onRate: onPlaybackRate };

export type FollowClock = Pick<PlaybackController, 'getSnapshot' | 'subscribe' | 'onPublish'>;

export function followTransport(client: DashboardClient, source: DashboardSource, clock: FollowClock, offset: number, lanes: FollowLanes = drawingLanes): FollowTransport {
  const drawPost = (message: ToWorker) => lanes.post(message);
  const nameOf = (branch: string) => JSON.stringify([source.id, source.revision, offset, branch]);
  interface Branch { ready: boolean; ids: Set<number>; route?: number; key?: string; toldSeek?: number; toldPaused?: number | null; serial: Promise<unknown>; ticket: number; epoch: number; consumed?: number }
  const branches = new Map<string, Branch>(), inFlight = new Map<number, Promise<Availability>>();
  const owners = new Map<number, Branch>();
  const readings = new LatestReadings(id => owners.get(id)?.epoch);
  let receive: Show = () => {}, failed: (reason: string) => void = () => {}, listening = false;
  const ask = (product: string, params: Record<string, unknown>) => client.product({
    sourceId: source.id, revision: source.revision, product, params,
  }, new AbortController().signal);
  const pageHour = (s: PlaybackSnapshot) => s.hour + offset;
  const consume = (s: PlaybackSnapshot) => {
    for (const [name, b] of branches) if (b.ready && b.consumed !== pageHour(s)) {
      b.consumed = pageHour(s);
      drawPost({ type: 'consumed', branch: nameOf(name), epoch: b.epoch, hour: b.consumed });
    }
  };
  const renew = (b: Branch, name: string, carry: boolean, retainThrough: number | undefined, seek: number | null) => {
    b.epoch++; b.consumed = undefined;
    readings.renew(b.ids, carry);
    if (seek !== null) drawPost({ type: 'seek', branch: nameOf(name), seek, epoch: b.epoch, retainThrough });
  };
  const published = (previous: PlaybackSnapshot, next: PlaybackSnapshot) => {
    const sought = next.seekSequence !== previous.seekSequence, paused = !next.playing;
    const stopping = paused && previous.playing, stepped = paused && !previous.playing && next.hour !== previous.hour;
    if (sought || stopping || stepped) {
      const carry = !sought || !!next.forwardSeek;
      for (const [name, b] of branches) if (b.ready) renew(b, name, carry, carry ? pageHour(next) : undefined, next.seekSequence);
    }
    if (!listening) return;
    if (sought || stopping || stepped || previous.playing !== next.playing) readings.refresh(pageHour(next), paused, receive);
    else readings.advance(pageHour(next), paused, receive);
    consume(next);
  };
  const anchor = () => {
    const s = clock.getSnapshot();
    const paused = s.playing ? null : pageHour(s);
    const frameRate = lanes.rate();
    const key = JSON.stringify([s.playing, s.hoursPerSecond, s.seekSequence, paused, frameRate]);
    for (const [name, b] of branches) {
      if (!b.ready || b.key === key) continue;
      const sought = b.toldSeek !== s.seekSequence || paused !== null && b.toldPaused != null && paused !== b.toldPaused;
      b.key = key; b.toldSeek = s.seekSequence; b.toldPaused = paused;
      const ticket = ++b.ticket;
      const composedAt = performance.timeOrigin + (s.composedAt ?? performance.now());
      b.serial = b.serial.then(async () => {
        if (branches.get(name) !== b) return;
        await ask('follow-clock', { branchId: name, clock: offset, hour: s.position + offset,
          playing: s.playing, speed: s.hoursPerSecond, frameRate, seek: s.seekSequence, sought, composedAt, sentAt: performance.timeOrigin + performance.now() });
      }).catch(() => { if (b.ticket === ticket) b.key = undefined; });
    }
  };
  let stopWorkerClock: (() => void) | undefined, stopPublish: (() => void) | undefined;
  const remove = (id: number) => {
    const owner = owners.get(id);
    if (!owner) return;
    owners.delete(id);
    readings.forget([id]);
    owner.ids.delete(id);
    drawPost({ type: 'remove', id });
    if (!owner.ids.size) {
      for (const [name, b] of branches) if (b === owner) branches.delete(name);
      if (!branches.size) { stopWorkerClock?.(); stopWorkerClock = undefined; stopPublish?.(); stopPublish = undefined; }
    }
  };
  return {
    follow(id, query) {
      const branch = queryBranch(query);
      if (!client.openFollow || branch === null) return Promise.resolve(NO_CHANNEL);
      let owner = branches.get(branch);
      if (!owner) {
        owner = { ready: false, ids: new Set(), serial: Promise.resolve(), ticket: 0, epoch: 0 };
        branches.set(branch, owner);
      }
      owner.ids.add(id);
      owners.set(id, owner);
      stopWorkerClock ??= lanes.clock(clock as PlaybackController);
      stopPublish ??= clock.onPublish(published);
      const product = typeof query.panel === 'string' ? query.panel : undefined;
      drawPost({ type: 'add', branch: nameOf(branch), id, ...(product !== undefined && { product }) });
      const registered = lanes.open(client).then(() => ask('follow', { id, query, branchId: branch, clock: offset })).then(reply => {
        const told = availability(reply.data);
        if (!told.available) remove(id);
        else if (owners.get(id) === owner) {
          const route = (reply.data as { route: number }).route;
          if (!Number.isInteger(route)) throw new Error('A followed subscription has no route');
          owner.route = route;
          drawPost({ type: 'bind', branch: nameOf(branch), route, seek: clock.getSnapshot().seekSequence });
          owner.ready = true;
          anchor();
          consume(clock.getSnapshot());
        }
        return told;
      });
      inFlight.set(id, registered);
      void registered.then(() => inFlight.delete(id), () => { inFlight.delete(id); remove(id); });
      return registered;
    },
    unfollow(id) {
      if (!owners.has(id)) return;
      remove(id);
      void Promise.resolve(inFlight.get(id)).then(() => ask('unfollow', { id })).catch(() => {});
    },
    listen(show, fail) {
      receive = show; failed = fail; listening = true;
      const off = clock.subscribe(anchor), offRate = lanes.onRate(anchor);
      const stop = lanes.listen(message => {
        if (message.type === 'followFailure' || (message.type === 'failed' && message.page === -1)) {
          const s = clock.getSnapshot();
          for (const [name, b] of branches) if (b.ready) renew(b, name, false, undefined, null);
          readings.refresh(pageHour(s), !s.playing, receive);
          failed(message.reason);
          return;
        }
        if (message.type !== 'structure') return;
        const s = clock.getSnapshot();
        for (const update of message.updates) readings.land(update, pageHour(s), !s.playing, receive);
      });
      return () => { stop(); off(); offRate(); listening = false; receive = () => {}; failed = () => {}; };
    },
  };
}

export function followStore(client: DashboardClient, source: DashboardSource, widgetClock: PlaybackController, offset = 0): FollowStore {
  const clock = pageClock(widgetClock);
  let stores = clocks.get(clock);
  if (!stores) clocks.set(clock, stores = new PerClient());
  let held = stores.get(client);
  if (!held) stores.set(client, held = new Map());
  const key = `${source.id}\u0000${source.revision}\u0000${offset}`;
  let store = held.get(key);
  if (!store) {
    const transport = followTransport(baseClient(client), source, clock, offset);
    store = new FollowStore(transport, undefined, ids);
    held.set(key, store);
  }
  return store;
}
