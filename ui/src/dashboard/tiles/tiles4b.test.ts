import assert from 'node:assert/strict';
import test from 'node:test';
import type { SeriesTileReply } from '../../generated/dashboard';
import type { Run } from '../../charts/series';
import { playbackDomain } from '../../charts/view.ts';
import { TileCache } from './cache.ts';
import { FIRST_BREAK, HAS_BREAK, PRESENT, RUN, SEAM, TILE_BINS, TILE_GROUP, binHoursOf, decodeTileBins, tilesOf } from './tile.ts';
import type { TileReply } from './tile.ts';
import { TileView, cellBins, viewRuns } from './view.ts';
import type { TileLane, TileViewport } from './view.ts';

const LAST = 105_541;
const settle = () => new Promise<void>(resolve => setImmediate(resolve));
const base64 = (bytes: Uint8Array) => { let binary = ''; for (const byte of bytes) binary += String.fromCharCode(byte); return btoa(binary); };

function binned(tags: readonly string[], binHours: number, index: number, marks: boolean): SeriesTileReply {
  const first = index * TILE_BINS * binHours, last = first + TILE_BINS * binHours - 1;
  const bytes = new Uint8Array(tags.length * TILE_BINS * (1 + 4 + 5));
  const view = new DataView(bytes.buffer);
  bytes.fill(PRESENT | 8, 0, tags.length * TILE_BINS);
  for (let at = 0; at < tags.length * TILE_BINS; at++) view.setInt32(tags.length * TILE_BINS + at * 4, at, true);
  return { branchId: '1', measure: 'politicalPower', decimals: 0, binHours, index, firstHour: first, immutable: last < LAST, filled: Math.min(last, LAST),
    rawValues: true, hourBytes: 1, bins: base64(bytes),
    lines: tags.map((tag, at) => ({ country: tag, missing: [], parts: [{ firstHour: first, lastHour: last, slot: at, countryId: at, segmentFirstHour: 0 }],
      ...(marks ? { marks: { before: String(index * 51), bins: Array.from({ length: 52 }, (_, bin) => [bin * 5, 1] as [number, number]).filter(([bin]) => bin < TILE_BINS) } } : {}) })) };
}

function lane(tags: readonly string[], options: { marks?: boolean; key?: string; reply?: typeof binned } = {}) {
  const asked: { binHours: number; index: number; tags: readonly string[] }[] = [];
  const out: TileLane & { asked: typeof asked } = { key: options.key ?? 'source|1|1|measure', tags, lastHour: LAST, marks: options.marks, asked,
    ask: async (binHours, index, tags) => { asked.push({ binHours, index, tags }); return (options.reply ?? binned)(tags, binHours, index, options.marks === true); } };
  return out;
}

function following(head: number, days: number): TileViewport {
  const [from, end] = playbackDomain('window', [0, LAST], head, days * 24, 24);
  return { from: Math.floor(from), to: Math.floor(Math.min(end, head)), end: Math.floor(end), width: 600 };
}

test('following with spend ticks on asks the series and its marks together, one tile per 256 bins, nothing inside a tile', async () => {
  const cache = new TileCache(), marked = lane(['GER', 'ITA'], { marks: true });
  let renders = 0;
  const view = new TileView(cache, marked, () => { renders++; });
  view.setView(following(8760, 365));
  await settle();
  const initial = marked.asked.length;
  let frames = 0, quiet = 0;
  for (let head = 8784; head <= LAST; head += 24) {
    const before = marked.asked.length;
    view.setView(following(head, 365));
    await settle();
    frames++;
    if (marked.asked.length === before) quiet++;
    else assert.equal(marked.asked.length - before, 1, `one tile at hour ${head}`);
  }
  const span = TILE_BINS * binHoursOf(8760, 600), moving = marked.asked.length - initial;
  assert.ok(moving <= Math.ceil((LAST - 8760) / span));
  assert.equal(quiet + moving, frames);
  assert.ok(renders <= initial + moving);
  const line = view.current!.get('GER')!;
  const clusters = line.marks!.clusters;
  assert.ok(clusters.length > 0 && line.marks!.tiles);
  const binHours = binHoursOf(8760, 600), cell = binHours * cellBins(8760, 600, binHours);
  for (const cluster of clusters) {
    assert.equal(Math.floor(cluster.firstHour / cell), Math.floor(cluster.lastHour / cell), 'within one cell');
    assert.equal(cluster.firstHour % binHours, 0, 'its bins on the absolute grid');
  }
  const firstIndex = clusters[0].firstHour / span | 0;
  assert.equal(clusters[0].firstOrdinal, String(firstIndex * 51));
  assert.equal(clusters[1].firstOrdinal, String(firstIndex * 51 + Number(clusters[0].count)));
  console.log(`following with marks at 1,440 h/s: ${initial} tiles to open, then ${moving} requests for series and marks together`);
});

