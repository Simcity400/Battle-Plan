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
  red.surfaces = [red.surfaces[0]!, SAME]; red.fields = { legend: 'Lred2', a11y: SAME }; red.content = { ...red.content, legend: 'Lred2' };
  offer(red); p.publish(2);
  assert.deepEqual([shown(), view().id, view().fields], [['red/0', 'red/1'], 4, { legend: 'Lred2', a11y: 'Ared' }], 'kept parts are exactly the installed ones');
});

test('whole widget: a transfer that fails part-way leaves one named status, never half a picture', () => {
  const { p, shown, offer, view } = page();
  offer(entry({ id: 1, content: 'red' })); p.publish(0);
  const e = entry({ id: 2, content: 'blue', c: 10 });
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

test('while playing, an entry whose window ended before the actual target still replaces an older shown picture, and an older one is refused', () => {
  const { p, offer, view, frames } = page();
  offer(entry({ id: 1, h: 96, c: 119, lo: 96, hi: 119 })); p.publish(100, { playing: true, step: 16 });
  offer(entry({ id: 2, h: 120, c: 143, lo: 120, hi: 143 })); p.publish(150);
  assert.deepEqual([view().id, view().h], [2, 120]);
  offer(entry({ id: 3, h: 100, c: 110, lo: 100, hi: 110 })); p.publish(151);
  assert.deepEqual([view().id, frames().at(-1)!.refused.map(r => [r.id, r.reason])], [2, [[3, 'superseded']]]);
  offer(entry({ id: 4, h: 130, c: 143, lo: 130, hi: 143 })); p.publish(152);
  assert.deepEqual([view().id, view().h], [4, 130], 'the same window end with a later start is the newer picture');
});

test('review 3: a paused step to 20 never installs a delayed entry whose window ended at 14; the visible picture stays, dimmed', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 10, c: 18, lo: 10, hi: 18 })); p.publish(18);
  p.publish(18, { playing: false });
  offer(entry({ id: 2, h: 12, c: 14, lo: 12, hi: 14 }));
  p.publish(20);
  assert.deepEqual([view().id, view().through, view().dim], [1, 18, true]);
});

test('review 7 B: while paused, a different picture with the same fact hour that still does not cover the target never replaces the shown one', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 96, c: 119, content: 'X' })); p.publish(150);
  p.publish(150, { playing: false });
  offer(entry({ id: 2, h: 96, c: 140, content: 'Y' })); p.frame();
  assert.deepEqual([view().id, p.stats.rejected.uncovered], [1, 1], 'a later confirmation from the same start is not coverage');
  offer(entry({ id: 3, h: 96, c: 150, content: 'Y' })); p.frame();
  assert.equal(view().id, 3, 'the picture covering the target installs');
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
  assert.deepEqual([viewQ().id, viewQ().dim], [1, true], 'review 7 A: a joined subscription keeps the shown picture, dimmed, until its joined picture installs');
});

test('review 3: an empty same-seek reset withdraws a follow picture in that commit; a panel picture stays', () => {
  const { p, offer, view } = page();
  p.mount(2, { route: 9, lane: 0, surfaces: [surface()] });
  offer(entry({ id: 1, h: 0, c: 50 }), entry({ id: 2, widget: 2, n: 1, h: 0, c: 50, follows: false })); p.publish(10);
  p.offer({ lane: 1, serial: 1, resets: [[9, 1]], entries: [] }); p.frame();
  assert.deepEqual([view().mode, p.view(2).id], ['reading', 2]);
  assert.deepEqual([p.widgets.get(1)!.content, (p.widgets.get(1)!.surfaces[0] as FakeSurface).content], [{}, null], 'pixels, fields and content ids go together');
});

