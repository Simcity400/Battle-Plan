export type WritesState = 'saving' | 'failed' | null;

export interface PendingWrite {
  saving(): void;
  saved(): void;
  failed(retry?: () => void): void;
  discard(): void;
}

type Entry = { state: 'saving' } | { state: 'failed'; retry?: () => void };

const writes = new Map<symbol, Entry>();
type Draft = { kind: 'work' } | { kind: 'setting'; section: string };

const drafts = new Map<() => boolean, Draft>();
const listeners = new Set<() => void>();

const changed = () => {
  for (const listener of [...listeners]) listener();
};

export function enterWrite(): PendingWrite {
  const key = Symbol('write');
  writes.set(key, { state: 'saving' });
  changed();
  const set = (entry: Entry | null) => {
    if (!writes.has(key)) return;
    if (entry) writes.set(key, entry); else writes.delete(key);
    changed();
  };
  return { saving: () => set({ state: 'saving' }), saved: () => set(null), failed: retry => set({ state: 'failed', retry }), discard: () => set(null) };
}

export interface WriteSlot {
  enter(): void;
  saved(): void;
  failed(retry?: () => void): void;
  discard(): void;
}

export function writeSlot(): WriteSlot {
  let entry: PendingWrite | null = null;
  return {
    enter: () => { if (entry) entry.saving(); else entry = enterWrite(); },
    saved: () => { entry?.saved(); entry = null; },
    failed: retry => { entry?.failed(retry); },
    discard: () => { entry?.discard(); entry = null; },
  };
}

export function registerDraft(unsaved: () => boolean, section?: string): () => void {
  drafts.set(unsaved, section === undefined ? { kind: 'work' } : { kind: 'setting', section });
  return () => { drafts.delete(unsaved); };
}

export interface SettingDraft<T> {
  value(): T | null;
  set(value: T | null): void;
}

export function settingDraft<T>(section: string): SettingDraft<T> {
  let held: T | null = null;
  registerDraft(() => held !== null, section);
  return {
    value: () => held,
    set: value => {
      const changes = (held === null) !== (value === null);
      held = value;
      if (changes) changed();
    },
  };
}

export function draftSection(): string | null {
  for (const [unsaved, draft] of drafts) if (draft.kind === 'setting' && unsaved()) return draft.section;
  return null;
}

export function writesState(): WritesState {
  const entries = [...writes.values()];
  if (entries.some(entry => entry.state === 'saving')) return 'saving';
  const settingDraft = draftSection() !== null;
  return entries.length || settingDraft ? 'failed' : null;
}

export const unsavedNow = () => writes.size > 0 || [...drafts.keys()].some(unsaved => unsaved());

export function retryFailed(): void {
  for (const entry of [...writes.values()]) if (entry.state === 'failed') entry.retry?.();
}

export function settled(): Promise<boolean> {
  return new Promise(resolve => {
    const check = () => {
      const state = writesState();
      if (state === 'saving') return;
      listeners.delete(check);
      resolve(state === null);
    };
    listeners.add(check);
    check();
  });
}

export function watchWrites(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
