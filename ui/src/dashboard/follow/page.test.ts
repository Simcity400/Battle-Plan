import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Page, SAME, type Bitmap, type Entry, type FrameRequest, type PageMessage, type PageSurface } from './page.ts';

interface Fake extends Bitmap { content: string; closed: boolean; fail?: boolean }
interface FakeSurface extends PageSurface<Fake> { content: string | null }
const surface = (): FakeSurface => ({ content: null, install(b) { if (b.fail) throw new Error('detached'); this.content = b.content; b.closed = true; }, clear() { this.content = null; } });
const bitmap = (content: string): Fake => ({ content, closed: false, width: 1, close() { this.closed = true; } });
let ids = 0;
type Options = Partial<Omit<Entry<Fake>, 'content'>> & { n?: number; content?: string | number };
const entry = (o: Options = {}): Entry<Fake> => {
  const id = o.id ?? ++ids, n = o.n ?? 2, c = String(o.content ?? id);
  const { n: _n, content: _content, ...rest } = o;
  return { widget: 1, binding: 1, id, h: 0, c: 0, lo: -Infinity, hi: Infinity, gen: { seek: 0, reset: 0 }, follows: true, mode: 'picture',
    surfaces: Array.from({ length: n }, (_, k) => bitmap(`${c}/${k}`)), fields: { legend: `L${c}`, a11y: `A${c}` }, ...rest,
    content: Object.fromEntries([...Array.from({ length: n }, (_, k) => [String(k), `${c}/${k}`]), ['legend', `L${c}`], ['a11y', `A${c}`], ['model', `M${c}`]]) };
};
function page({ surfaces = 2, offset = 0, pin = null as number | null, bounds = null as readonly [number, number] | null } = {}) {
  const posted: [number | 'all', PageMessage][] = [];
  const p = new Page<Fake>({ lanes: 2, post: (lane, m) => posted.push([lane, m]) });
  p.mount(1, { route: 9, lane: 0, offset, pin, bounds, surfaces: Array.from({ length: surfaces }, surface) });
  const shown = () => p.widgets.get(1)!.surfaces.map(s => (s as FakeSurface).content);
  let serial = 0;
  const offer = (...entries: Entry<Fake>[]) => p.offer({ lane: 0, serial: ++serial + 10_000, entries });
  const frames = (lane = 0) => posted.filter(([l, m]) => l === lane && m.type === 'frame').map(([, m]) => m as FrameRequest);
  return { p, posted, shown, offer, frames, view: () => p.view(1) };
}

test('whole widget: a presentation missing one crop installs nothing, and the complete picture stays', () => {
  const { p, shown, offer, view } = page();
  offer(entry({ id: 1, content: 'red' })); p.publish(0);
  const half = entry({ id: 2, content: 'blue' });
  const first = half.surfaces[0] as Fake;
  half.surfaces = [first, null];
  offer(half); p.publish(1);
  assert.deepEqual([shown(), view().id, view().fields?.legend], [['red/0', 'red/1'], 1, 'Lred']);
  assert.equal(p.stats.rejected['kept content'], 1, 'a missing crop is a part the page does not show');
  assert.ok(first.closed, 'the transferred crop of the rejected entry is closed');
});

test('whole widget: an omitted part names the content it keeps; omitting content the page does not show is refused', () => {
  const { p, shown, offer, view, frames } = page();
  offer(entry({ id: 1, content: 'red', h: 0, c: 0 })); p.publish(0);
  offer(entry({ id: 2, content: 'blue', binding: 99 })); p.publish(1);
  offer({ ...entry({ id: 3, content: 'blue', h: 1, c: 1 }), surfaces: [SAME, SAME] }); p.publish(2);
  assert.deepEqual([shown(), view().id, view().h], [['red/0', 'red/1'], 1, 0], "red pixels never get the blue picture's date");
  assert.equal(p.stats.rejected['kept content'], 1);
  assert.deepEqual(frames().at(-1)!.refused.map(r => [r.id, r.reason]), [[3, 'kept content']], 'the refusal goes back to the lane at once');
  const red = entry({ id: 4, content: 'red', h: 2, c: 2 });
  red.surfaces = [red.surfaces[0]!, SAME]; red.fields = { legend: 'Lred2', a11y: SAME };
  offer(red); p.publish(2);
  assert.deepEqual([shown(), view().id, view().fields], [['red/0', 'red/1'], 4, { legend: 'Lred2', a11y: 'Ared' }], 'kept parts are exactly the installed ones');
});