test('the first accepted post-reset entry retires stale followed siblings in that commit', () => {
  const { p, offer, view } = page();
  p.mount(2, { route: 9, lane: 1, surfaces: [surface()] });
  p.mount(3, { route: 9, lane: 1, surfaces: [surface()] });
  offer(entry({ id: 1, h: 0, c: 50 }), entry({ id: 2, widget: 2, n: 1, h: 0, c: 50 }),
    entry({ id: 3, widget: 3, n: 1, h: 0, c: 50, follows: false }));
  p.publish(10, { playing: true });
  assert.equal(p.view(2).id, 2);
  offer(entry({ id: 4, h: 10, c: 50, gen: { seek: 0, reset: 1 } }));
  p.publish(11);
  assert.equal(view().id, 4);
  assert.equal(p.view(2).mode, 'reading');
  assert.equal((p.widgets.get(2)!.surfaces[0] as FakeSurface).content, null);
  assert.equal(p.view(3).id, 3);
  p.offer({ lane: 1, serial: 1, resets: [[9, 1]], entries: [] });
  p.frame();
  assert.equal(view().id, 4);
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
  assert.equal(view().expired, true, 'its coverage has expired');
  p.rebind(1);
  assert.deepEqual([view().dim, view().expired], [true, true], 'a pending rebind does not hide that the coverage expired');
});

test('clock: a picture already dimmed by a pending rebind tells its widget when a paused step then expires it (R3-S4)', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 10, c: 20 })); p.publish(20, { playing: false });
  p.rebind(1);
  assert.deepEqual([view().dim, view().expired], [true, false], 'dimmed by the rebind, still covered');
  let told = 0;
  p.widgets.get(1)!.changed = () => { told++; };
  p.publish(21);
  assert.deepEqual([view().id, view().dim, view().expired], [1, true, true]);
  assert.equal(told, 1, 'the expiry reaches the host, whose input gate must refuse the old picture');
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
  assert.deepEqual([view().id, view().dim, view().expired], [1, true, false], 'a pending rebind alone dims without expiring');
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

test('the page shows its own day though the lane selected ahead', () => {
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

test('a reading becomes current at the hour the clock published, whatever the lane reached', () => {
  const { p, offer, view } = page();
  p.publish(0, { playing: true, step: 12 });
  offer(entry({ id: 1, h: 0, c: 11, lo: 0, hi: 11 }), entry({ id: 2, h: 12, c: 40, lo: 12, hi: Infinity }));
  p.publish(1);
  assert.equal(view().h, 0);
  p.publish(12);
  assert.equal(view().h, 12);
});

test('a seek removes what it invalidates before any frame listener runs', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 100, c: 120 })); p.publish(100);
  const seen: string[] = [];
  p.widgets.get(1)!.changed = () => seen.push(view().mode);
  p.publish(20, { seek: true });
  assert.deepEqual(seen, ['reading']);
});

test('an unchanged structure or coverage crosses as a delta on the installed base, never a resend', () => {
  const { p, offer, view, shown } = page();
  offer(entry({ id: 1, h: 10, c: 10, content: 'S', fields: { legend: 'L', a11y: 'A', model: 'K' } })); p.publish(10, { playing: false });
  offer({ ...entry({ id: 2, h: 10, c: 14, content: 'S' }), surfaces: [SAME, SAME], fields: { legend: SAME, a11y: SAME, model: SAME } }); p.publish(14);
  assert.deepEqual([view().id, view().through, view().dim, view().fields?.model, shown()], [2, 14, false, 'K', ['S/0', 'S/1']]);
});

test('a follow failure is terminal for its stream and named, until a later stream brings data', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, c: 10 })); p.publish(5);
  p.fail(9, p.command(9), 'stream closed');
  offer(entry({ id: 2, h: 6, c: 20 })); p.publish(8);
  assert.deepEqual([view().mode, view().reason], ['failed', 'stream closed']);
  p.publish(30, { seek: true });
  offer(entry({ id: 3, h: 30, c: 30, gen: { seek: 1, reset: 0 } })); p.frame();
  assert.deepEqual([view().id, view().dim], [3, false]);
});

