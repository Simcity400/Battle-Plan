import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { desktopAvailable, errorMessage } from './api';
import { newestSwitch, type MapSwitch } from './mapSwitch';

export function useMapMuted() {
  const [muted, setMuted] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [take] = useState(() => newestSwitch(next => { setMuted(next.value); setError(next.warning); }));
  const [reads, setReads] = useState(0);
  useEffect(() => {
    if (!desktopAvailable) return;
    let live = true;
    const changed = listen<MapSwitch>('map-muted', event => { take(event.payload); });
    void changed.then(() => invoke<MapSwitch>('map_muted')).then(value => { if (live) take(value); }, cause => { if (live) setError(errorMessage(cause)); });
    return () => { live = false; void changed.then(stop => stop(), () => {}); };
  }, [take, reads]);
  const set = (value: boolean) => {
    setError(null);
    void invoke<MapSwitch>('set_map_muted', { muted: value }).then(take, cause => setError(errorMessage(cause)));
  };
  const retry = () => setReads(count => count + 1);
  return { muted, error, set, retry };
}
