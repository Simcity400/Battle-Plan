import { stretch } from '../draw/weakCpu.ts';
import {
  createTimeChartWorkerHandler,
  type FontLoadResult,
  type FromTimeChartWorker,
  type ToTimeChartWorker,
  type WorkerFontFace,
} from './timeChartWorkerCore.ts';

const post = (message: FromTimeChartWorker) => self.postMessage(message);
const loadedKeys = new Set<string>();
const yieldChannel = new MessageChannel();
let pendingRun: (() => void) | null = null;
yieldChannel.port1.onmessage = () => {
  const run = pendingRun;
  pendingRun = null;
  run?.();
};

async function loadWorkerFonts(faces: readonly WorkerFontFace[]): Promise<FontLoadResult> {
  const fontSet = (self as unknown as { fonts?: FontFaceSet }).fonts;
  if (!fontSet || typeof FontFace !== 'function') return { added: false, failed: faces.map(f => f.family) };
  let added = false;
  const failed: string[] = [];
  await Promise.all(faces.map(async entry => {
    const key = `${entry.family}|${entry.source}|${entry.descriptors?.weight ?? ''}|${entry.descriptors?.style ?? ''}|${entry.descriptors?.unicodeRange ?? ''}`;
    if (loadedKeys.has(key)) return;
    loadedKeys.add(key);
    try {
      const face = new FontFace(entry.family, entry.source, entry.descriptors);
      await face.load();
      fontSet.add(face);
      added = true;
    } catch {
      loadedKeys.delete(key);
      failed.push(entry.family);
    }
  }));
  return { added, failed };
}

const handler = createTimeChartWorkerHandler({
  schedule: run => {
    pendingRun = run;
    yieldChannel.port2.postMessage(0);
  },
  stretch: (workMs, rate) => { stretch(workMs, rate, performance.now()); },
  loadFonts: loadWorkerFonts,
  onError: reason => { post({ type: 'error', reason }); },
  onPainted: (id, frameId, ms, replaced, at) => {
    post({ type: 'painted', id, frameId, ...(ms !== undefined ? { ms, replaced, at } : {}) });
  },
  onFontsResult: (fontKey, ok, failed) => {
    post({ type: 'fontsResult', fontKey, ok, failed });
  },
});

self.onmessage = ({ data }: MessageEvent<ToTimeChartWorker>) => handler.handle(data);
self.onerror = event => { post({ type: 'error', reason: typeof event === 'string' ? event : (event as ErrorEvent).message || 'Worker error' }); };
self.onmessageerror = () => { post({ type: 'error', reason: 'Time chart worker failed to deserialize a message.' }); };