test('pausing keeps the picture dimmed while nothing covers the hour; resuming shows it undimmed at once', () => {
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

test('astra 6: a rewind keeps a picture only where its painted window holds, not merely its confirmation', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 96, c: 140, lo: 120, hi: 143 })); p.publish(124);
  assert.equal(view().id, 1);
  p.publish(100, { seek: true, playing: false });
  assert.equal(view().mode, 'reading', 'the day of 120 painted from the fact of 96 is not the picture of hour 100');
});

test('astra 10: a named failure is never overwritten by a presentation queued before it; a new binding installs again', () => {
  const { p, offer, view, frames } = page();
  offer(entry({ id: 1, h: 0, c: 50 }));
  p.name(1, 'failed', 'The registration was refused');
  p.publish(10);
  assert.deepEqual([view().mode, view().reason], ['failed', 'The registration was refused']);
  assert.deepEqual(frames().at(-1)!.refused.map(r => [r.id, r.reason]), [[1, 'failed']]);
  p.rebind(1, { subscription: true });
  offer(entry({ id: 2, binding: 2, h: 0, c: 50 })); p.frame();
  assert.equal(view().id, 2);
});

test('astra 14: a clock step tells only widgets whose face changed; a later hour of the same picture is a date', () => {
  let told = 0;
  const p = new Page<Fake>({ lanes: 1 });
  p.mount(1, { route: 9, lane: 0, surfaces: [], changed: () => told++ });
  p.publish(9, { playing: true });
  p.offer({ lane: 0, serial: 1, entries: [entry({ id: 1, n: 0, h: 0, c: 100 })] }); p.publish(10);
  const installed = told;
  for (let hour = 11; hour < 20; hour++) p.publish(hour);
  assert.equal(told, installed, 'nothing about the widget but its date changed');
  p.publish(150, { playing: false, seek: true });
  assert.equal(told, installed + 1, 'the rewind past its coverage changed what it shows');
});

test('should-fix b: an entry for a widget that unmounted is refused to its lane, never dropped silently', () => {
  const { p, offer, frames } = page();
  p.unmount(1);
  offer(entry({ id: 7 })); p.frame();
  assert.deepEqual(frames(0).at(-1)!.refused.map(r => [r.id, r.reason]), [[7, 'unmounted']]);
});

test('review 4 finding 5: a forward step past the painted window never redates its pixels: shown through the window end and dimmed', () => {
  const { p, offer, view } = page();
  p.publish(100, { playing: false });
  offer(entry({ id: 1, h: 96, c: 140, lo: 96, hi: 119 })); p.frame();
  assert.deepEqual([view().through, view().dim], [100, false]);
  p.publish(120, { playing: false });
  assert.deepEqual([view().id, view().through, view().dim], [1, 119, true], 'text painted for 96-119 is not shown as 120');
  p.publish(125, { playing: true });
  assert.equal(view().through, 119, 'nor while playing');
});

test('review 4 finding 9: a named failure survives a resize, and a failed bitmap transfer holds against queued entries of the same commit', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1 })); p.publish(0);
  p.name(1, 'failed', 'The pushed answers stopped');
  p.rebind(1, {});
  offer(entry({ id: 2, binding: 2 })); p.frame();
  assert.deepEqual([view().mode, view().reason], ['failed', 'The pushed answers stopped'], 'a new binding of the same failed stream does not revive it');
  const { p: q, offer: offerQ, view: viewQ } = page();
  const broken = entry({ id: 1 });
  (broken.surfaces[0] as Fake).fail = true;
  offerQ(broken, entry({ id: 2 })); q.publish(0);
  assert.deepEqual([viewQ().mode, viewQ().reason], ['failed', 'drawing failed']);
});

