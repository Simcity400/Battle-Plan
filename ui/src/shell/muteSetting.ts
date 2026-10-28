import { useEffect, useRef, useState } from 'react';
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
  // Events carry no sequence: a reply is dropped once an event newer than its request arrived.
  const heard = useRef(0);
  useEffect(() => {
    if (!desktopAvailable) return;
    let live = true;
    const changed = listen<boolean>('map-muted', event => { heard.current += 1; setMuted(event.payload); setError(null); });
    void changed.then(() => {
      const seen = heard.current;
      return invoke<boolean>('map_muted').then(value => { if (live && heard.current === seen) setMuted(value); });
    }).catch(cause => { if (live) setError(errorMessage(cause)); });
    return () => { live = false; void changed.then(stop => stop(), () => {}); };
  }, []);
  const set = (value: boolean) => {
    setError(null);
    const seen = heard.current;
    void invoke<boolean>('set_map_muted', { muted: value }).then(next => { if (heard.current === seen) setMuted(next); }, cause => setError(errorMessage(cause)));
  };
  return { muted, error, set };
}
