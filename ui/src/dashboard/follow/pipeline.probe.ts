import { Lane } from '../../draw/lane.ts';
import { fieldsView } from '../../draw/views/fields.ts';
import { Page, type Bitmap, type PageMessage } from './page.ts';
import { splitForLanes } from './split.ts';

interface Fake extends Bitmap { closed: boolean }
const NATIVE = 3, ROUTE = 1, LANES = 2, WIDGETS = 114, FRAMES = 240, HOURS_PER_FRAME = 24;

function table(rows: number, day: number, sub: number, changed: readonly number[] | null): Uint8Array {
  const list = changed ?? Array.from({ length: rows }, (_, row) => row), n = list.length, columns = 1;
  const texts = changed ? [] : list.map(row => `C${row.toString(36).padStart(3, '0')}`);
  const bytes = texts.reduce((sum, text) => sum + text.length, 0);
  const pad = (at: number) => (at + 7) & ~7;
  const statesAt = 16 + n * 16 + n * columns * 8, reasonsAt = pad(statesAt + n * columns), namesAt = pad(reasonsAt + n * 4), endsAt = pad(namesAt + n * 4);
  const textAt = pad(endsAt + (texts.length + 1) * 4), size = pad(textAt + bytes);
  const out = new Uint8Array(size), view = new DataView(out.buffer);
  view.setUint8(0, changed ? 3 : 1); view.setUint8(1, 1); view.setUint32(4, n, true); view.setUint32(8, columns, true); view.setUint32(12, texts.length, true);
  list.forEach((row, at) => {
    view.setBigUint64(16 + at * 8, BigInt(row + 1), true);
    view.setBigInt64(16 + n * 16 + at * 8, BigInt((row * 7919 + day * 31 + sub) % 1_000_003), true);
    view.setUint32(reasonsAt + at * 4, 0xffff_ffff, true);
    view.setUint32(namesAt + at * 4, changed ? 0xffff_ffff : at, true);
  });
  let end = 0, put = textAt;
  view.setUint32(endsAt, 0, true);
  texts.forEach((text, at) => { for (const char of text) out[put++] = char.charCodeAt(0); end += text.length; view.setUint32(endsAt + (at + 1) * 4, end, true); });
  return out;
}

function message(hour: number, answers: { id: number; body: Uint8Array }[]) {
  const out = new Uint8Array(24 + answers.reduce((sum, answer) => sum + 16 + answer.body.length, 0)), view = new DataView(out.buffer);
  view.setUint32(0, 0x334c4f46, true); view.setUint32(4, NATIVE << 1, true); view.setInt32(8, hour, true); view.setUint32(12, answers.length, true);
  let at = 24;
  for (const answer of answers) { view.setUint32(at, answer.id, true); view.setInt32(at + 4, hour, true); view.setUint32(at + 8, answer.body.length, true); out.set(answer.body, at + 16); at += 16 + answer.body.length; }
  return out;
}

const flush = async () => { for (let i = 0; i < 3; i++) await new Promise<void>(resolve => setImmediate(resolve)); };
const quantile = (values: number[], q: number) => { const sorted = [...values].sort((a, b) => a - b); return sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] ?? 0; };

async function workload(name: string, countries: number, change: number, distinct: boolean) {
  const lanes: Lane<Fake>[] = [];
  const page = new Page<Fake>({ lanes: LANES, post: (lane, m: PageMessage) => {
    for (const [index, target] of lanes.entries()) if (lane === 'all' || lane === index) {
      if (m.type === 'frame') target.frame(m); else if (m.type === 'stream') target.stream(m.route, m.seek, m.target);
    }
  } });
  let laneMs = 0;
  for (let index = 0; index < LANES; index++) lanes.push(new Lane<Fake>({
    lane: index, lanes: LANES, views: () => fieldsView(() => undefined),
    crop: async sources => sources.map((): Fake => ({ closed: false, width: 1, close() { this.closed = true; } })),
    present: presentation => page.offer(presentation), release: () => {}, charge: () => {}, fail: () => {}, later: () => {},
  }));
  const subs = new Set<number>();
  for (let widget = 0; widget < WIDGETS; widget++) {
    const lane = widget % LANES, sub = distinct ? (widget + 1) * LANES + lane : LANES + lane;
    if (!subs.has(sub)) { subs.add(sub); lanes[lane]!.follow(sub, 'followed'); lanes[lane]!.bindRoute(sub, ROUTE, NATIVE); }
    lanes[lane]!.mount(widget, [sub], 'fields');
    page.mount(widget, { route: ROUTE, lane, surfaces: [] });
  }
  const bytes: number[] = [], ingest: number[] = [], serve: number[] = [], commit: number[] = [], ages: number[] = [];
  page.publish(0, { playing: true, step: HOURS_PER_FRAME, speed: 1440, seek: 0 });
  for (let frame = 0; frame < FRAMES; frame++) {
    const hour = frame * HOURS_PER_FRAME, day = frame;
    const sparse = change < 1 && frame > 0 ? Array.from({ length: Math.max(1, Math.round(countries * change)) }, (_, k) => (day * 7 + k * 13) % countries).sort((a, b) => a - b) : null;
    const unique = sparse && [...new Set(sparse)];
    const sent = message(hour, [...subs].map(id => ({ id, body: table(countries, day, id, unique) })));
    bytes.push(sent.byteLength);
    const split = splitForLanes(sent, LANES);
    const i0 = performance.now();
    if (split.kind !== 'failure') split.lanes.forEach((part, lane) => { if (part) lanes[lane]!.receive('follow', part); });
    ingest.push(performance.now() - i0);
    const s0 = performance.now();
    await flush();
    serve.push(performance.now() - s0);
    const c0 = performance.now();
    page.publish(hour + HOURS_PER_FRAME - 1, { playing: true, step: HOURS_PER_FRAME, speed: 1440 });
    commit.push(performance.now() - c0);
    for (let widget = 0; widget < WIDGETS; widget++) { const view = page.view(widget); ages.push(view.through === undefined ? Infinity : view.target - view.through); }
  }
  laneMs = quantile(serve, .95);
  const finite = ages.filter(Number.isFinite);
  console.log(`| ${name} | ${countries} | ${change} | ${(quantile(bytes, .5) / 1024).toFixed(0)} KiB | ${quantile(ingest, .5).toFixed(2)} / ${quantile(ingest, .95).toFixed(2)} | ${quantile(serve, .5).toFixed(2)} / ${laneMs.toFixed(2)} | ${quantile(commit, .5).toFixed(2)} / ${quantile(commit, .95).toFixed(2)} | ${quantile(finite, .5)} / ${quantile(finite, .95)} h | ${ages.length - finite.length} |`);
}

console.log('| Page | Countries | Change | Message p50 | Lane ingest ms p50 / p95 | Lane frame ms p50 / p95 | Page commit ms p50 / p95 | Age p50 / p95 | Frames with nothing installed |');
console.log('|---|---|---|---|---|---|---|---|---|');
for (const [name, distinct] of [['copy', false], ['distinct', true]] as const)
  for (const countries of [128, 1500]) for (const change of [1, 0.05]) await workload(name, countries, change, distinct);
