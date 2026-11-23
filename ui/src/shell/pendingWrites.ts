export type WritesState = 'saving' | 'failed' | null;

export interface PendingWrite {
  saving(): void;
  saved(): void;
  failed(): void;
  discard(): void;
}

const writes = new Map<symbol, 'saving' | 'failed'>();
const drafts = new Set<() => boolean>();
const listeners = new Set<() => void>();
let version = 0;

const changed = () => {
  version++;
  for (const listener of listeners) listener();
};

export function enterWrite(): PendingWrite {
  const key = Symbol('write');
  writes.set(key, 'saving');
  changed();
  const set = (state: 'saving' | 'failed' | null) => {
    if (!writes.has(key)) return;
    if (state) writes.set(key, state); else writes.delete(key);
    changed();
  };
  return { saving: () => set('saving'), saved: () => set(null), failed: () => set('failed'), discard: () => set(null) };
}

export interface WriteSlot {
  enter(): void;
  saved(): void;
  failed(): void;
  discard(): void;
}

export function writeSlot(): WriteSlot {
  let entry: PendingWrite | null = null;
  return {
    enter: () => { if (entry) entry.saving(); else entry = enterWrite(); },
    saved: () => { entry?.saved(); entry = null; },
    failed: () => { entry?.failed(); },
    discard: () => { entry?.discard(); entry = null; },
  };
}

export function registerDraft(unsaved: () => boolean): () => void {
  drafts.add(unsaved);
  return () => { drafts.delete(unsaved); };
}

export function writesState(): WritesState {
  const states = [...writes.values()];
  return states.includes('saving') ? 'saving' : states.length ? 'failed' : null;
}

export const unsavedNow = () => writes.size > 0 || [...drafts].some(unsaved => unsaved());

export function settled(): Promise<boolean> {
  return new Promise(resolve => {
    const check = () => {
      const state = writesState();
      if (state === 'saving') return false;
      listeners.delete(check);
      resolve(state === null);
      return true;
    };
    listeners.add(check);
    check();
  });
}

export const writesVersion = () => version;

export function watchWrites(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
