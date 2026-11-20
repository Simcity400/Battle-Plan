import type { ToWorker, FromWorker } from '../draw/kit';
import type { DashboardClient } from './client';
import type { PlaybackController } from './playback';
import { APPEARANCE_EVENT } from '../shell/appearance/events.ts';
import { perfLog } from '../shell/perf.ts';
import { pageClock } from './widgetClock.ts';

const listeners = new Set<(message: FromWorker) => void>();
let drawing: Worker | undefined;
let displayRate = 60;
const cap = () => document.documentElement.dataset.playbackRate === 'uncapped' ? 0 : Number(document.documentElement.dataset.playbackRate) || 60;
export const playbackRate = () => cap() || displayRate;
export function onPlaybackRate(changed: () => void) {
  window.addEventListener(APPEARANCE_EVENT, changed);
  window.addEventListener('playback-display-rate', changed);
  return () => { window.removeEventListener(APPEARANCE_EVENT, changed); window.removeEventListener('playback-display-rate', changed); };
}
export function drawPost(message: ToWorker, transfer: Transferable[] = []) {
  if (!drawing) {
    drawing = new Worker(new URL('../draw/worker.ts', import.meta.url), { type: 'module' });
    drawing.onmessage = ({ data }: MessageEvent<FromWorker>) => {
      if (data.type === 'displayRate') { displayRate = data.hz; window.dispatchEvent(new Event('playback-display-rate')); }
      for (const listener of listeners) listener(data);
    };
    drawing.onerror = event => { for (const listener of listeners) listener({ type: 'failed', page: -1, reason: event.message }); };
    const options = () => drawing!.postMessage({ type: 'options', cap: cap(), measured: !!perfLog, rate: Number((window as { __weakCpu?: number }).__weakCpu) || 1 } satisfies ToWorker);
    options(); window.addEventListener(APPEARANCE_EVENT, options);
    if (perfLog) window.addEventListener('weakcpu', options);
  }
  drawing.postMessage(message, transfer);
}
export function onDraw(listener: (message: FromWorker) => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
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

let nextInspection = 1;
export function inspectFollow(id: number) {
  const request = nextInspection++;
  return new Promise<import('../draw/followed').PageReading | undefined>(resolve => {
    const off = onDraw(message => {
      if (message.type === 'inspection' && message.request === request) { off(); resolve(message.reading); }
      if (message.type === 'failed' || message.type === 'followFailure') { off(); resolve(undefined); }
    });
    drawPost({ type: 'inspect', id, request });
  });
}

const opened = new WeakMap<DashboardClient, Promise<void>>();
export function openFollow(client: DashboardClient): Promise<void> {
  let opening = opened.get(client);
  if (!opening) {
    const channel = new MessageChannel();
    drawPost({ type: 'port', port: channel.port1 }, [channel.port1]);
    opening = client.openFollow!(channel.port2).catch(error => { opened.delete(client); throw error; });
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
