import { MessageChannel } from 'node:worker_threads';
import { serialize } from 'node:v8';
import { Followed, type FollowUpdate } from './followed.ts';
import { FollowDecoder } from '../dashboard/follow/decode.ts';
import { datedPanel } from '../dashboard/follow/messages.fixture.ts';
import { pagePanel } from '../dashboard/follow/latest.ts';

const SUBSCRIPTIONS = Number(process.argv[2] ?? 114), COUNTRIES = 128, SPEED = 1440, FPS = 60, FRAMES = 240, NATIVE_LEAD = 720;
const step = SPEED / FPS;

const arsenal = (day: number) => ({ missing: null, warnings: [], types: [1, 2, 3].map(ordinal => ({ ordinal, token: `nuke_${ordinal}`, name: `Nuke ${ordinal}` })),
  countries: Array.from({ length: COUNTRIES }, (_, i) => ({ tag: `C${i}`, countryId: i, total: i * 3 + day,
    counts: [1, 2, 3].map(ordinal => ({ ordinal, value: i + ordinal + day, ready: i })) })) });

const decoder = new FollowDecoder(), followed = new Followed(what => decoder.release(what));
const ids = Array.from({ length: SUBSCRIPTIONS }, (_, i) => i + 1);
for (const id of ids) followed.add(id, 'b');
followed.bind('b', 9, 1);
let landed = -1;
const landThrough = (hour: number) => {
  for (; (landed + 1) * 24 <= hour; landed++) for (const id of ids) {
    const packet = datedPanel(id, (landed + 1) * 24, arsenal(landed + 1));
    packet[33] = 1;
    followed.land([decoder.item(packet.slice().buffer)]);
  }
};

const { port1, port2 } = new MessageChannel();
let heard: (data: unknown) => void = () => {};
port2.on('message', data => heard(data));
const roundTrip = (message: unknown) => new Promise<number>(resolve => {
  const began = performance.now();
  heard = () => resolve(performance.now() - began);
  port1.postMessage(message);
});

const percentile = (values: number[], at: number) => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * at))]!;
const sizes: number[] = [], times: number[] = [], readings: number[] = [], decodes: number[] = [];
let head = 0;
landThrough(NATIVE_LEAD);
followed.drain(head);
const first = [...followed.updates(head + 2 * step)];
const firstBytes = serialize({ type: 'structure', updates: first }).byteLength, firstTime = await roundTrip({ type: 'structure', updates: first });
for (let frame = 0; frame < FRAMES; frame++) {
  head += step;
  landThrough(head + NATIVE_LEAD);
  followed.drain(head);
  const updates: FollowUpdate[] = [...followed.updates(head + 2 * step)];
  const message = { type: 'structure', updates };
  sizes.push(serialize(message).byteLength);
  readings.push(updates.reduce((count, update) => count + (update.put?.length ?? 0), 0));
  times.push(await roundTrip(message));
  const began = performance.now();
  for (const update of updates) for (const reading of update.put ?? []) void pagePanel(reading.panel)?.data;
  decodes.push(performance.now() - began);
}
port1.close();

const leads = followed.ahead(1).length;
const whole = { type: 'structure', updates: ids.map(id => ({ now: followed.current(id), ahead: followed.ahead(id).map(lead => ({ hour: lead.hour, panel: lead.panel })) })) };
const wholeBytes = serialize(whole).byteLength, wholeTime = await new Promise<number>(resolve => {
  const { port1: a, port2: b } = new MessageChannel(), began = performance.now();
  b.on('message', () => { resolve(performance.now() - began); a.close(); });
  a.postMessage(whole);
});
const kib = (bytes: number) => `${(bytes / 1024).toFixed(0)} KiB`;
console.log(`${SUBSCRIPTIONS} subscriptions x ${COUNTRIES}-country panels, ${SPEED} h/s at ${FPS} frames/s, ${FRAMES} frames`);
console.log(`first message after a seek: ${first.reduce((n, u) => n + (u.put?.length ?? 0), 0)} readings, ${kib(firstBytes)}, postMessage ${firstTime.toFixed(2)} ms`);
console.log(`per frame: readings p50 ${percentile(readings, 0.5)} max ${Math.max(...readings)}; size p50 ${kib(percentile(sizes, 0.5))} p95 ${kib(percentile(sizes, 0.95))};` +
  ` postMessage p50 ${percentile(times, 0.5).toFixed(2)} ms p95 ${percentile(times, 0.95).toFixed(2)} ms; page decode of every new panel p50 ${percentile(decodes, 0.5).toFixed(2)} ms p95 ${percentile(decodes, 0.95).toFixed(2)} ms`);
console.log(`for comparison, resending each subscription's reading and its ${leads} native leads as decoded objects: ${kib(wholeBytes)}, postMessage ${wholeTime.toFixed(1)} ms`);