test('whole widget: a transfer that fails part-way leaves one named status, never half a picture', () => {
  const { p, shown, offer, view } = page();
  offer(entry({ id: 1, content: 'red' })); p.publish(0);
  const e = entry({ id: 2, content: 'blue' });
  (e.surfaces[1] as Fake).fail = true;
  offer(e); p.publish(1);
  assert.deepEqual([view().mode, view().reason, shown()], ['failed', 'drawing failed', [null, null]]);
});

test('whole widget: a remount (new canvases) shows "Reading hour T" until a picture for its new binding installs', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1 })); p.publish(5);
  p.remount(1, [surface(), surface()]);
  assert.deepEqual([view().mode, view().target], ['reading', 5]);
  offer(entry({ id: 2, binding: 1 })); p.frame();
  assert.equal(view().mode, 'reading', 'the old binding is refused');
  offer(entry({ id: 3, binding: 2 })); p.frame();
  assert.equal(view().id, 3);
});

test('clock: dates come from the actual target: a fact at 96 confirmed through 140, requested for 124, shows through 100 at 100', () => {
  const { p, offer, view } = page();
  p.publish(76, { playing: true, step: 24 });
  offer(entry({ id: 1, h: 96, c: 140, lo: 96, hi: Infinity })); p.publish(100);
  assert.deepEqual([view().h, view().through], [96, 100]);
  p.publish(123); assert.equal(view().through, 123);
  p.publish(147); assert.deepEqual([view().through, view().dim], [140, false], 'playing past c ages it; never dims it');
});

test('clock: a skipped prediction installs once the clock has crossed it, while its window holds, and not after', () => {
  const { p, offer, view } = page();
  p.publish(100, { playing: true, step: 24 });
  offer(entry({ id: 1, h: 110, c: 130, lo: 110, hi: 130 })); p.publish(123);
  assert.equal(view().id, 1);
  offer(entry({ id: 2, h: 110, c: 130, lo: 110, hi: 130, content: 'x' })); p.publish(147);
  assert.equal(view().id, 1, 'past its window: refused');
  assert.equal(p.stats.rejected.superseded, 1);
  offer(entry({ id: 3, h: 140, c: 150, lo: 140, hi: 150 })); p.publish(139);
  assert.equal(view().id, 1, 'never ahead: a picture of 140 is not installed at 139');
  p.publish(140);
  assert.deepEqual([view().id, view().through], [3, 140], 'it waited for its window and installs at 140 without a resend');
});

test('review 3: an entry whose window ended before the actual target is refused even if newer, and the lane hears it', () => {
  const { p, offer, view, frames } = page();
  offer(entry({ id: 1, h: 96, c: 119, lo: 96, hi: 119 })); p.publish(100, { playing: true, step: 16 });
  offer(entry({ id: 2, h: 120, c: 143, lo: 120, hi: 143 })); p.publish(150);
  assert.deepEqual([view().id, view().h], [1, 96]);
  assert.deepEqual(frames().at(-1)!.refused.map(r => [r.id, r.reason]), [[2, 'superseded']]);
});

test('review 3: a paused step to 20 never installs a delayed entry whose window ended at 14; the visible picture stays, dimmed', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 10, c: 18, lo: 10, hi: 18 })); p.publish(18);
  p.publish(18, { playing: false });
  offer(entry({ id: 2, h: 12, c: 14, lo: 12, hi: 14 }));
  p.publish(20);
  assert.deepEqual([view().id, view().through, view().dim], [1, 18, true]);
});

test('review 3: rebinding a widget showing hour 96 to a pin at 20 shows "Reading hour 20", never hour 96', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 96, c: 120 })); p.publish(100);
  p.rebind(1, { pin: 20 });
  assert.deepEqual([view().mode, view().target, view().h], ['reading', 20, undefined]);
  const { p: q, offer: offerQ, view: viewQ } = page();
  offerQ(entry({ id: 1, h: 10, c: 120 })); q.publish(100);
  q.rebind(1, { pin: 20 });
  assert.deepEqual([viewQ().id, viewQ().through, viewQ().dim], [1, 20, true], 'a picture valid at the new pin stays, dimmed until its binding installs');
  q.rebind(1, { subscription: true });
  assert.equal(viewQ().mode, 'reading', 'a new subscription never shows the old one');
});

