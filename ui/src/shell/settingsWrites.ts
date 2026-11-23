let open = 0;
const listeners = new Set<() => void>();

const changed = () => { for (const listener of listeners) listener(); };

export function settingsWriting(): () => void {
  open++;
  changed();
  let done = false;
  return () => {
    if (done) return;
    done = true;
    open--;
    changed();
  };
}

export const settingsSaving = () => open > 0;

export function watchSettingsWrites(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
