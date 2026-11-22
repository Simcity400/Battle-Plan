import { MessageChannel } from 'node:worker_threads';
import { serialize } from 'node:v8';
import { Followed, type FollowUpdate } from './followed.ts';
import { FollowDecoder } from '../dashboard/follow/decode.ts';
import { datedPanel } from '../dashboard/follow/messages.fixture.ts';
import { LatestReadings } from '../dashboard/follow/latest.ts';
import { FollowStore, type FollowEntry } from '../dashboard/follow/store.ts';
import { manualFrames, fakeTransport } from '../dashboard/follow/messages.fixture.ts';
import { readingStructure } from './panels/views.ts';

const SUBSCRIPTIONS = Number(process.argv[2] ?? 114), COUNTRIES = 128, SPEED = 1440, FPS = 60, FRAMES = 240, NATIVE_LEAD = 720;
const step = SPEED / FPS;

const arsenal = (day: number) => ({ missing: null, warnings: [], types: [1, 2, 3].map(ordinal => ({ ordinal, token: `nuke_${ordinal}`, name: `Nuke ${ordinal}` })),
  countries: Array.from({ length: COUNTRIES }, (_, i) => ({ tag: `C${i}`, countryId: i, total: i * 3 + day,
    counts: [1, 2, 3].map(ordinal => ({ ordinal, value: i + ordinal + day, ready: i })) })) });

const decoder = new FollowDecoder(), followed = new Followed(what => decoder.release(what));
const ids = Array.from({ length: SUBSCRIPTIONS }, (_, i) => i + 1);
for (const id of ids) followed.add(id, 'b', readingStructure('nuclear-arsenal'));
followed.bind('b', 9, 1);
let landed = -1;
const landThrough = (hour: number) => {
  for (; (landed + 1) * 24 <= hour; landed++) for (const id of ids) {
    const packet = datedPanel(id, (landed + 1) * 24, arsenal(landed + 1));
    packet[33] = 1;
    followed.land([decoder.item(packet.slice().buffer)]);
  }
};

const transport = fakeTransport(), store = new FollowStore(transport, manualFrames().schedule);
const entries = new Map<number, FollowEntry>();
for (const id of ids) entries.set(id, store.acquire({ panel: 'nuclear-arsenal', q: id, branchId: 'b' }).entry);
const page = new LatestReadings(() => 0);

const { port1, port2 } = new MessageChannel();
let heard: (data: unknown) => void = () => {};
port2.on('message', data => heard(data));
const roundTrip = (message: unknown) => new Promise<{ ms: number; data: { updates: FollowUpdate[] } }>(resolve => {
  const began = performance.now();
  heard = data => resolve({ ms: performance.now() - began, data: data as { updates: FollowUpdate[] } });
  port1.postMessage(message);
});

const percentile = (values: number[], at: number) => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * at))]!;
const sizes: number[] = [], posts: number[] = [], readings: number[] = [], applies: number[] = [], worker: number[] = [], bytes: number[] = [];
let head = 0;
landThrough(NATIVE_LEAD);
followed.drain(head);
const first = [...followed.updates(head + 2 * step)];
const firstBytes = serialize({ type: 'structure', updates: first }).byteLength, firstTrip = await roundTrip({ type: 'structure', updates: first });
for (const update of firstTrip.data.updates) page.land(update, head, false, (reading, placed) => transport.reading(reading, placed));
for (let frame = 0; frame < FRAMES; frame++) {
  head += step;
  landThrough(head + NATIVE_LEAD);
  const began = performance.now();
  followed.drain(head);
  const updates: FollowUpdate[] = [...followed.updates(head + 2 * step)];
  worker.push(performance.now() - began);
  const message = { type: 'structure', updates };
  sizes.push(serialize(message).byteLength);
  readings.push(updates.reduce((count, update) => count + (update.put?.length ?? 0), 0));
  bytes.push(updates.reduce((count, update) => count + (update.put?.filter(reading => !reading.kept).length ?? 0), 0));
  const trip = await roundTrip(message);
  posts.push(trip.ms);
  const applied = performance.now();
  for (const update of trip.data.updates) page.land(update, Math.floor(head), false, (reading, placed) => transport.reading(reading, placed));
  page.advance(Math.floor(head), false, (reading, placed) => transport.reading(reading, placed));
  for (const entry of entries.values()) void entry.structure;
  applies.push(performance.now() - applied);
}
port1.close();

const kib = (size: number) => `${(size / 1024).toFixed(0)} KiB`;
const ms = (values: number[]) => `p50 ${percentile(values, 0.5).toFixed(2)} ms, p95 ${percentile(values, 0.95).toFixed(2)} ms`;
console.log(`${SUBSCRIPTIONS} subscriptions x ${COUNTRIES}-country panels, ${SPEED} h/s at ${FPS} frames/s, ${FRAMES} frames, unthrottled`);
console.log(`first message after a seek: ${first.reduce((n, u) => n + (u.put?.length ?? 0), 0)} readings, ${kib(firstBytes)}, postMessage ${firstTrip.ms.toFixed(2)} ms`);
console.log(`per frame: readings p50 ${percentile(readings, 0.5)} max ${Math.max(...readings)}, of which carry panel bytes p50 ${percentile(bytes, 0.5)}; size p50 ${kib(percentile(sizes, 0.5))} p95 ${kib(percentile(sizes, 0.95))}`);
console.log(`page per frame: postMessage ${ms(posts)}; landing and placing the readings in the page store ${ms(applies)}`);
console.log(`worker per frame (lane 0): drain and diff ${ms(worker)}`);