test('review 3: an empty same-seek reset withdraws a follow picture in that commit; a panel picture stays', () => {
  const { p, offer, view } = page();
  p.mount(2, { route: 9, lane: 0, surfaces: [surface()] });
  offer(entry({ id: 1, h: 0, c: 50 }), entry({ id: 2, widget: 2, n: 1, h: 0, c: 50, follows: false })); p.publish(10);
  p.offer({ lane: 1, serial: 1, resets: [[9, 1]], entries: [] }); p.frame();
  assert.deepEqual([view().mode, p.view(2).id], ['reading', 2]);
  assert.deepEqual([p.widgets.get(1)!.content, (p.widgets.get(1)!.surfaces[0] as FakeSurface).content], [{}, null], 'pixels, fields and content ids go together');
});

test('review 3: after an uncovered rewind a SAME entry naming the vanished base is refused, never read from a missing base', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 50, c: 60, content: 'K' })); p.publish(55);
  p.publish(20, { seek: true });
  assert.equal(view().mode, 'reading');
  const delta = entry({ id: 2, h: 10, c: 30, content: 'K', gen: { seek: 1, reset: 0 } });
  delta.surfaces = [SAME, SAME]; delta.fields = { legend: SAME, a11y: SAME };
  offer(delta);
  assert.doesNotThrow(() => p.frame());
  assert.deepEqual([view().mode, p.stats.rejected['kept content']], ['reading', 1]);
});

test('review 3: an early entry retried at the end of a commit never overwrites a newer same-hour correction', () => {
  const { p, offer, view, frames } = page();
  p.publish(10, { playing: true, step: 10 });
  offer(entry({ id: 5, h: 20, c: 20, lo: 20, hi: 30, content: 'old' })); p.publish(15);
  offer(entry({ id: 6, h: 20, c: 25, lo: 20, hi: 30, content: 'fixed' })); p.publish(20);
  assert.deepEqual([view().id, view().fields?.legend], [6, 'Lfixed']);
  assert.deepEqual(frames().at(-1)!.refused.map(r => [r.id, r.reason]), [[5, 'superseded']]);
});

test('review 3: every offered entry ends acknowledged or refused, including displaced and retired early entries', () => {
  const { p, offer, frames } = page();
  p.publish(10, { playing: true, step: 10 });
  offer(entry({ id: 1, lo: 30, hi: 40 })); p.publish(11);
  offer(entry({ id: 2, lo: 30, hi: 40 })); p.publish(12);
  assert.deepEqual(frames().at(-1)!.refused.map(r => [r.id, r.reason]), [[1, 'displaced']]);
  p.publish(12, { playing: false });
  assert.deepEqual(frames().at(-1)!.refused.map(r => [r.id, r.reason]), [[2, 'retired']], 'an exact request retires what the lane speculated');
});

test('review 3: an install in the final paused frame is acknowledged at once, not queued until the clock moves', () => {
  const { p, offer, frames } = page();
  p.publish(10, { playing: false });
  const before = frames().length;
  offer(entry({ id: 9, h: 10, c: 10 })); p.frame();
  assert.equal(frames().length, before + 1);
  assert.deepEqual(frames().at(-1)!.installed, [{ widget: 1, binding: 1, id: 9 }]);
  p.frame();
  assert.equal(frames().length, before + 1, 'nothing more to tell: no request');
});

test('addendum 4: only entries for the current ranked window install', () => {
  const { p, offer, view } = page();
  p.rebind(1, { rank: 3 });
  offer(entry({ id: 1, binding: 2, rank: 2 })); p.frame();
  assert.deepEqual([view().mode, p.stats.rejected['rank window']], ['reading', 1]);
  offer(entry({ id: 2, binding: 2, rank: 3 })); p.frame();
  assert.equal(view().id, 2);
});

test('outside the recording: a target outside the branch shows the named state', () => {
  const { p, offer, view } = page({ bounds: [10, 100] });
  offer(entry({ id: 1, h: 10, c: 200 })); p.publish(50);
  assert.equal(view().id, 1);
  p.publish(150, { seek: true });
  assert.equal(view().mode, 'outside');
});

