import { APPEARANCE_EVENT } from '../shell/appearance/events.ts';
import { DisplayRate } from '../draw/frameRate.ts';
import type { DrawSample } from '../draw/notifications';

let displayRate = 60;
const display = new DisplayRate();
const cap = () => document.documentElement.dataset.playbackRate === 'uncapped' ? 0 : Number(document.documentElement.dataset.playbackRate) || 60;
export const playbackRate = () => cap() || displayRate;
export function onPlaybackRate(changed: () => void) {
  window.addEventListener(APPEARANCE_EVENT, changed);
  window.addEventListener('playback-display-rate', changed);
  return () => { window.removeEventListener(APPEARANCE_EVENT, changed); window.removeEventListener('playback-display-rate', changed); };
}

export function noteFrame(at: number) {
  const hz = display.sample(at);
  if (hz === undefined) return;
  displayRate = hz;
  window.dispatchEvent(new Event('playback-display-rate'));
}

let audit: DrawSample[] | null = null;
let auditOffset = 0;

export const auditing = () => audit !== null;
export function recordPicture(sample: DrawSample) { audit?.push(sample); }

export function capturePictures(start: boolean): Promise<DrawSample[]> {
  if (start) { audit = []; auditOffset = 0; return Promise.resolve([]); }
  const records = audit ?? [];
  audit = null; auditOffset = 0;
  return Promise.resolve(records);
}

export function capturePicturesBatch(limit: number): Promise<{ records: DrawSample[]; done: boolean }> {
  const records = audit ?? [], count = Math.max(1, Math.floor(limit));
  const chunk = records.slice(auditOffset, auditOffset + count);
  auditOffset += chunk.length;
  const done = auditOffset >= records.length;
  if (done) { audit = null; auditOffset = 0; }
  return Promise.resolve({ records: chunk, done });
}
