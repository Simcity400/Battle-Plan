import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable, errorMessage } from './api';

/**
 * The maps' mute choice. The desktop host saves it, sends it to the native maps and
 * tells every window through `map-muted`. Volumes themselves follow the map settings.
 */
export function useMapMuted() {
  const [muted, setMuted] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!desktopAvailable) return;
    let live = true;
    void invoke<boolean>('map_muted').then(value => { if (live) setMuted(value); }, cause => { if (live) setError(errorMessage(cause)); });
    const changed = listen<boolean>('map-muted', event => { setMuted(event.payload); setError(null); });
    return () => { live = false; void changed.then(stop => stop()); };
  }, []);
  const set = (value: boolean) => {
    setError(null);
    void invoke<boolean>('set_map_muted', { muted: value }).then(setMuted, cause => setError(errorMessage(cause)));
  };
  return { muted, error, set };
}
