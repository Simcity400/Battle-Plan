import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { PlaybackStrip } from './controls.tsx';
import { PlaybackController } from './playback.ts';
import { Pipeline } from './follow/pipeline.ts';
import { followMapPlayback } from './mapPlayback.ts';
import { FrameEngine } from '../engine/frame.ts';
import { HOUR_EPOCH, formatDateTime } from './calendar.ts';
import type { DashboardClient, DashboardSource } from './client';
import type { BoundTrack } from './model';
import type { FromLane, ToLane } from '../draw/laneMessages.ts';
import { PrepareGate } from './prepare.ts';
import { StripPage } from './strip/driver.ts';
import { stripLattice } from './strip/lattice.ts';
import { WidgetStrips } from './strip/store.ts';
import '../styles.css';

const root = createRoot(document.body.appendChild(document.createElement('div')));
const frames = new FrameEngine({ request() {} });
const clock = new PlaybackController(frames);
const source = { id: 'scrub-test', revision: '1', path: 'synthetic.battleplan' } as DashboardSource;
const track = { id: 'track', roleId: 'main', source, branch: { id: '1', firstHour: HOUR_EPOCH, lastHour: HOUR_EPOCH + 100 }, axisOffset: HOUR_EPOCH } as BoundTrack;
const asks: Record<string, unknown>[] = [];
const mapHours: number[] = [];
const scrubWire: Extract<ToLane, { type: 'scrub' }>[] = [];
const ports: MessagePort[] = [];
let pipeline: Pipeline;
let stopMap: () => void = () => {};
let stopStrips = () => {};
let cleared = 0, pauses = 0, initialSeek = 0, captured = false;
let now = 0, nextTimer = -1;
const timers = new Map<number, { at: number; run: () => void }>();
const realTimeout = window.setTimeout.bind(window), realClear = window.clearTimeout.bind(window);
window.setTimeout = ((run: TimerHandler, delay?: number, ...args: unknown[]) => {
  if (delay !== 150 || typeof run !== 'function') return realTimeout(run, delay, ...args);
  const id = nextTimer--;
  timers.set(id, { at: now + delay, run: () => run(...args) });
  return id;
}) as typeof window.setTimeout;
window.clearTimeout = id => { if (!timers.delete(Number(id))) realClear(id); };
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const input = () => document.querySelector<HTMLInputElement>('.header-timeline')!;
const pointer = (type: string, hour: number) => {
  const rect = input().getBoundingClientRect();
  flushSync(() => input().dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: 7, button: 0, buttons: type === 'pointerup' ? 0 : 1,
    clientX: rect.left + 7 + hour / 100 * (rect.width - 14), clientY: rect.top + rect.height / 2 })));
};
const change = (hour: number) => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input(), String(hour));
  flushSync(() => input().dispatchEvent(new Event('input', { bubbles: true })));
};
const state = () => ({ seeks: clock.getSnapshot().seekSequence - initialSeek, requests: asks.length, clears: cleared, pauses,
  position: clock.getSnapshot().position, playing: clock.getSnapshot().playing, scrub: clock.getScrubPosition(),
  handle: Number(input().value), fill: document.querySelector<HTMLElement>('.timeline-fill')!.style.transform,
  knob: document.querySelector<HTMLElement>('.timeline-knob')!.style.transform,
  date: document.querySelector('.date-time-value')!.textContent, dated: input().getAttribute('aria-valuetext'),
  expectedDate: formatDateTime((clock.getScrubPosition() ?? clock.getHour()) + HOUR_EPOCH), mapHours: [...mapHours], captured,
  scrubHours: scrubWire.map(message => message.frame?.hour ?? null) });

