import { useEffect, useRef, useState } from 'react';
import type { DashboardClient } from '../dashboard/client';
import type { PreparationPreferences } from './preparation/contract';
import { errorMessage } from './api';

export default function PreparationSetting({ client }: { client: DashboardClient }) {
  const [preferences, setPreferences] = useState<PreparationPreferences | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef<AbortController | null>(null);
  const request = (enabled: boolean | null) => {
    pending.current?.abort();
    const controller = pending.current = new AbortController();
    setBusy(true); setError(null);
    void client.preparationPreferences!(enabled, controller.signal).then(value => {
      if (!controller.signal.aborted) setPreferences(value);
    }, cause => {
      if (!controller.signal.aborted) setError(errorMessage(cause));
    }).finally(() => { if (!controller.signal.aborted) setBusy(false); });
  };
  useEffect(() => {
    request(null);
    return () => pending.current?.abort();
  }, [client]);
  return <>
    <label className="settings-row"><span className="settings-row-text"><strong>Prepare recordings in the background</strong>
      <small>Prepare opened recordings for faster focus, equipment, map and chart views. Turning this off stops automatic preparation and keeps saved data. You can still prepare a recording from its menu.</small></span>
      <input type="checkbox" checked={preferences?.backgroundEnabled ?? true} disabled={busy || !preferences}
        onChange={event => request(event.currentTarget.checked)} aria-label="Prepare recordings in the background" /></label>
    {error && <p className="map-menu-error" role="alert">{error}</p>}
  </>;
}
