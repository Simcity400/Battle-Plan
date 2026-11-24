import { useEffect, useState } from 'react';
import { normalizeView, type FamilyViewEntry, type FamilyViewModule } from '../manifest.ts';

const loaded = new Map<string, FamilyViewModule>();
const pending = new Map<string, Promise<FamilyViewModule>>();

const entryKey = (entry: FamilyViewEntry) => `${entry.family}\u0000${entry.kind}\u0000${normalizeView(entry.view)}`;

export function loadFamilyViewSpec(entry: FamilyViewEntry): Promise<FamilyViewModule> {
  const key = entryKey(entry);
  const done = loaded.get(key);
  if (done) return Promise.resolve(done);
  let load = pending.get(key);
  if (!load) {
    load = entry.loadSpec().then(spec => {
      loaded.set(key, spec);
      return spec;
    }).finally(() => pending.delete(key));
    pending.set(key, load);
  }
  return load;
}

export function loadedFamilyViewSpec(entry: FamilyViewEntry): FamilyViewModule | undefined {
  return loaded.get(entryKey(entry));
}

export function useFamilyViewSpec(entry: FamilyViewEntry | undefined): { readonly spec: FamilyViewModule | undefined; readonly failed: boolean } {
  const [, setVersion] = useState(0);
  const [failed, setFailed] = useState<string | null>(null);
  const spec = entry ? loadedFamilyViewSpec(entry) : undefined;
  useEffect(() => {
    if (!entry || spec) return;
    let live = true;
    loadFamilyViewSpec(entry).then(
      () => { if (live) setVersion(version => version + 1); },
      () => { if (live) setFailed(entryKey(entry)); },
    );
    return () => { live = false; };
  }, [entry, spec]);
  return { spec, failed: entry !== undefined && failed === entryKey(entry) };
}