test('should-fix: an entry that only moves the confirmation dates the widget without telling its install listeners', () => {
  let told = 0, dated = 0;
  const p = new Page<Fake>({ lanes: 1 });
  p.mount(1, { route: 9, lane: 0, surfaces: [], changed: () => told++, dated: () => dated++ });
  p.offer({ lane: 0, serial: 1, entries: [entry({ id: 1, n: 0, h: 0, c: 10 })] }); p.publish(5);
  const installed = told;
  const later = entry({ id: 2, n: 0, h: 0, c: 20, content: 1 });
  later.fields = { legend: SAME, a11y: SAME };
  p.offer({ lane: 0, serial: 2, entries: [later] }); p.frame();
  assert.deepEqual([told, dated, p.view(1).id], [installed, 1, 2]);
});

test('review 5 finding 4: widgets showing one content hold one page copy of it, dropped once none shows it', () => {
  const p = new Page<Fake>({ lanes: 1 });
  for (const id of [1, 2, 3]) p.mount(id, { route: 9, lane: 0, surfaces: [] });
  const answer = (widget: number, id: number, content: string, body: Uint8Array) =>
    ({ ...entry({ id, n: 0, widget }), fields: { 'answer:5': body }, content: { 'answer:5': content } });
  const first = new Uint8Array([1, 2]), second = new Uint8Array([1, 2]);
  p.offer({ lane: 0, serial: 1, entries: [answer(1, 1, 'answer:7', first)] }); p.publish(0);
  p.offer({ lane: 0, serial: 2, entries: [answer(2, 2, 'answer:7', second)] }); p.frame();
  assert.equal(p.view(2).fields?.['answer:5'], first, 'a second message of the same content is not kept again');
  p.offer({ lane: 0, serial: 3, entries: [answer(1, 3, 'answer:8', new Uint8Array([3]))] }); p.frame();
  assert.equal(p.view(2).fields?.['answer:5'], first, 'the copy stays while a widget still shows it');
  p.unmount(2);
  const third = new Uint8Array([1, 2]);
  p.offer({ lane: 0, serial: 4, entries: [answer(3, 4, 'answer:7', third)] }); p.frame();
  assert.equal(p.view(3).fields?.['answer:5'], third, 'once no widget showed it, the old copy was dropped');
});

test('review 6 A: entries held for a later hour share the page-owned copy from receipt, so ten widgets park one buffer', () => {
  const p = new Page<Fake>({ lanes: 1 });
  for (let id = 1; id <= 10; id++) p.mount(id, { route: 9, lane: 0, surfaces: [] });
  for (let id = 1; id <= 10; id++)
    p.offer({ lane: 0, serial: id, entries: [{ ...entry({ id, n: 0, widget: id, lo: 50, hi: 60 }), fields: { 'answer:5': new Uint8Array([1, 2]) }, content: { 'answer:5': 'answer:7' } }] });
  p.publish(0);
  const parked = new Set([...p.widgets.values()].map(widget => widget.early?.fields['answer:5']));
  assert.deepEqual([parked.size, [...parked][0] instanceof Uint8Array], [1, true], 'one buffer for every parked copy');
});

test('review 6 B: pausing tells the widget it dims, and a paused step keeps the shown picture until a replacement covers the target', () => {
  let told = 0;
  const p = new Page<Fake>({ lanes: 1 });
  p.mount(1, { route: 9, lane: 0, surfaces: [], changed: () => told++ });
  p.publish(19, { playing: true, step: 1 });
  p.offer({ lane: 0, serial: 1, entries: [entry({ id: 1, n: 0, h: 10, c: 10, lo: 10, hi: Infinity })] }); p.publish(20, { playing: true, step: 1 });
  const installed = told;
  p.publish(20, { playing: false });
  assert.deepEqual([p.view(1).dim, told > installed], [true, true], 'the dim change is published when playback pauses');
  p.publish(21, { playing: false });
  p.offer({ lane: 0, serial: 2, entries: [entry({ id: 2, n: 0, h: 15, c: 15, lo: 15, hi: Infinity })] }); p.frame();
  assert.equal(p.view(1).h, 10, 'a picture confirmed only through 15 does not replace the shown one at 21');
  p.offer({ lane: 0, serial: 3, entries: [entry({ id: 3, n: 0, h: 21, c: 21, lo: 21, hi: Infinity })] }); p.frame();
  assert.deepEqual([p.view(1).h, p.view(1).dim], [21, false], 'a picture that covers the target installs');
});

