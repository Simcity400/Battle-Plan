import { useSyncExternalStore } from 'react';
import { normalizeCampaignPath } from './campaigns.ts';
import { object } from './validation.ts';

/** Recording names are display labels, never part of files or layouts. The desktop app
 * keeps them in the recorder's shared settings, so Battle Plan Recorder shows the same names;
 * the browser keeps them in this device's UI settings. */
export type RecordingNames = Record<string, string>;
const KEY = 'battle-plan.recording-names.v1';
// The same storage budget as campaign settings; names have no individual length limit.
export const MAX_RECORDING_NAMES_STORAGE = 2_000_000;
type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;

export const recordingFilename = (path: string) => path.replaceAll('\\', '/').split('/').pop()!;
const fileName = (path: string) => recordingFilename(path).replace(/\.(?:battleplan|hoi4replay)$/i, '');

export function recordingDisplayName(path: string, names: RecordingNames): string {
  const key = normalizeCampaignPath(path);
  return Object.hasOwn(names, key) ? names[key] : fileName(path);
}

export function recordingNameTooltip(path: string, names: RecordingNames): string {
  const name = recordingDisplayName(path, names), filename = recordingFilename(path);
  return name === filename ? filename : `${name} · ${filename}`;
}

export function recordingNameMatches(path: string, query: string, names: RecordingNames): boolean {
  const needle = query.trim().toLowerCase();
  return recordingDisplayName(path, names).toLowerCase().includes(needle) || recordingFilename(path).toLowerCase().includes(needle);
}

export function setRecordingName(names: RecordingNames, path: string, draft: string): RecordingNames {
  const next = { ...names }, key = normalizeCampaignPath(path), name = draft.trim();
  if (!name || name === fileName(path) || name === recordingFilename(path)) delete next[key];
  else next[key] = name;
  return next;
}

export function parseRecordingNames(input: unknown): RecordingNames {
  return Object.fromEntries(Object.entries(object(input) ? input : {}).flatMap(([path, draft]) => {
    if (!path || path.includes('\0') || typeof draft !== 'string') return [];
    const name = draft.trim();
    return name && name !== fileName(path) && name !== recordingFilename(path) ? [[normalizeCampaignPath(path), name]] : [];
  }));
}

export function loadRecordingNames(storage?: Storage): RecordingNames {
  try {
    const raw = (storage ?? localStorage).getItem(KEY);
    return raw && raw.length <= MAX_RECORDING_NAMES_STORAGE ? parseRecordingNames(JSON.parse(raw)) : {};
  } catch { return {}; }
}

/** Names to set, by normalized recording path; `null` clears that recording's name. */
export type RecordingNameChanges = Record<string, string | null>;

/** The names `after` sets or clears relative to `before`. */
export function recordingNameChanges(before: RecordingNames, after: RecordingNames): RecordingNameChanges {
  const changes: RecordingNameChanges = {};
  for (const [path, name] of Object.entries(after)) if (before[path] !== name) changes[path] = name;
  for (const path of Object.keys(before)) if (!Object.hasOwn(after, path)) changes[path] = null;
  return changes;
}

let current: RecordingNames | null = null;
let shared: ((changes: RecordingNameChanges) => Promise<void>) | null = null;
const listeners = new Set<() => void>();
const notify = () => { for (const listener of listeners) listener(); };

/** Use the recorder's shared settings for names. Names saved on this device fill in
 * any the shared settings lack, once; they then leave this device's storage. Every
 * later save sends only the names it changes, so a rename made meanwhile in the game
 * Recorder's settings is never overwritten. */
export async function shareRecordingNames(stored: unknown, save: (changes: RecordingNameChanges) => Promise<void>,
  storage?: Pick<globalThis.Storage, 'getItem' | 'setItem' | 'removeItem'>): Promise<void> {
  const local = loadRecordingNames(storage), sharedNames = parseRecordingNames(stored);
  const merged = { ...local, ...sharedNames };
  if (Object.keys(local).length) {
    const missing = recordingNameChanges(sharedNames, merged);
    if (Object.keys(missing).length) await save(missing);
    (storage ?? localStorage).removeItem(KEY);
  }
  shared = save;
  current = merged;
  notify();
}

/** Saves and updates every view; a refused save still applies for this session. With
 * shared settings the save completes later and `report` hears its outcome. */
export function saveRecordingNames(names: RecordingNames, storage?: Storage, report?: (error: string | null) => void): string | null {
  const before = snapshot();
  current = names;
  notify();
  if (shared) {
    const changes = recordingNameChanges(before, names);
    if (!Object.keys(changes).length) { report?.(null); return null; }
    void shared(changes).then(() => report?.(null), () => report?.('Recording names could not be saved; they apply until the app closes.'));
    return null;
  }
  try {
    const raw = JSON.stringify(names);
    if (raw.length > MAX_RECORDING_NAMES_STORAGE) throw new Error('Recording names exceed the settings storage budget.');
    (storage ?? localStorage).setItem(KEY, raw);
    return null;
  } catch { return 'Recording names could not be saved on this device; they apply until the app closes.'; }
}

const snapshot = () => (current ??= loadRecordingNames());
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };

export function useRecordingNames(): RecordingNames {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
