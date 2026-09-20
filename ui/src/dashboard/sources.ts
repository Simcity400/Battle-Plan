import { useCallback, useEffect, useRef, useState } from 'react';
import { errorMessage } from '../api';
import type { CountryMetadata } from '../api';
import type { DashboardClient, DashboardSource } from './client';
import { countryData } from './countryData';
import { EffectLifetime } from './effectLifetime';

export interface SourceBinding { source: DashboardSource | null; loading: boolean; error: string | null }

export function useSourceRoles(client: DashboardClient) {
  const [bindings, setBindings] = useState<Record<string, SourceBinding>>({});
  const sources = useRef(new Map<string, DashboardSource>());
  const controllers = useRef(new Map<string, AbortController>());
  const mounted = useRef(false);
  const lifetime = useRef(new EffectLifetime());

  const release = useCallback((source: DashboardSource) => { countryData(client).release(source.id); void client.releaseSource(source.id).catch(() => {}); }, [client]);
  const remove = useCallback((roleId: string) => {
    controllers.current.get(roleId)?.abort();
    controllers.current.delete(roleId);
    const old = sources.current.get(roleId);
    sources.current.delete(roleId);
    if (old) release(old);
    setBindings((previous) => { const next = { ...previous }; delete next[roleId]; return next; });
  }, [release]);

  const open = useCallback(async (roleId: string, path: string, onError?: (message: string) => void): Promise<DashboardSource | null> => {
    controllers.current.get(roleId)?.abort();
    const controller = new AbortController();
    controllers.current.set(roleId, controller);
    const old = sources.current.get(roleId);
    sources.current.delete(roleId);
    if (old) release(old);
    setBindings((previous) => ({ ...previous, [roleId]: { source: null, loading: true, error: null } }));
    try {
      const source = await client.openSource(path, controller.signal);
      if (!mounted.current || controller.signal.aborted || controllers.current.get(roleId) !== controller) { release(source); return null; }
      sources.current.set(roleId, source);
      setBindings((previous) => ({ ...previous, [roleId]: { source, loading: false, error: null } }));
      return source;
    } catch (cause) {
      if (mounted.current && !controller.signal.aborted && controllers.current.get(roleId) === controller) {
        onError?.(errorMessage(cause));
        setBindings((previous) => ({ ...previous, [roleId]: { source: null, loading: false, error: errorMessage(cause) } }));
      }
      return null;
    } finally {
      if (controllers.current.get(roleId) === controller) controllers.current.delete(roleId);
    }
  }, [client, release]);

  useEffect(() => {
    // Hot reload replays effects while preserving these bindings and refs.
    // Their native leases must live until the session actually unmounts.
    const detach = lifetime.current.attach(client, () => {
      mounted.current = false;
      for (const controller of controllers.current.values()) controller.abort();
      controllers.current.clear();
      for (const source of sources.current.values()) release(source);
      sources.current.clear();
    });
    mounted.current = true;
    return detach;
  }, [client, release]);

  const observeCountries = useCallback((sourceId: string, revision: string, countries: CountryMetadata[]) => {
    if (!mounted.current) return;
    const updates = new Map<string, DashboardSource>();
    for (const [roleId, source] of sources.current) {
      if (source.id !== sourceId || source.revision !== revision) continue;
      const identity = (country: CountryMetadata) => `${country.observedBranchId}:${country.id}:${country.tag}`;
      const known = new Set(source.metadata.countries.map(identity));
      const added = countries.filter(country => { const key = identity(country); if (known.has(key)) return false; known.add(key); return true; });
      if (!added.length) continue;
      const next = { ...source, metadata: { ...source.metadata, countries: [...source.metadata.countries, ...added] } };
      sources.current.set(roleId, next); updates.set(roleId, next);
    }
    if (updates.size) setBindings(previous => Object.fromEntries(Object.entries(previous).map(([roleId, binding]) =>
      [roleId, updates.has(roleId) && binding.source?.id === sourceId && binding.source.revision === revision ? { ...binding, source: updates.get(roleId)! } : binding])));
  }, []);

  return { bindings, open, remove, observeCountries };
}
