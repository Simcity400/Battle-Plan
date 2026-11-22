import type { ToWorker, FromWorker } from '../draw/kit';
import type { DashboardClient } from './client';
import type { PlaybackController } from './playback';
import { APPEARANCE_EVENT } from '../shell/appearance/events.ts';
import { perfLog } from '../shell/perf.ts';
import { pageClock } from './widgetClock.ts';
import { DrawLanes } from './drawLanes.ts';
import type { SurfaceNotice } from './drawLanes.ts';

const listeners = new Set<(message: FromWorker) => void>();
let drawing: DrawLanes | undefined;
let displayRate = 60;
const laneCount = () => Math.max(1, Math.min(3, Number((window as { __drawLanes?: number }).__drawLanes) || Math.min(2, (navigator.hardwareConcurrency || 2) - 1)));
const cap = () => document.documentElement.dataset.playbackRate === 'uncapped' ? 0 : Number(document.documentElement.dataset.playbackRate) || 60;
export const playbackRate = () => cap() || displayRate;
export function onPlaybackRate(changed: () => void) {
  window.addEventListener(APPEARANCE_EVENT, changed);
  window.addEventListener('playback-display-rate', changed);
  return () => { window.removeEventListener(APPEARANCE_EVENT, changed); window.removeEventListener('playback-display-rate', changed); };
}
function lanes(): DrawLanes {
  if (drawing) return drawing;
  const workers = Array.from({ length: laneCount() }, () => new Worker(new URL('../draw/worker.ts', import.meta.url), { type: 'module' }));
  const heard = (data: FromWorker) => {
    if (data.type === 'displayRate') { displayRate = data.hz; window.dispatchEvent(new Event('playback-display-rate')); }
    for (const listener of listeners) listener(data);
  };
  const made = new DrawLanes(workers, heard);
  workers.forEach((worker, lane) => {
    worker.onmessage = ({ data }: MessageEvent<FromWorker>) => made.receive(lane, data);
    worker.onerror = event => { for (const listener of listeners) listener({ type: 'failed', page: -1, reason: event.message }); };
  });
  const options = () => workers.forEach((worker, lane) => worker.postMessage({ type: 'options', cap: cap(), measured: !!perfLog,
    rate: Number((window as { __weakCpu?: number }).__weakCpu) || 1, lane } satisfies ToWorker));
  options(); window.addEventListener(APPEARANCE_EVENT, options);
  if (perfLog) window.addEventListener('weakcpu', options);
  return drawing = made;
}
export function drawPost(message: ToWorker, transfer: Transferable[] = []) {
  lanes().post(message, transfer);
}
export function onDraw(listener: (message: FromWorker) => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function onSurface(id: number, listener: (notice: SurfaceNotice) => void) {
  return lanes().listen(id, listener);
}

let nextSurface = 1;
export const surfaceId = () => nextSurface++;
export function capturePictures(start: boolean) {
  return new Promise<import('../draw/notifications').DrawSample[]>((resolve, reject) => {
    const off = onDraw(message => {
      if (message.type === 'audit') { off(); resolve(message.records); }
      else if (message.type === 'failed') { off(); reject(new Error(message.reason)); }
    });
    drawPost({ type: 'audit', start });
  });
}

export function capturePicturesBatch(limit: number) {
  return new Promise<{ records: import('../draw/notifications').DrawSample[]; done: boolean }>((resolve, reject) => {
    const off = onDraw(message => {
      if (message.type === 'audit') { off(); resolve({ records: message.records, done: message.done }); }
      else if (message.type === 'failed') { off(); reject(new Error(message.reason)); }
    });
    drawPost({ type: 'audit', start: false, limit });
  });
}

const opened = new WeakMap<DashboardClient, Promise<void>>();
export function openFollow(client: DashboardClient): Promise<void> {
  let opening = opened.get(client);
  if (!opening) {
    const draw = lanes(), channels = Array.from({ length: draw.count }, () => new MessageChannel());
    channels.forEach((channel, lane) => draw.postTo(lane, { type: 'port', port: channel.port1 }, [channel.port1]));
    opening = client.openFollow!(channels.map(channel => channel.port2)).catch(error => { opened.delete(client); throw error; });
    opened.set(client, opening);
  }
  return opening;
}

const clocks = new WeakMap<PlaybackController, { users: number; generation: number; release: () => void }>();
export function drawClock(widgetClock: PlaybackController) {
  const clock = pageClock(widgetClock);
  let binding = clocks.get(clock);
  if (!binding) {
    binding = { users: 0, generation: 0, release: () => {} };
    const anchor = () => {
      const s = clock.getSnapshot();
      drawPost({ type: 'clock', anchor: { at: performance.timeOrigin + (s.composedAt ?? performance.now()), position: s.position,
        playing: s.playing, speed: s.hoursPerSecond, lastHour: s.bounds?.lastHour ?? s.hour, seek: s.seekSequence, generation: binding!.generation, forwardSeek: s.forwardSeek } });
    };
    const stopAnchor = clock.onAnchor(() => { binding!.generation++; });
    anchor();
    const release = clock.subscribe(anchor);
    binding.release = () => { release(); stopAnchor(); };
    clocks.set(clock, binding);
  }
  binding.users++;
  return () => { if (--binding.users === 0) { binding.release(); clocks.delete(clock); } };
}

export const clockGeneration = (widgetClock: PlaybackController) => clocks.get(pageClock(widgetClock))?.generation ?? 0;
