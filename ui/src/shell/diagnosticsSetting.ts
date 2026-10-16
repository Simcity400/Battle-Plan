import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable, errorMessage } from './api';
import { isDiagnosticsShortcut } from './diagnosticsShortcut';

/**
 * Shows or hides the maps' technical stats and data limitations; `undefined` flips the
 * current choice. The desktop host saves it and tells every window through `map-diagnostics`.
 */
export function setMapDiagnostics(enabled?: boolean): Promise<boolean> {
  return invoke<boolean>('set_map_diagnostics', { enabled: enabled ?? null });
}

/** The maps' diagnostics choice, kept current when the shortcut or another control changes it. */
export function useMapDiagnostics() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!desktopAvailable) return;
    let live = true;
    void invoke<boolean>('map_diagnostics').then(value => { if (live) setEnabled(value); }, cause => { if (live) setError(errorMessage(cause)); });
    const changed = listen<boolean>('map-diagnostics', event => { setEnabled(event.payload); setError(null); });
    const failed = listen<string>('map-diagnostics-error', event => setError(event.payload));
    return () => { live = false; void changed.then(stop => stop()); void failed.then(stop => stop()); };
  }, []);
  const set = (value: boolean) => { setError(null); void setMapDiagnostics(value).then(setEnabled, cause => setError(errorMessage(cause))); };
  return { enabled, error, set };
}

/** Ctrl+F3 anywhere in the app toggles map diagnostics, as it does in the native map. */
export function useDiagnosticsShortcut() {
  useEffect(() => {
    if (!desktopAvailable) return;
    const key = (event: KeyboardEvent) => {
      if (!isDiagnosticsShortcut(event)) return;
      event.preventDefault();
      // A failure reaches Settings through the host's `map-diagnostics-error` event.
      void setMapDiagnostics().catch(() => undefined);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
}
