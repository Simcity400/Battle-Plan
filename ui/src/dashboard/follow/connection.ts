/** The follow store of one open recording, bound to the dashboard client: `follow` and `unfollow` go as
 * requests (the host forwards them to the viewer, where the playhead is), pushed messages come on the client's
 * raw channel, and the clock's anchors (play, pause, speed, seek) go once each so the playhead's clock runs as
 * the page's. A client with no channel follows nothing: every query is not available and widgets ask as before. */
import type { DashboardClient, DashboardSource } from '../client';
import type { PlaybackController } from '../playback';
import { FollowStore, queryBranch } from './store.ts';
import type { Availability, FollowQuery, FollowTransport } from './store.ts';
import { baseClient, PerClient } from '../prepare.ts';
import { engine } from '../../engine/frame.ts';

/** Ids are the viewer's, shared by every store this page makes. */
let nextId = 1;
const ids = () => nextId++;

const stores = new PerClient<Map<string, FollowStore>>();

const NO_CHANNEL: Availability = { available: false, reason: 'This connection has no channel for pushed answers' };

function availability(data: unknown): Availability {
  const reply = data as { available?: unknown; reason?: unknown } | null;
  if (reply?.available === true) return { available: true };
  return { available: false, reason: typeof reply?.reason === 'string' ? reply.reason : 'The service did not say whether it follows this' };
}

/** The transport of `source` over `client`, the clock's anchors sent for every timeline followed. The track shows game
 * hour position + `offset` (its `axisOffset`): that hour is sent, and the offset with every request, so each mapping
 * of one timeline has a playhead of its own. A seek while paused is told to `seek` (branch, game hour), whose returned
 * function is called if the service follows nothing there. */
export function followTransport(client: DashboardClient, source: DashboardSource, clock?: PlaybackController,
  seek?: (branchId: string, hour: number) => () => void, offset = 0): FollowTransport {
  const branches = new Set<string>();
  const ask = (product: string, params: Record<string, unknown>) =>
    client.product<unknown>({ sourceId: source.id, revision: source.revision, product, params }, new AbortController().signal);
  let anchored = '';
  let seeks = -1;
  /** The hour last anchored while paused: a paused step to another hour (Next hour) is a seek there. */
  let pausedAt: number | null = null;
  const anchor = () => {
    if (!clock || !branches.size) return;
    const snapshot = clock.getSnapshot();
    const hour = snapshot.playing ? null : Math.floor(snapshot.position);
    const sought = snapshot.seekSequence !== seeks || (hour !== null && pausedAt !== null && hour !== pausedAt);
    const key = JSON.stringify([snapshot.playing, snapshot.hoursPerSecond, snapshot.seekSequence, hour]);
    if (key === anchored) return;
    anchored = key;
    seeks = snapshot.seekSequence;
    pausedAt = hour;
    for (const branchId of branches) {
      const awaited = sought && !snapshot.playing ? seek?.(branchId, Math.floor(snapshot.position) + offset) : undefined;
      void ask('follow-clock', { branchId, clock: offset, hour: snapshot.position + offset, playing: snapshot.playing, speed: snapshot.hoursPerSecond, sought })
        .then(reply => { if (!availability(reply.data).available) awaited?.(); }, () => awaited?.());
    }
  };
  const stopClock = clock?.subscribe(anchor);
  return {
    follow(id: number, query: FollowQuery) {
      if (!client.onFollowMessages) return Promise.resolve(NO_CHANNEL);
      const branchId = queryBranch(query);
      if (branchId !== null && !branches.has(branchId)) {
        branches.add(branchId);
        anchored = '';
        seeks = -1;
      }
      return ask('follow', { id, branchId, clock: offset, query }).then(reply => {
        const told = availability(reply.data);
        if (told.available) anchor();
        return told;
      });
    },
    unfollow(id: number) {
      if (client.onFollowMessages) void ask('unfollow', { id }).catch(() => {});
    },
    listen(receive) {
      if (!client.onFollowMessages) return () => stopClock?.();
      let stopped = false, stop: (() => void) | null = null;
      // A message costs a push where it arrives; every message since the last frame lands in its data phase, in order.
      const land = engine.batch<ArrayBuffer | Uint8Array>(messages => { if (!stopped) for (const bytes of messages) receive(bytes); });
      void client.onFollowMessages(land).then(unlisten => { if (stopped) unlisten(); else stop = unlisten; });
      return () => { stopped = true; stop?.(); stopClock?.(); };
    },
  };
}

/** The one store of `source` (its revision) on `client` for a track showing game hour position + `offset`. */
export function followStore(client: DashboardClient, source: DashboardSource, clock?: PlaybackController, offset = 0): FollowStore {
  let held = stores.get(client);
  if (!held) stores.set(client, held = new Map());
  const key = `${source.id}\u0000${source.revision}\u0000${offset}`;
  let store = held.get(key);
  if (!store) {
    let made: FollowStore | null = null;
    made = new FollowStore(followTransport(baseClient(client), source, clock, (branchId, hour) => made?.seek(branchId, hour) ?? (() => {}), offset), undefined, ids);
    held.set(key, store = made);
  }
  return store;
}
