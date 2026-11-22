import { Followed } from '../../draw/followed.ts';
import type { FollowUpdate } from '../../draw/followed';
import { StructureOutbox } from '../../draw/structureOutbox.ts';
import type { FromWorker, ToWorker } from '../../draw/kit';
import type { DashboardClient, DashboardSource } from '../client';
import type { PlaybackSnapshot } from '../playback';
import { followTransport, type FollowClock, type FollowLanes } from './connection.ts';
import { FollowStore, type FollowEntry } from './store.ts';
import { manualFrames } from './messages.fixture.ts';
import type { FollowItem } from './message';
import { liveKey } from '../../charts/live.ts';

export const ROUTE = 9, BRANCH = 'branch', VALUE = liveKey('1', 'v');

export interface Content { structure: string; value: number }
export const panelOf = (content: Content) => ({ s: content.structure,
  live: { groups: [{ table: 0, keys: ['1'], columns: [0], values: [content.value] }], rows: ['1'], kinds: [{ id: 'v', op: 'value', scale: 1 }], cells: [0, 0, 1, 0, 0, 0] } });

export class TestClock implements FollowClock {
  snapshot: PlaybackSnapshot = { position: 0, hour: 0, fraction: 0, playing: true, hoursPerSecond: 1440, sequence: 0, seekSequence: 0, bounds: { firstHour: 0, lastHour: 1_000_000 } };
  private readonly published = new Set<(previous: PlaybackSnapshot, next: PlaybackSnapshot) => void>();
  private readonly anchored = new Set<() => void>();
  readonly frames = new Set<() => void>();
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.anchored.add(listener); return () => { this.anchored.delete(listener); }; };
  onPublish = (listener: (previous: PlaybackSnapshot, next: PlaybackSnapshot) => void) => { this.published.add(listener); return () => { this.published.delete(listener); }; };
  publish(change: Partial<PlaybackSnapshot>) {
    const previous = this.snapshot, position = change.position ?? previous.position;
    this.snapshot = { ...previous, forwardSeek: false, ...change, position, hour: Math.floor(position), fraction: position % 1, sequence: previous.sequence + 1 };
    for (const listener of this.published) listener(previous, this.snapshot);
    for (const listener of this.frames) listener();
    for (const listener of this.anchored) listener();
  }
  frame(position: number) { this.publish({ position }); }
  pause() { this.publish({ playing: false }); }
  resume() { this.publish({ playing: true }); }
  seek(position: number) {
    const s = this.snapshot;
    this.publish({ position, playing: false, seekSequence: s.seekSequence + 1, forwardSeek: position > s.position });
  }
  step(direction: 1 | -1) {
    const s = this.snapshot, position = s.hour + direction;
    this.publish(direction > 0 ? { position, playing: false, forwardSeek: true } : { position, playing: false, seekSequence: s.seekSequence + 1 });
  }
}

export class TestLane implements FollowLanes {
  readonly followed = new Followed(() => {});
  readonly channel: FollowUpdate[][] = [];
  readonly posted: ToWorker[] = [];
  readonly delivered: FollowUpdate[][] = [];
  private readonly listeners = new Set<(message: FromWorker) => void>();
  private head = 0;
  private ahead = 0;
  private readonly outbox = new StructureOutbox(() => {
    const updates = [...this.followed.updates(this.head + this.ahead)];
    if (updates.length) this.channel.push(updates);
    return updates.length > 0;
  });
  private readonly structure: (data: { s: string }) => string;
  constructor(structure = (data: { s: string }) => data.s) { this.structure = structure; }
  post(message: ToWorker) {
    this.posted.push(message);
    if (message.type === 'add') this.followed.add(message.id, message.branch, panel => (panel.data as { s: string } | undefined) && this.structure(panel.data as { s: string }));
    else if (message.type === 'remove') this.followed.remove(message.id);
    else if (message.type === 'seek') this.followed.seek(message.branch, message.seek, message.epoch, message.retainThrough);
    else if (message.type === 'bind') this.followed.bind(message.branch, message.route, message.seek);
    else if (message.type === 'consumed') this.followed.consume(message.branch, message.epoch, message.hour);
  }
  listen(listener: (message: FromWorker) => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  open() { return Promise.resolve(); }
  clock() { return () => {}; }
  rate() { return 60; }
  onRate() { return () => {}; }
  land(items: FollowItem[]) {
    for (const item of items) if (item.kind === 'failure') for (const listener of this.listeners) listener({ type: 'followFailure', reason: item.reason });
    this.followed.land(items);
  }
  frame(head: number, ahead: number) { this.head = head; this.ahead = ahead; this.followed.drain(head); this.outbox.flush(); }
  deliver() {
    const updates = this.channel.shift();
    if (!updates) return false;
    this.delivered.push(updates);
    for (const listener of this.listeners) listener({ type: 'structure', updates });
    this.outbox.taken();
    return true;
  }
}

const SOURCE = { id: 'recording', revision: '1' } as DashboardSource;

export async function pipeline(ids: readonly number[], structure?: (data: { s: string }) => string) {
  const clock = new TestClock(), lane = new TestLane(structure);
  const client = { openFollow: () => Promise.resolve(), product: (request: { product: string }) =>
    Promise.resolve({ data: request.product === 'follow' ? { available: true, route: ROUTE } : {} }) } as unknown as DashboardClient;
  const transport = followTransport(client, SOURCE, clock, 0, lane);
  let next = 0;
  const order = [...ids];
  const store = new FollowStore(transport, manualFrames().schedule, () => order[next++]!);
  const entries = new Map<number, FollowEntry>(ids.map(id => [id, store.acquire({ panel: 'test', q: id, branchId: BRANCH }).entry]));
  await new Promise(resolve => setTimeout(resolve, 0));
  return { clock, lane, store, entries };
}

export function panels(seek: number, completed: number | undefined, readings: { id: number; hour: number; content: Content }[]): FollowItem {
  return { kind: 'panels', batch: { route: ROUTE, clock: 0, seek, completed,
    panels: readings.map(({ id, hour, content }) => ({ id, hour, stepped: true, panel: { data: panelOf(content), refused: null } })) } };
}
