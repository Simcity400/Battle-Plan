import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable, errorMessage } from './api';
import { newestSwitch, type MapSwitch } from './mapSwitch';

export function useMapMuted() {
  const [muted, setMuted] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [take] = useState(() => newestSwitch(setMuted));
  useEffect(() => {
    if (!desktopAvailable) return;
    let live = true;
    const changed = listen<MapSwitch>('map-muted', event => { if (take(event.payload)) setError(null); });
    void changed.then(() => invoke<MapSwitch>('map_muted')).then(value => { if (live) take(value); }, cause => { if (live) setError(errorMessage(cause)); });
    return () => { live = false; void changed.then(stop => stop(), () => {}); };
  }, [take]);
  const set = (value: boolean) => {
    setError(null);
    void invoke<MapSwitch>('set_map_muted', { muted: value }).then(take, cause => setError(errorMessage(cause)));
  };
  return { muted, error, set };
}
