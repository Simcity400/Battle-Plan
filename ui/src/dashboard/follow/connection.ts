import type { DashboardClient, DashboardSource } from '../client';
import type { PlaybackController } from '../playback';
import { FollowStore, queryBranch } from './store.ts';
import type { Availability, FollowTransport } from './store.ts';
import { baseClient, PerClient } from '../prepare.ts';
import { pageClock } from '../widgetClock.ts';
import { drawClock, drawPost, openFollow, onDraw, playbackRate, onPlaybackRate } from '../pulled.ts';
import { LatestReadings } from './latest.ts';
import type { FollowUpdate, PageReading } from '../../draw/followed';

let nextId = 1;
const ids = () => nextId++;
const shown = new Map<number, PageReading>();

let nextInspection = 1;
export function inspectFollow(id: number): Promise<PageReading | undefined> {
  const request = nextInspection++;
  return new Promise<boolean>(resolve => {
    const off = onDraw(message => {
      if (message.type === 'inspection' && message.request === request) { off(); resolve(true); }
      if (message.type === 'failed' || message.type === 'followFailure') { off(); resolve(false); }
    });
    drawPost({ type: 'inspect', id, request });
  }).then(answered => answered ? shown.get(id) : undefined);
}

const clocks = new WeakMap<PlaybackController, PerClient<Map<string, FollowStore>>>();

const NO_CHANNEL: Availability = { available: false, reason: 'This connection has no channel for pushed answers' };

function availability(data: unknown): Availability {
  const reply = data as { available?: unknown; reason?: unknown } | null;
  if (reply?.available === true) return { available: true };
  return { available: false, reason: typeof reply?.reason === 'string' ? reply.reason : 'The service did not say whether it follows this' };
}

function followTransport(client: DashboardClient, source: DashboardSource, clock: PlaybackController, offset: number,
  seek: (branch: string, retainThrough?: number) => void): FollowTransport {
  const nameOf = (branch: string) => JSON.stringify([source.id, source.revision, offset, branch]);
  interface Branch { ready: boolean; ids: Set<number>; route?: number; key?: string; sequence?: number; paused?: number | null; serial: Promise<unknown>; ticket: number; seek: number }
  const branches = new Map<string, Branch>(), inFlight = new Map<number, Promise<Availability>>();
  const readings = new LatestReadings(shown);
  const ask = (product: string, params: Record<string, unknown>) => client.product({
    sourceId: source.id, revision: source.revision, product, params,
  }, new AbortController().signal);
  const anchor = () => {
    const s = clock.getSnapshot();
    const hour = Math.floor(s.position) + offset, paused = s.playing ? null : hour;
    const frameRate = playbackRate();
    const key = JSON.stringify([s.playing, s.hoursPerSecond, s.seekSequence, paused, frameRate]);
    for (const [name, b] of branches) {
      if (!b.ready || b.key === key) continue;
      const sought = b.sequence !== s.seekSequence || (paused !== null && b.paused != null && paused !== b.paused);
      const stopping = paused !== null && b.paused === null;
      b.key = key; b.sequence = s.seekSequence; b.paused = paused;
      if (sought || stopping) {
        readings.forget(b.ids);
        const retainThrough = (sought && s.forwardSeek) || (!sought && stopping) ? hour : undefined;
        seek(name, retainThrough);
        b.seek = s.seekSequence;
        drawPost({ type: 'seek', branch: nameOf(name), seek: b.seek, retainThrough });
      }
      const ticket = ++b.ticket;
      const composedAt = performance.timeOrigin + (s.composedAt ?? performance.now());
      b.serial = b.serial.then(async () => {
        if (branches.get(name) !== b) return;
        await ask('follow-clock', { branchId: name, clock: offset, hour: s.position + offset,
          playing: s.playing, speed: s.hoursPerSecond, frameRate, seek: s.seekSequence, sought, composedAt, sentAt: performance.timeOrigin + performance.now() });
      }).catch(() => { if (b.ticket === ticket) b.key = undefined; });
    }
  };
  let stopWorkerClock: (() => void) | undefined;
  const owners = new Map<number, Branch>();
  const remove = (id: number) => {
    const owner = owners.get(id);
    if (!owner) return;
    owners.delete(id);
    readings.forget([id]);
    owner.ids.delete(id);
    drawPost({ type: 'remove', id });
    if (!owner.ids.size) {
      for (const [name, b] of branches) if (b === owner) branches.delete(name);
      if (!branches.size) { stopWorkerClock?.(); stopWorkerClock = undefined; }
    }
  };
  return {
    follow(id, query) {
      const branch = queryBranch(query);
      if (!client.openFollow || branch === null) return Promise.resolve(NO_CHANNEL);
      let owner = branches.get(branch);
      if (!owner) {
        owner = { ready: false, ids: new Set(), serial: Promise.resolve(), ticket: 0, seek: 0 };
        branches.set(branch, owner);
      }
      owner.ids.add(id);
      owners.set(id, owner);
      stopWorkerClock ??= drawClock(clock);
      drawPost({ type: 'add', branch: nameOf(branch), id, product: typeof query.panel === 'string' ? query.panel : undefined });
      const registered = openFollow(client).then(() => ask('follow', { id, query, branchId: branch, clock: offset })).then(reply => {
        const told = availability(reply.data);
        if (!told.available) remove(id);
        else if (owners.get(id) === owner) {
          const route = (reply.data as { route: number }).route;
          if (!Number.isInteger(route)) throw new Error('A followed subscription has no route');
          owner.route = route;
          drawPost({ type: 'bind', branch: nameOf(branch), route, seek: clock.getSnapshot().seekSequence });
          owner.ready = true;
          anchor();
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
    listen(receive, fail) {
      let stopFrame: (() => void) | undefined;
      const hour = () => Math.floor(clock.positionAt()) + offset;
      const watch = () => {
        if (readings.waiting) stopFrame ??= clock.subscribeFrame(advance);
        else { stopFrame?.(); stopFrame = undefined; }
      };
      const advance = () => { readings.advance(hour(), receive); watch(); };
      const land = (update: FollowUpdate) => {
        const { id, seek } = update.now;
        if (owners.get(id)?.seek === seek && clock.getSnapshot().seekSequence === seek) readings.land(update, hour(), receive);
      };
      const off = clock.subscribe(anchor), offRate = onPlaybackRate(anchor);
      const stop = onDraw(message => {
        if (message.type === 'followFailure' || (message.type === 'failed' && message.page === -1)) {
          readings.forget(owners.keys());
          fail(message.reason);
          watch();
        }
        if (message.type === 'inspection' && message.update) { land(message.update); watch(); }
        if (message.type === 'structure') { for (const update of message.updates) land(update); watch(); }
      });
      return () => { stopFrame?.(); stop(); off(); offRate(); };
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
    const transport = followTransport(baseClient(client), source, clock, offset, (branch, retainThrough) => store?.seek(branch, retainThrough));
    store = new FollowStore(transport, undefined, ids);
    held.set(key, store);
  }
  return store;
}
