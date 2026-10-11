import { useEffect, useSyncExternalStore } from 'react';
import { installedFonts } from './fonts.ts';

type FontDiscovery = { status: 'unknown' | 'unavailable' } | { status: 'granted'; families: string[] };
let state: FontDiscovery = typeof window !== 'undefined' && typeof window.queryLocalFonts === 'function'
  ? { status: 'unknown' } : { status: 'unavailable' };
let pending: Promise<void> | null = null;
let probed = false;
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
export const readFontDiscovery = () => state;

/** One session-wide request, including denied access, shared across every picker and settings visit. */
export function discoverInstalledFonts(): Promise<void> {
  if (pending) return pending;
  if (state.status !== 'unknown') return Promise.resolve();
  pending = installedFonts().then(families => {
    state = families === null ? { status: 'unavailable' } : { status: 'granted', families };
    pending = null;
    for (const listener of listeners) listener();
  });
  return pending;
}
function probeGrantedPermission() {
  if (probed || state.status !== 'unknown') return;
  probed = true;
  if (!navigator.permissions?.query) return;
  void navigator.permissions.query({ name: 'local-fonts' as PermissionName }).then(permission => {
    if (permission.state === 'granted') void discoverInstalledFonts();
  }, () => {});
}
export function useFontDiscovery(): FontDiscovery {
  useEffect(probeGrantedPermission, []);
  return useSyncExternalStore(subscribe, readFontDiscovery);
}
