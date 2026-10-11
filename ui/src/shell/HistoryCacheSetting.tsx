import { useEffect, useRef, useState } from 'react';
import type { DashboardClient, HistoryCacheReply } from '../dashboard/client';
import { Icon } from '../components/Icons';
import { errorMessage } from './api';

/** Whole megabytes rounded up, or gigabytes to one decimal from 1 GB. */
function size(bytes: number): string {
  const megabytes = Math.ceil(bytes / 2 ** 20);
  return megabytes < 1024 ? `${megabytes} MB` : `${+(megabytes / 1024).toFixed(1)} GB`;
}

/** The widget history the data service keeps on disk: its size, read when shown, and clearing it. */
export default function HistoryCacheSetting({ client, contentReady }: { client: DashboardClient; contentReady: boolean }) {
  const [cache, setCache] = useState<HistoryCacheReply | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pending = useRef<AbortController | null>(null);
  const run = (clear: boolean) => {
    pending.current?.abort();
    const controller = pending.current = new AbortController();
    setBusy(true); setError(null);
    void client.historyCache!(clear, controller.signal)
      .then(setCache, cause => { if (!controller.signal.aborted) setError(errorMessage(cause)); })
      .finally(() => { if (!controller.signal.aborted) setBusy(false); });
  };
  useEffect(() => {
    if (!contentReady) return;
    run(false);
    return () => pending.current?.abort();
  }, [client, contentReady]);
  const status = !contentReady ? ' Available once the game folders are chosen.' : cache ? ` Using ${size(cache.bytes)} of ${size(cache.limit)}.` : busy ? ' Measuring…' : '';
  return <>
    <div className="settings-row"><span className="settings-row-text"><strong>Saved history</strong>
      <small>Widgets keep their history here so campaigns you reopen load instantly.{status}</small></span>
      <button type="button" className="button small" disabled={busy || !contentReady || !cache?.bytes} onClick={() => run(true)}><Icon name="trash" />Clear</button></div>
    {error && <div className="settings-group-body"><p className="map-menu-error" role="alert">{error}</p></div>}
  </>;
}