clock.onAnchor((previous, next) => { if (previous.playing && !next.playing && previous.seekSequence === next.seekSequence) pauses++; });
const fixture = {
  async mount(playing = false, ready = false) {
    flushSync(() => root.render(null));
    stopStrips(); pipeline?.close(); stopMap();
    for (const port of ports.splice(0)) port.close();
    timers.clear(); clock.endScrub(); clock.setBounds({ firstHour: 0, lastHour: 100 }, true); clock.seek(80);
    if (playing) clock.setPlaying(true);
    const client = { available: true, openFollow: async () => undefined,
      product: async (request: { product: string; params: Record<string, unknown> }) => {
        if (request.product === 'follow-clock') asks.push(request.params);
        return { data: { available: true, route: request.params.route } };
      } } as unknown as DashboardClient;
    pipeline = new Pipeline(clock, client, { lanes: [0, 1].map(() => ({
      onmessage: null as ((event: MessageEvent<FromLane>) => void) | null,
      postMessage(message: ToLane) {
        if (message.type === 'scrub') scrubWire.push(message);
        if (message.type === 'follow' && message.route) queueMicrotask(() => this.onmessage?.({ data: { type: 'route-bound', sub: message.sub, route: message.route!.id, seek: message.route!.seek } } as MessageEvent<FromLane>));
      },
    })), open: () => Promise.resolve(undefined),
      channel: () => { const channel = new MessageChannel(); ports.push(channel.port1, channel.port2); return channel; }, frameRate: () => 60, rate: () => 1 });
    const id = pipeline.acquire('scrub|0', { offset: 0, pin: null });
    pipeline.bind(id, [{ install() {}, clear() { cleared++; } }], { surfaces: [{ width: 1, height: 1, dpr: 1 }] });
    pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state' }, 'followed');
    await flush();
    pipeline.page.offer({ lane: 0, serial: 0, entries: [{ widget: id, id: 1, binding: pipeline.binding(id),
      gen: { seek: pipeline.page.seek, reset: 0 }, follows: true, h: 80, c: 80, lo: 80, hi: 80, mode: 'picture',
      surfaces: [{ width: 1, close() {} } as ImageBitmap], content: {}, fields: {} }] });
    pipeline.page.frame();
    if (pipeline.view(id)?.mode !== 'picture') throw new Error('The initial picture did not install');
    if (ready) {
      const held = new WidgetStrips();
      const gate = new PrepareGate(() => true, { request: run => { queueMicrotask(run); return 0; }, cancel() {} });
      const page = new StripPage(stripLattice(track.branch, HOUR_EPOCH), clock, client, { frames, gate });
      page.register('scrub', { key: 'scrub', supported: true, settled: () => true, scrub: pipeline.scrub,
        keyframe: recorded => ({ asks: [], scene: { recorded, parts: [], allowEmpty: true } }),
        store: async (_build, update) => held.update(update),
      });
      stopStrips = page.start();
      for (let i = 0; i < 100 && page.status !== 'ready'; i++) await flush();
      if (page.status !== 'ready') throw new Error(`The strip did not build: ${page.status}`);
    }
    stopMap = followMapPlayback(clock, track, async (_path, _revision, _branch, hour) => { mapHours.push(hour - HOUR_EPOCH); }, () => {});
    flushSync(() => root.render(<PlaybackStrip clock={clock} tracks={[track]} alignment="calendar" defaultRoleId="main" />));
    input().setPointerCapture = () => { captured = true; };
    input().hasPointerCapture = () => captured;
    input().releasePointerCapture = () => { captured = false; };
    asks.length = 0; mapHours.length = 0; scrubWire.length = 0; cleared = 0; pauses = 0; captured = false;
    initialSeek = clock.getSnapshot().seekSequence;
    return state();
  },
  press(hour = 80) { pointer('pointerdown', hour); return state(); },
  move(hour: number) { pointer('pointermove', hour); change(hour); return state(); },
  frame() { frames.tick(performance.now()); return state(); },
  async release(cancel = false) { pointer(cancel ? 'pointercancel' : 'pointerup', Number(input().value)); await flush(); return state(); },
  async advance(ms: number) {
    now += ms;
    for (const [id, timer] of [...timers]) if (timer.at <= now && timers.delete(id)) flushSync(() => timer.run());
    await flush(); return state();
  },
  async change(hour: number) { change(hour); await flush(); return state(); },
  async unmount() {
    flushSync(() => root.render(null));
    now += 150;
    for (const [id, timer] of [...timers]) if (timer.at <= now && timers.delete(id)) timer.run();
    await flush();
    return { seeks: clock.getSnapshot().seekSequence - initialSeek, scrub: clock.getScrubPosition() };
  },
};
(window as unknown as { timeline: typeof fixture }).timeline = fixture;