test('review 7 B: a numeric-only install keeps the structure React reads; a structural one changes it', () => {
  const p = new Page<Fake>({ lanes: 1 });
  p.mount(1, { route: 9, lane: 0, surfaces: [] });
  const install = (id: number, content: Record<string, string>) => ({ ...entry({ id, n: 0, c: 10 }), fields: Object.fromEntries(Object.keys(content).map(key => [key, key])), content });
  p.offer({ lane: 0, serial: 1, entries: [install(1, { 'model:3': 'structure:A', 'live:3': 'l1', hour: '1' })] }); p.publish(5);
  const first = p.view(1).structure;
  p.offer({ lane: 0, serial: 2, entries: [install(2, { 'model:3': 'structure:A', 'live:3': 'l2', hour: '2' })] }); p.frame();
  assert.equal(p.view(1).structure, first, 'a countdown that only changes live values is painted, not committed');
  p.offer({ lane: 0, serial: 3, entries: [install(3, { 'model:3': 'structure:B', 'live:3': 'l2', hour: '3' })] }); p.frame();
  assert.notEqual(p.view(1).structure, first);
});

test('review 8 2: while paused, a joined picture of a new binding that does not cover the target never replaces the shown one', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 96, c: 150, content: 'X' })); p.publish(150);
  p.publish(150, { playing: false });
  p.rebind(1, { subscription: true });
  const binding = p.widgets.get(1)!.binding;
  offer(entry({ id: 2, binding, h: 96, c: 140, content: 'Y' })); p.frame();
  assert.deepEqual([view().id, view().dim], [1, true], 'the joined answer confirmed only through 140 is not coverage at 150');
  offer(entry({ id: 3, binding, h: 96, c: 150, content: 'Y' })); p.frame();
  assert.equal(view().id, 3);
});

test('review 9: while paused, a dated refusal that does not cover the target never clears the held picture', () => {
  const { p, offer, view } = page();
  offer(entry({ id: 1, h: 96, c: 150, content: 'X' })); p.publish(150);
  p.publish(150, { playing: false });
  offer(entry({ id: 2, h: 96, c: 140, mode: 'refused', reason: 'Not recorded', content: 'R' })); p.frame();
  assert.deepEqual([view().id, view().mode], [1, 'picture']);
  offer(entry({ id: 3, h: 96, c: 150, mode: 'refused', reason: 'Not recorded', content: 'R' })); p.frame();
  assert.deepEqual([view().id, view().mode], [3, 'refused'], 'the refusal that covers the target installs');
});


test('a later-window speculative entry does not displace an earlier upcoming entry waiting for the clock', () => {
  const { p, offer, view } = page();
  p.publish(100, { playing: true, step: 24 });
  offer(entry({ id: 1, h: 120, c: 143, lo: 120, hi: 143, content: 'day5' }));
  p.publish(110, { playing: true, step: 24 });
  offer(entry({ id: 2, h: 144, c: 167, lo: 144, hi: 167, content: 'day6' }));
  p.publish(115, { playing: true, step: 24 });
  p.publish(124, { playing: true, step: 24 });
  assert.deepEqual([view().id, view().h], [1, 120], 'the earlier speculative window installs when the clock reaches it');
  p.publish(148, { playing: true, step: 24 });
  assert.deepEqual([view().id, view().h], [2, 144], 'the later speculative window follows once the clock reaches it');
});

