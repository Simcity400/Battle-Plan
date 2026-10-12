import { useSyncExternalStore } from 'react';
import { normalizeCampaignPath } from './campaigns.ts';
import { object } from './validation.ts';

/** Recording names belong to this device's UI settings, never to files or layouts. */
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

let current: RecordingNames | null = null;
const listeners = new Set<() => void>();

/** Saves and updates every view; a refused save still applies for this session. */
export function saveRecordingNames(names: RecordingNames, storage?: Storage): string | null {
  current = names;
  for (const listener of listeners) listener();
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
