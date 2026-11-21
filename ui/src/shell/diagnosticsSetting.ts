import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable, errorMessage } from './api';
import { isDiagnosticsShortcut } from './diagnosticsShortcut';
import { newestSwitch, type MapSwitch } from './mapSwitch';

export function setMapDiagnostics(enabled?: boolean): Promise<MapSwitch> {
  return invoke<MapSwitch>('set_map_diagnostics', { enabled: enabled ?? null });
}

export function useMapDiagnostics() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [take] = useState(() => newestSwitch(setEnabled));
  useEffect(() => {
    if (!desktopAvailable) return;
    let live = true;
    const changed = listen<MapSwitch>('map-diagnostics', event => { if (take(event.payload)) setError(null); });
    const failed = listen<string>('map-diagnostics-error', event => setError(event.payload));
    void Promise.all([changed, failed]).then(() => invoke<MapSwitch>('map_diagnostics')).then(value => { if (live) take(value); }, cause => { if (live) setError(errorMessage(cause)); });
    return () => { live = false; void changed.then(stop => stop(), () => {}); void failed.then(stop => stop(), () => {}); };
  }, [take]);
  const set = (value: boolean) => {
    setError(null);
    void setMapDiagnostics(value).then(take, cause => setError(errorMessage(cause)));
  };
  return { enabled, error, set };
}

export function useDiagnosticsShortcut() {
  useEffect(() => {
    if (!desktopAvailable) return;
    const key = (event: KeyboardEvent) => {
      if (!isDiagnosticsShortcut(event)) return;
      event.preventDefault();
      void setMapDiagnostics().catch(() => undefined);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
}