test('quiet frames visit zero unchanged widgets across 114 mounted widgets, and K changed visits O(K)', () => {
  const p = new Page<Fake>({ lanes: 2 });
  for (let id = 1; id <= 114; id++) p.mount(id, { route: 9, lane: id % 2, surfaces: [] });
  p.publish(0, { playing: true, step: 24, speed: 1440 });
  for (let id = 1; id <= 114; id++) {
    p.offer({ lane: id % 2, serial: id, entries: [entry({ id, widget: id, n: 0, h: 0, c: 10_000, lo: 0, hi: 10_000 })] });
  }
  p.publish(24, { playing: true, step: 24, speed: 1440 });
  const beforeQuiet = p.stats.visited;
  for (let step = 2; step <= 20; step++) p.publish(step * 24, { playing: true, step: 24, speed: 1440 });
  assert.equal(p.stats.visited - beforeQuiet, 0, 'quiet playing frames visit zero widgets');
  const beforeK = p.stats.visited;
  for (let id = 1; id <= 3; id++) {
    p.offer({ lane: id % 2, serial: 1000 + id, entries: [entry({ id: 1000 + id, widget: id, n: 0, h: 500, c: 10_000, lo: 500, hi: 10_000 })] });
  }
  p.publish(504, { playing: true, step: 24, speed: 1440 });
  assert.ok(p.stats.visited - beforeK <= 6, `visited ${p.stats.visited - beforeK} widgets for 3 changed out of 114`);
});

test('S2: a stale-generation confirmation queued before a reset never leaks into the post-reset picture', () => {
  const { p, view } = page();
  p.publish(20, { playing: true, step: 1 });
  p.offer({
    lane: 0, serial: 1, resets: [[9, 0]],
    confirmed: [[9, 'followed', 500, 0, 0]],
    entries: [entry({ id: 1, h: 10, c: 500, gen: { seek: 0, reset: 0 }, open: 'followed' })],
  });
  p.offer({
    lane: 0, serial: 2, resets: [[9, 1]],
    confirmed: [[9, 'followed', 20, 0, 1]],
    entries: [entry({ id: 2, h: 20, c: 20, gen: { seek: 0, reset: 1 }, open: 'followed' })],
  });
  p.publish(20, { playing: false });
  assert.equal(view().id, 2);
  assert.equal(p.widgets.get(1)!.installed!.c, 20, 'old-reset confirmation 500 must not extend post-reset picture past 20');
  p.publish(25, { playing: false });
  assert.equal(view().dim, true, 'post-reset picture dims at 25 because it is only confirmed through 20');
});

test('S4: open-ended speculative entries [120, Infinity] and [144, Infinity] both stay parked until their hours', () => {
  const { p, offer, view } = page();
  p.publish(100, { playing: true, step: 24 });
  offer(entry({ id: 1, h: 120, c: 140, lo: 120, hi: Infinity, content: 'day5' }));
  p.publish(110, { playing: true, step: 24 });
  offer(entry({ id: 2, h: 144, c: 160, lo: 144, hi: Infinity, content: 'day6' }));
  p.publish(115, { playing: true, step: 24 });
  p.publish(124, { playing: true, step: 24 });
  assert.deepEqual([view().id, view().h], [1, 120], 'the [120, Infinity] entry must survive [144, Infinity] and install at 124');
  p.publish(148, { playing: true, step: 24 });
  assert.deepEqual([view().id, view().h], [2, 144], 'the [144, Infinity] entry installs once the clock reaches 144');
});

test('S5: an unclosed parked open-followed entry keeps advancing with shared confirmations while a closed one freezes', () => {
  const { p, view } = page();
  p.publish(100, { playing: true, step: 24 });
  p.offer({
    lane: 0, serial: 1,
    confirmed: [[9, 'followed', 130, 0, 0]],
    entries: [entry({ id: 1, h: 120, c: 130, lo: 120, hi: Infinity, open: 'followed', content: 'open' })],
  });
  p.publish(110, { playing: true, step: 24 });
  p.publish(124, { playing: true, step: 24 });
  assert.equal(view().id, 1);
  p.offer({
    lane: 0, serial: 2,
    confirmed: [[9, 'followed', 150, 0, 0]],
    entries: [],
  });
  p.publish(150, { playing: false });
  assert.deepEqual([p.widgets.get(1)!.installed!.c, view().dim], [150, false], 'installed from early without close still tracks shared confirmation');

  const closed = page();
  closed.p.publish(100, { playing: true, step: 24 });
  closed.p.offer({
    lane: 0, serial: 1,
    confirmed: [[9, 'followed', 130, 0, 0]],
    entries: [entry({ id: 1, h: 120, c: 130, lo: 120, hi: Infinity, open: 'followed', content: 'closed' })],
  });
  closed.p.publish(110, { playing: true, step: 24 });
  closed.p.offer({
    lane: 0, serial: 2,
    closed: [[1, -Infinity, Infinity]],
    confirmed: [[9, 'followed', 160, 0, 0]],
    entries: [],
  });
  closed.p.publish(124, { playing: true, step: 24 });
  closed.p.publish(150, { playing: false });
  assert.deepEqual([closed.p.widgets.get(1)!.installed!.c, closed.view().dim], [130, true], 'parked entry closed before confirmation 160 freezes at 130');
});