function hourly(runs: (tag: number) => Run[]) {
  return (tags: readonly string[], binHours: number, index: number): SeriesTileReply => {
    assert.equal(binHours, 1);
    const first = index * TILE_BINS, last = first + TILE_BINS - 1;
    const flags = new Uint8Array(tags.length * TILE_BINS), values: number[] = [];
    tags.forEach((_, tag) => {
      for (const run of runs(tag)) {
        if (run.lastHour < first || run.firstHour > last) continue;
        const start = Math.max(first, run.firstHour);
        flags[tag * TILE_BINS + start - first] = PRESENT | RUN | (run.firstHour < first ? SEAM : run.breakBefore ? FIRST_BREAK : 0);
        for (let hour = start + 1; hour <= Math.min(last, run.lastHour); hour++) flags[tag * TILE_BINS + hour - first] = PRESENT;
        values.push(Math.round(run.value * 100));
      }
    });
    const bytes = new Uint8Array(flags.length + values.length * 4);
    bytes.set(flags);
    values.forEach((value, at) => new DataView(bytes.buffer).setInt32(flags.length + at * 4, value, true));
    return { branchId: '1', measure: 'politicalPower', decimals: 2, binHours: 1, index, firstHour: first, immutable: true, filled: last,
      rawValues: true, hourBytes: 0, bins: base64(bytes),
      lines: tags.map((tag, at) => ({ country: tag, missing: [], parts: [{ firstHour: first, lastHour: last, slot: at, countryId: at, segmentFirstHour: 1000 }] })) };
  };
}

test('an hourly tile holds the hourly runs, joined across tile seams, and its bins are those runs', async () => {
  const source = (tag: number): Run[] => [
    { firstHour: 1000, lastHour: 1100, value: 1.5 + tag, breakBefore: true },
    { firstHour: 1101, lastHour: 1700, value: -2.25, breakBefore: false },
    { firstHour: 1750, lastHour: 1760, value: 7, breakBefore: true },
    { firstHour: 1761, lastHour: 1761, value: 7.01, breakBefore: false },
  ];
  const cache = new TileCache(), runs = lane(['GER', 'ITA'], { reply: hourly(source) as never });
  const view = new TileView(cache, runs, () => {});
  const at = { from: 1050, to: 1765, end: 1765, width: 800 };
  assert.equal(binHoursOf(at.end - at.from + 1, at.width), 1);
  view.setView(at);
  await settle();
  for (const [tag, at_] of [['GER', 0], ['ITA', 1]] as const) {
    const line = view.current!.get(tag)!;
    assert.deepEqual(line.summary.runs, [{ ...source(at_)[0], firstHour: 1024, breakBefore: false }, ...source(at_).slice(1)], `${tag}'s runs`);
    assert.deepEqual(viewRuns(line, 1050, 1765), [{ ...source(at_)[0], firstHour: 1050, breakBefore: false }, ...source(at_).slice(1)]);
    const buckets = line.summary.buckets;
    const offset = (hour: number) => hour - line.summary.from;
    assert.equal(buckets[offset(1100)]!.firstValue, 1.5 + at_);
    assert.equal(buckets[offset(1101)]!.firstValue, -2.25);
    assert.equal(buckets[offset(1101)]!.firstBreak, false);
    assert.equal(buckets[offset(1720)], null, 'an hour no run holds is not recorded, never zero');
    assert.equal(buckets[offset(1750)]!.firstBreak, true);
  }
  assert.deepEqual(runs.asked.map(ask => ask.index), [4, 5, 6, 7], 'the tiles of the view and the next, asked ahead');
});

test('a view of many keys asks its groups once, side by side, and a second chart sharing keys asks nothing', async () => {
  const tags = Array.from({ length: 439 }, (_, at) => `T${at}`);
  const cache = new TileCache();
  const all = lane(tags);
  const view = new TileView(cache, all, () => {});
  const at = { from: 0, to: 8759, end: 8759, width: 600 };
  view.setView(at);
  const tiles = Math.floor(8759 / (TILE_BINS * binHoursOf(8760, 600))) + 1, groups = Math.ceil(tags.length / TILE_GROUP);
  assert.equal(all.asked.length, tiles * groups, 'every group of every tile asked at once, before any lands');
  assert.ok(all.asked.every(ask => ask.tags.length <= TILE_GROUP));
  await settle();
  assert.equal(view.current!.size, tags.length);
  view.setView({ ...at, to: 8700 });
  await settle();
  assert.equal(all.asked.length, tiles * groups, 'asked once');
  const some = lane(['T400', 'T3', 'T77'], { key: all.key });
  const other = new TileView(cache, some, () => {});
  other.setView(at);
  await settle();
  assert.equal(some.asked.length, 0);
  assert.equal(other.current!.size, 3);
});