test('clock: a rewind redates a provably valid picture in the seek publish, or shows the named reading state', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 10, c: 100 })); p.publish(100);
  p.publish(20, { seek: true, playing: false });
  assert.deepEqual([view().id, view().through, view().dim], [1, 20, false], 'h=10 through=100 rewound to 20 shows through 20');
  p.publish(5, { seek: true });
  assert.deepEqual([view().mode, view().target], ['reading', 5]);
  offer(entry({ id: 2, h: 4, c: 4, gen: { seek: 1, reset: 0 } })); p.frame();
  assert.equal(view().mode, 'reading', 'a presentation of the previous stream is refused');
});

test('clock: a paused forward step that native has not covered keeps the installed picture, dimmed', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 10, c: 20 })); p.publish(20, { playing: false });
  offer(entry({ id: 2, h: 22, c: 22, lo: 22, hi: 22 }));
  p.publish(21);
  assert.deepEqual([view().id, view().through, view().dim], [1, 20, true], 'the actually installed picture, not one pending or selected');
});

test('clock: pause, speed change, seek and rebind each wake a request at the actual target', () => {
  const { p, posted } = page();
  p.publish(100, { playing: true, step: 24 });
  posted.length = 0;
  p.publish(115, { playing: true, step: 24 });
  const lane0 = () => posted.filter(([l, m]) => l === 0 && m.type === 'frame').map(([, m]) => m as FrameRequest);
  assert.deepEqual(lane0().map(m => [m.target, m.exact]), [[139, false]], 'speculative while playing');
  posted.length = 0; p.ahead[0] = 0.4; p.publish(116, { playing: true, step: 24 });
  assert.equal(lane0()[0]!.target, 140, 'a smaller lag estimate never moves the speculative target back');
  posted.length = 0; p.publish(118, { playing: true, step: 23 });
  assert.equal(lane0()[0]!.exact, false, 'frame-time jitter in the step is not a speed change');
  for (const act of [() => p.publish(117, { playing: false }), () => p.publish(117, { playing: true, step: 48, speed: 2880 }), () => p.publish(300, { seek: true }), () => p.rebind(1)]) {
    posted.length = 0; act();
    assert.deepEqual(lane0().map(m => [m.target, m.exact]), [[p.T, true]]);
  }
});

test('reset: once the page has seen a same-seek reset, a pre-reset presentation from another lane is refused', () => {
  const { p, offer, view } = page();
  p.mount(2, { route: 9, lane: 1, surfaces: [surface()] });
  offer(entry({ id: 1, h: 0, c: 50, gen: { seek: 0, reset: 1 } }));
  offer(entry({ id: 2, widget: 2, n: 1, h: 0, c: 50, gen: { seek: 0, reset: 0 } }));
  p.publish(10);
  assert.deepEqual([view().id, p.view(2).mode], [1, 'reading']);
  assert.equal(p.stats.rejected.reset, 1);
});

test('ownership: the frame request acknowledges only what was installed', () => {
  const { p, offer, frames } = page();
  offer(entry({ id: 1, binding: 7 })); offer(entry({ id: 2, lo: 50 }));
  p.publish(10, { playing: true, step: 1 });
  assert.deepEqual(frames().at(-1)!.installed, [], 'id 1 names another binding and id 2 waits for 50');
  offer(entry({ id: 3 })); p.publish(11);
  assert.deepEqual(frames().at(-1)!.installed, [{ widget: 1, binding: 1, id: 3 }]);
});

test('binding: a geometry rebind holds the installed picture dimmed; the old binding is refused; the new one installs', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, c: 100 })); p.publish(10, { playing: true });
  p.rebind(1);
  assert.deepEqual([view().id, view().dim], [1, true]);
  offer(entry({ id: 2, binding: 1 })); offer(entry({ id: 3, binding: 2, c: 100 })); p.publish(11);
  assert.deepEqual([view().id, view().dim], [3, false]);
});

test('inspection: a tooltip shows only when the installed picture answers the current inspect target', () => {
  const { p, offer, view } = page();
  p.inspect(1, 'GER');
  offer(entry({ id: 1, fields: { legend: 'L', a11y: 'A', tooltip: { key: 'GER', text: 'GER 12' } } })); p.publish(1);
  assert.equal(view().tooltip, 'GER 12');
  p.inspect(1, 'SOV');
  assert.equal(view().tooltip, null);
});