test('S12: one bounded widget outside its recording bounds does not force page scans of all bounded widgets on quiet ticks', () => {
  const p = new Page<Fake>({ lanes: 1 });
  p.mount(1, { route: 9, lane: 0, bounds: [0, 50], surfaces: [] });
  for (let id = 2; id <= 50; id++) p.mount(id, { route: 9, lane: 0, bounds: [0, 10_000], surfaces: [] });
  p.publish(0, { playing: true, step: 24, speed: 1440 });
  for (let id = 1; id <= 50; id++) {
    p.offer({ lane: 0, serial: id, entries: [entry({ id, widget: id, n: 0, h: 0, c: 10_000, lo: 0, hi: 10_000 })] });
  }
  p.publish(100, { playing: true, step: 24, speed: 1440 });
  assert.equal(p.view(1).mode, 'outside');
  const before = p.stats.visited;
  for (let h = 124; h <= 400; h += 24) p.publish(h, { playing: true, step: 24, speed: 1440 });
  assert.equal(p.stats.visited - before, 0, `one outside bounded widget caused ${p.stats.visited - before} visits on quiet ticks`);
  p.publish(40, { playing: true, step: 24, speed: 1440 });
  assert.equal(p.view(1).mode, 'picture', 're-entering bounds restores picture');
});

test('accepting a refusal or extra surface closes every transferred bitmap that is not installed', () => {
  const { p, shown, offer, view } = page({ surfaces: 1 });
  const pic = entry({ id: 1, n: 2, c: 10, content: 'wide' });
  const installed = pic.surfaces[0] as Fake;
  const trailing = pic.surfaces[1] as Fake;
  offer(pic);
  p.publish(10);
  assert.deepEqual([shown(), installed.closed, trailing.closed], [['wide/0'], true, true], 'trailing extra bitmap beyond widget surfaces is closed');

  const ref = entry({ id: 2, n: 1, c: 10, mode: 'refused', reason: 'withheld', content: 'ref' });
  const unused = ref.surfaces[0] as Fake;
  offer(ref);
  p.publish(10);
  assert.deepEqual([view().mode, shown(), unused.closed], ['refused', [null], true], 'refusal clears surfaces and closes transferred bitmaps');
});

test('a withdrawal refuses and closes parked early entries even when nothing is installed yet', () => {
  const { p, shown, view } = page({ surfaces: 1 });
  p.publish(5, { playing: true, step: 5 });
  const parked = entry({ id: 1, n: 1, h: 10, c: 20, lo: 10, hi: 20, content: 'future' });
  const bmp = parked.surfaces[0] as Fake;
  p.offer({ lane: 0, serial: 1, entries: [parked] });
  p.publish(6, { playing: true, step: 5 });
  assert.equal(view().mode, 'reading');
  assert.equal(bmp.closed, false);

  p.offer({ lane: 0, serial: 2, withdrawn: [[1, 1]], entries: [] });
  p.publish(10, { playing: false });
  assert.deepEqual([view().mode, shown(), bmp.closed], ['reading', [null], true], 'parked entry promoted to earlyByWidget is refused and closed on withdrawal');
});