test('a landing tile past the off-thread size is parsed and decoded by the reply worker, its one buffer handed over, and read on the page without base64, each key from its own bytes', async () => {
  const native = Uint8Array as unknown as { fromBase64?: (text: string) => Uint8Array };
  const { fromBase64 } = native, { atob } = globalThis;
  const global = globalThis as unknown as { self?: unknown; Worker?: unknown };
  const scope = { onmessage: null as null | ((event: { data: unknown }) => void), postMessage: (_message: unknown, _options: { transfer: ArrayBuffer[] }) => {} };
  const sent: ArrayBuffer[] = [], returned: { buffer: ArrayBuffer; bins: unknown }[] = [], decoded: ArrayBuffer[] = [];
  let inWorker = false, workerParses = 0, pageParses = 0;
  class InProcessWorker {
    onmessage: ((event: { data: unknown }) => void) | null = null;
    onerror: unknown = null;
    constructor() {
      scope.postMessage = (message, { transfer }) => {
        const value = (message as { value?: { data?: { bins?: unknown } } }).value;
        returned.push({ buffer: transfer[0]!, bins: value?.data?.bins });
        const landed = structuredClone(message, { transfer });
        queueMicrotask(() => this.onmessage?.({ data: landed }));
      };
    }
    postMessage(message: { bytes: ArrayBuffer }, transfer: ArrayBuffer[]) {
      sent.push(message.bytes);
      const arrived = structuredClone(message, { transfer });
      inWorker = true;
      try { scope.onmessage!({ data: arrived }); } finally { inWorker = false; }
    }
    terminate() {}
  }
  let decodes = 0;
  native.fromBase64 = text => {
    decodes++;
    const out = fromBase64 ? fromBase64(text) : Uint8Array.from(atob(text), letter => letter.charCodeAt(0));
    decoded.push(out.buffer as ArrayBuffer);
    return out;
  };
  globalThis.atob = text => { decodes++; return atob(text); };
  const { parse } = JSON;
  JSON.parse = (...args: Parameters<typeof JSON.parse>) => { if (inWorker) workerParses++; else pageParses++; return parse(...args); };
  global.self = scope;
  global.Worker = InProcessWorker;
  try {
    await import('../replyParser.worker.ts');
    const { parseReplyBytes } = await import('../replyParse.ts');
    for (const keys of [TILE_GROUP, 100]) {
      const tags = Array.from({ length: keys }, (_, at) => `T${at}`), count = keys * TILE_BINS;
      const bytes = new Uint8Array(count * (1 + 16 + 5));
      for (let at = 0; at < keys; at++) {
        for (let bin = 0; bin < TILE_BINS; bin++) {
          const pattern = bin < 8 ? ((at >> bin) & 1) * 15 : (at * 7 + bin) % 16;
          bytes[at * TILE_BINS + bin] = PRESENT | (pattern & 1 ? FIRST_BREAK : 0) | (pattern & 2 ? HAS_BREAK : 0) | (pattern & 4 ? RUN : 0) | (pattern & 8 ? SEAM : 0);
        }
        bytes.fill(at + 1, count + at * TILE_BINS * 16, count + (at + 1) * TILE_BINS * 16);
        bytes.fill(100 + at, count * 17 + at * TILE_BINS * 5, count * 17 + (at + 1) * TILE_BINS * 5);
      }
      const reply: SeriesTileReply = { ...binned(tags, 6, 10, false), hourBytes: 1, bins: base64(bytes) };
      const request = new TextEncoder().encode(JSON.stringify({ product: 'series-tiles', data: reply }));
      assert.ok(request.byteLength > 64 * 1024, `${request.byteLength} bytes go to the worker`);
      decodes = 0;
      workerParses = 0;
      pageParses = 0;
      sent.length = 0;
      returned.length = 0;
      decoded.length = 0;
      const parsed = await parseReplyBytes(request) as { data: TileReply };
      assert.equal(sent.length, 1, 'the reply went to the worker');
      assert.deepEqual([workerParses, pageParses], [1, 0], 'the worker parses the reply and the page parses nothing');
      assert.equal(request.byteLength, 0, 'its bytes were handed to the worker, not copied');
      assert.equal(decodes, 1, 'the worker decodes the base64 once');
      const [{ buffer, bins }] = returned;
      assert.ok(bins instanceof Uint8Array && bins.buffer === buffer && buffer === decoded[0], 'the worker hands over the buffer the decoder made, not a copy');
      assert.equal(buffer.byteLength, 0, 'and keeps no copy of it');
      assert.ok(parsed.data.bins instanceof Uint8Array && parsed.data.bins.byteLength === bytes.length && parsed.data.bins.buffer.byteLength === bytes.length,
        'the page holds the one buffer of the bins');
      const tiles = tilesOf(parsed.data, tags, 6, 10, tag => tag);
      assert.equal(decodes, 1, `the page decodes nothing for a tile of ${keys} keys`);
      const own = (array: ArrayBufferView) => [...new Uint8Array(array.buffer, array.byteOffset, array.byteLength)];
      tiles.forEach((tile, at) => {
        assert.deepEqual(own(tile.flags), [...bytes.subarray(at * TILE_BINS, (at + 1) * TILE_BINS)], `${tile.tag} flags`);
        assert.deepEqual(own(tile.values), [...bytes.subarray(count + at * TILE_BINS * 16, count + (at + 1) * TILE_BINS * 16)], `${tile.tag} values`);
        assert.deepEqual(own(tile.hours), [...bytes.subarray(count * 17 + at * TILE_BINS * 5, count * 17 + (at + 1) * TILE_BINS * 5)], `${tile.tag} hours`);
      });
    }
  } finally {
    if (fromBase64) native.fromBase64 = fromBase64; else delete native.fromBase64;
    globalThis.atob = atob;
    JSON.parse = parse;
    delete global.self;
    delete global.Worker;
  }
});

