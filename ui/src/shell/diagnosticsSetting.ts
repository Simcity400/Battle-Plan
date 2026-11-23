import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { invokeWrite } from './writes';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable, errorMessage } from './api';
import { isDiagnosticsShortcut } from './diagnosticsShortcut';
import { newestSwitch, type MapSwitch } from './mapSwitch';

export function setMapDiagnostics(enabled?: boolean): Promise<MapSwitch> {
  return invokeWrite<MapSwitch>('set_map_diagnostics', { enabled: enabled ?? null });
}

export function useMapDiagnostics() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [asked, setAsked] = useState<boolean | null>(null);
  const [take] = useState(() => newestSwitch(next => { setEnabled(next.value); setWarning(next.warning); setError(null); setAsked(null); }));
  const [reads, setReads] = useState(0);
  const [pending, setPending] = useState(false);
  useEffect(() => {
    if (!desktopAvailable) return;
    let live = true;
    const changed = listen<MapSwitch>('map-diagnostics', event => { take(event.payload); });
    const failed = listen<{ error: string; wanted: boolean }>('map-diagnostics-error', event => { setError(event.payload.error); setAsked(event.payload.wanted); });
    void Promise.all([changed, failed]).then(() => invoke<MapSwitch>('map_diagnostics')).then(value => { if (live) take(value); }, cause => { if (live) setError(errorMessage(cause)); })
      .finally(() => { if (live) setPending(false); });
    return () => { live = false; void changed.then(stop => stop(), () => {}); void failed.then(stop => stop(), () => {}); };
  }, [take, reads]);
  const set = (value: boolean) => {
    setError(null); setAsked(value); setPending(true);
    void setMapDiagnostics(value).then(take, cause => setError(errorMessage(cause))).finally(() => setPending(false));
  };
  const reread = () => { setPending(true); setReads(count => count + 1); };
  return { enabled, error, warning, pending, set, retry: () => { if (asked === null) reread(); else set(asked); } };
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
