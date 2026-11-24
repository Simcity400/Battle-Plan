import type { FamilyProjection } from './contract.ts';
import { DEFAULT_VIEW, normalizeView, type FamilyViewEntry } from './manifest.ts';
import type { FamilyId } from './plan.ts';

export { DEFAULT_VIEW };

const projections = new Map<string, FamilyProjection<any, any>>();
const manifestByKey = new Map<string, FamilyViewEntry>();
const pendingLoads = new Map<string, Promise<FamilyProjection<any, any> | undefined>>();
const waiters = new Map<string, Set<() => void>>();

function registryKey(family: FamilyId, kind: string, view?: string): string {
  return `${family}\u0000${kind}\u0000${normalizeView(view)}`;
}

function resolveLookupKey(family: FamilyId, kind: string, view?: string): string {
  const exact = registryKey(family, kind, view);
  if (projections.has(exact) || manifestByKey.has(exact)) return exact;
  if (!view || view === DEFAULT_VIEW) {
    const prefix = `${family}\u0000${kind}\u0000`;
    for (const key of projections.keys()) {
      if (key.startsWith(prefix)) return key;
    }
    for (const key of manifestByKey.keys()) {
      if (key.startsWith(prefix)) return key;
    }
  }
  return exact;
}

export function bindFamilyManifest(entries: readonly FamilyViewEntry[]): void {
  for (const entry of entries) {
    manifestByKey.set(registryKey(entry.family, entry.kind, entry.view), entry);
  }
}

export function registerFamilyProjection<TModel, TOptions>(
  projection: FamilyProjection<TModel, TOptions>,
  view = DEFAULT_VIEW,
): FamilyProjection<TModel, TOptions> {
  projections.set(registryKey(projection.family, projection.kind, view), projection as FamilyProjection<any, any>);
  return projection;
}

export function familyProjection<TModel = unknown, TOptions = unknown>(
  family: FamilyId,
  kind: string,
  view?: string,
): FamilyProjection<TModel, TOptions> | undefined {
  return projections.get(resolveLookupKey(family, kind, view)) as FamilyProjection<TModel, TOptions> | undefined;
}

export function registeredFamilyProjections(family?: FamilyId): { readonly family: FamilyId; readonly kind: string; readonly view: string }[] {
  const out: { readonly family: FamilyId; readonly kind: string; readonly view: string }[] = [];
  for (const key of projections.keys()) {
    const [fam, kind, view] = key.split('\u0000');
    if (!fam || !kind || !view) continue;
    if (family !== undefined && fam !== family) continue;
    out.push({ family: fam, kind, view });
  }
  return out;
}

export function ensureFamilyProjection<TModel = unknown, TOptions = unknown>(
  family: FamilyId,
  kind: string,
  view: string | undefined,
  onReady: () => void,
): FamilyProjection<TModel, TOptions> | 'loading' | 'unknown' {
  const key = resolveLookupKey(family, kind, view);
  const existing = projections.get(key);
  if (existing) return existing as FamilyProjection<TModel, TOptions>;
  const entry = manifestByKey.get(key);
  if (!entry) return 'unknown';
  let listeners = waiters.get(key);
  if (!listeners) waiters.set(key, listeners = new Set());
  listeners.add(onReady);
  if (!pendingLoads.has(key)) {
    const load = entry.loadSpec().then(mod => {
      const projection = mod.projection;
      projections.set(key, projection);
      return projection;
    }).catch(() => undefined).finally(() => {
      pendingLoads.delete(key);
      const callbacks = waiters.get(key);
      waiters.delete(key);
      if (callbacks) {
        for (const wake of callbacks) wake();
      }
    });
    pendingLoads.set(key, load);
  }
  return 'loading';
}