test('a landing tile is decoded where replies are parsed, its bytes handed over, and reads as the same tiles', () => {
  const tags = ['GER', 'ITA'];
  const data = binned(tags, 6, 10, true);
  const parsed = { sourceId: 's', revision: '1', product: 'series-tiles', data: structuredClone(data) };
  const transfer = decodeTileBins(parsed);
  assert.ok((parsed.data.bins as unknown) instanceof Uint8Array, 'decoded off the frame');
  assert.deepEqual(transfer, [(parsed.data.bins as unknown as Uint8Array).buffer], 'the one buffer, handed over');
  const moved = structuredClone(parsed, { transfer });
  assert.equal(transfer[0].byteLength, 0, 'the worker keeps no copy');
  const key = (tag: string) => tag;
  assert.deepEqual(tilesOf(moved.data as never, tags, 6, 10, key), tilesOf(data, tags, 6, 10, key));
  assert.deepEqual(decodeTileBins({ product: 'series-summary', data: { bins: 'AA==' } }), [], 'other replies untouched');
});

test('an hourly view following the head asks one tile per 256 hours of travel, at 24 h/s and at 1 h/s', async () => {
  const source = (): Run[] => Array.from({ length: 400 }, (_, at) => ({ firstHour: at * 50, lastHour: at * 50 + 49, value: at % 7, breakBefore: at === 0 }));
  for (const [speed, travel] of [[24, 6_000], [1, 1_200]] as const) {
    const cache = new TileCache(), runs = lane(['GER', 'ITA'], { reply: hourly(source) as never });
    let renders = 0;
    const view = new TileView(cache, runs, () => { renders++; });
    const start = 1_000;
    view.setView(following(start, 7));
    await settle();
    assert.equal(binHoursOf(168, 600), 1, 'a week at 600 pixels is hourly');
    const initial = runs.asked.length;
    let frames = 0, quiet = 0;
    for (let frame = 1; frame <= travel * 60 / speed; frame++) {
      const before = runs.asked.length;
      view.setView(following(start + frame * speed / 60, 7));
      if (frame % 60 === 0) await settle();
      frames++;
      if (runs.asked.length === before) quiet++;
    }
    await settle();
    const moving = runs.asked.length - initial;
    assert.ok(moving <= Math.ceil(travel / TILE_BINS) && moving >= Math.floor(travel / TILE_BINS) - 1, `${moving} tiles over ${travel} hours`);
    assert.equal(quiet + moving, frames);
    assert.ok(renders <= initial + moving);
    console.log(`an hourly view following at ${speed} h/s: ${initial} tiles to open, then ${moving} requests over ${travel} hours, ${renders} renders`);
  }
});