test("commands: a rejection fails the stream only while it is the route's latest command; a later stream recovers", () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, c: 100 })); p.publish(5);
  const old = p.command(9), latest = p.command(9);
  assert.equal(p.fail(9, old, 'stale'), false);
  assert.equal(view().id, 1);
  assert.equal(p.fail(9, latest, 'stream closed'), true);
  assert.deepEqual([view().mode, view().reason], ['failed', 'stream closed']);
  offer(entry({ id: 2, c: 100 })); p.frame();
  assert.equal(view().mode, 'failed', 'a picture of the failed stream is refused');
  p.publish(30, { seek: true });
  offer(entry({ id: 3, h: 30, c: 30, gen: { seek: 1, reset: 0 } })); p.frame();
  assert.deepEqual([view().id, view().through], [3, 30]);
});

test('store.test.ts:818 the page shows its own day though the lane selected ahead', () => {
  const { p, offer, view } = page();
  p.publish(0, { playing: true, step: 24 });
  for (const hour of [23, 47, 71]) {
    const day = (hour - 23) / 24;
    offer(entry({ id: 100 + day, h: day * 24, c: day * 24 + 23, lo: day * 24, hi: day * 24 + 23, content: day }));
    offer(entry({ id: 200 + day, h: day * 24 + 24, c: day * 24 + 47, lo: day * 24 + 24, hi: day * 24 + 47, content: day + 1 }));
    p.publish(hour);
    assert.deepEqual([view().h, view().through], [day * 24, hour], `at ${hour}`);
  }
});

test('store.test.ts:829 a reading becomes current at the hour the clock published, whatever the lane reached', () => {
  const { p, offer, view } = page();
  p.publish(0, { playing: true, step: 12 });
  offer(entry({ id: 1, h: 0, c: 11, lo: 0, hi: 11 }), entry({ id: 2, h: 12, c: 40, lo: 12, hi: Infinity }));
  p.publish(1);
  assert.equal(view().h, 0);
  p.publish(12);
  assert.equal(view().h, 12);
});

test('store.test.ts:838 a seek removes what it invalidates before any frame listener runs', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 100, c: 120 })); p.publish(100);
  const seen: string[] = [];
  p.widgets.get(1)!.changed = () => seen.push(view().mode);
  p.publish(20, { seek: true });
  assert.deepEqual(seen, ['reading']);
});

test('store.test.ts:850 and :864 an unchanged structure or coverage crosses as a delta on the installed base, never a resend', () => {
  const { p, offer, view, shown } = page();
  offer(entry({ id: 1, h: 10, c: 10, content: 'S', fields: { legend: 'L', a11y: 'A', model: 'K' } })); p.publish(10, { playing: false });
  offer({ ...entry({ id: 2, h: 10, c: 14, content: 'S' }), surfaces: [SAME, SAME], fields: { legend: SAME, a11y: SAME, model: SAME } }); p.publish(14);
  assert.deepEqual([view().id, view().through, view().dim, view().fields?.model, shown()], [2, 14, false, 'K', ['S/0', 'S/1']]);
});

test('store.test.ts:876 a follow failure is terminal for its stream and named, until a later stream brings data', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, c: 10 })); p.publish(5);
  p.fail(9, p.command(9), 'stream closed');
  offer(entry({ id: 2, h: 6, c: 20 })); p.publish(8);
  assert.deepEqual([view().mode, view().reason], ['failed', 'stream closed']);
  p.publish(30, { seek: true });
  offer(entry({ id: 3, h: 30, c: 30, gen: { seek: 1, reset: 0 } })); p.frame();
  assert.deepEqual([view().id, view().dim], [3, false]);
});

test('store.test.ts:891 pausing keeps the picture dimmed while nothing covers the hour; resuming shows it undimmed at once', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 23, c: 23 })); p.publish(23, { playing: true, step: 24 });
  p.publish(24, { playing: false });
  assert.deepEqual([view().id, view().through, view().dim], [1, 23, true]);
  p.publish(24, { playing: true, step: 24 });
  assert.deepEqual([view().id, view().dim], [1, false]);
});

test('offset and pinned tracks: the target is H + offset or the pinned hour, at install and in every date', () => {
  const off = page({ offset: 100 });
  off.offer(entry({ id: 1, h: 110, c: 130, lo: 110, hi: 130 })); off.p.publish(15);
  assert.deepEqual([off.view().id, off.view().through], [1, 115]);
  const pin = page({ pin: 50 });
  pin.offer(entry({ id: 1, h: 40, c: 60, lo: 40, hi: 60 })); pin.p.publish(500);
  assert.deepEqual([pin.view().id, pin.view().through], [1, 50]);
});
