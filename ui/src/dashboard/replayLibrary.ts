import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, ReplayLibrary } from './client';
import { errorMessage } from '../shell/errors';

export function useReplayLibrary(client: DashboardClient, shellReady: boolean) {
  const [library, setLibrary] = useState<ReplayLibrary>({ folder: null, files: [] });
  const [loading, setLoading] = useState(!!client.replayLibrary);
  const [error, setError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const saving = useRef(false);
  const refresh = useCallback(async (): Promise<boolean> => {
    if (saving.current || !client.replayLibrary) return false;
    controller.current?.abort();
    const request = new AbortController(); controller.current = request;
    setLoading(true); setError(null);
    try {
      const result = await client.replayLibrary(request.signal);
      if (request.signal.aborted) return false;
      setLibrary(result);
      return true;
    } catch (error) { if (!request.signal.aborted) setError(errorMessage(error)); return false; }
    finally { if (!request.signal.aborted) setLoading(false); }
  }, [client]);
  const setFolder = useCallback(async (folder: string) => {
    if (saving.current) return;
    saving.current = true;
    controller.current?.abort();
    const request = new AbortController(); controller.current = request;
    setLoading(true); setError(null);
    try {
      if (!client.setReplayFolder) throw new Error('Open the desktop app to choose a recordings folder.');
      const result = await client.setReplayFolder(folder);
      if (!request.signal.aborted) { setLibrary(result); return result; }
    } catch (error) { if (!request.signal.aborted) { setError(errorMessage(error)); throw error; } }
    finally { saving.current = false; if (!request.signal.aborted) setLoading(false); }
  }, [client]);
  useEffect(() => () => {
    controller.current?.abort();
    controller.current = null;
  }, [refresh]);
  useEffect(() => { if (shellReady && !controller.current) void refresh(); }, [shellReady, refresh]);
  useEffect(() => {
    const stamp = client.replayLibraryStamp;
    if (!shellReady || !stamp) return;
    let live = true, last: string | null = null, timer: ReturnType<typeof setTimeout>;
    const check = async () => {
      if (typeof document === 'undefined' || !document.hidden) {
        try {
          const now = await stamp();
          if (live && now !== last && await refresh()) last = now;
        } catch {  }
      }
      if (live) timer = setTimeout(check, 2000);
    };
    timer = setTimeout(check, 2000);
    return () => { live = false; clearTimeout(timer); };
  }, [client, refresh, shellReady]);
  return useMemo(() => ({ ...library, loading, error, refresh, setFolder }), [library, loading, error, refresh, setFolder]);
}
export type ReplayLibraryState = ReturnType<typeof useReplayLibrary>;
