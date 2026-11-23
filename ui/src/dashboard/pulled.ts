import { DisplayRate } from '../draw/frameRate.ts';
import type { DrawSample } from '../draw/notifications';

let displayRate = 60;
const display = new DisplayRate();
const cap = () => document.documentElement.dataset.playbackRate === 'uncapped' ? 0 : Number(document.documentElement.dataset.playbackRate) || 60;
export const playbackRate = () => cap() || displayRate;

export function noteFrame(at: number) {
  const hz = display.sample(at);
  if (hz === undefined) return;
  displayRate = hz;
}

let audit: DrawSample[] | null = null;
let auditDesired = false;
let auditSession = 0;
let draining: DrawSample[] | null = null;
let auditOffset = 0;
const auditListeners = new Set<(active: boolean) => void | Promise<void>>();

function notifyAuditing(active: boolean): Promise<void>[] {
  const pending: Promise<void>[] = [];
  for (const listener of [...auditListeners]) {
    const result = listener(active);
    if (result && typeof (result as Promise<void>).then === 'function') pending.push(result as Promise<void>);
  }
  return pending;
}

export const auditing = () => audit !== null;
export const auditingDesired = () => auditDesired;
export function onAuditingChange(listener: (active: boolean) => void | Promise<void>): () => void {
  auditListeners.add(listener);
  return () => { auditListeners.delete(listener); };
}
export function recordPicture(sample: DrawSample) { audit?.push(sample); }

export async function capturePictures(start: boolean): Promise<DrawSample[]> {
  if (start) {
    const session = ++auditSession;
    audit = null; draining = null; auditOffset = 0;
    auditDesired = true;
    const ready = notifyAuditing(true);
    if (ready.length) await Promise.all(ready);
    if (auditSession !== session || !auditDesired) return [];
    audit = [];
    return [];
  }
  auditSession++;
  const wasDesired = auditDesired;
  const records = audit ?? draining ?? [];
  audit = null; draining = null; auditOffset = 0;
  auditDesired = false;
  if (wasDesired) notifyAuditing(false);
  return records;
}

export function capturePicturesBatch(limit: number): Promise<{ records: DrawSample[]; done: boolean }> {
  if (!draining) {
    auditSession++;
    const wasDesired = auditDesired;
    draining = audit ?? []; audit = null; auditOffset = 0;
    auditDesired = false;
    if (wasDesired) notifyAuditing(false);
  }
  const count = Math.max(1, Math.floor(limit));
  const chunk = draining.slice(auditOffset, auditOffset + count);
  auditOffset += chunk.length;
  const done = auditOffset >= draining.length;
  if (done) { draining = null; auditOffset = 0; }
  return Promise.resolve({ records: chunk, done });
}
