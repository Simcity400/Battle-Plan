import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, ReplayLibrary } from './client';

export function useReplayLibrary(client: DashboardClient) {
  const [library, setLibrary] = useState<ReplayLibrary>({ folder: null, files: [] });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const saving = useRef(false);
  /** Whether the library now lists what its folder holds. */
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
    } catch (error) { if (!request.signal.aborted) setError(String(error)); return false; }
    finally { if (!request.signal.aborted) setLoading(false); }
  }, [client]);
  const setFolder = useCallback(async (folder: string) => {
    if (saving.current) return;
    saving.current = true;
    controller.current?.abort();
    const request = new AbortController(); controller.current = request;
    setLoading(true); setError(null);
    try {
      if (!client.setReplayFolder) throw new Error('Open the desktop app to choose a replay folder.');
      const result = await client.setReplayFolder(folder);
      if (!request.signal.aborted) { setLibrary(result); return result; }
    } catch (error) { if (!request.signal.aborted) { setError(String(error)); throw error; } }
    finally { saving.current = false; if (!request.signal.aborted) setLoading(false); }
  }, [client]);
  useEffect(() => { void refresh(); return () => controller.current?.abort(); }, [refresh]);
  // A recording saved or renamed by either app shows without reopening the library. One check at a
  // time, the next after it ends; a stamp counts once a scan after it succeeded, so none is missed.
  useEffect(() => {
    const stamp = client.replayLibraryStamp;
    if (!stamp) return;
    let live = true, last: string | null = null, timer: ReturnType<typeof setTimeout>;
    const check = async () => {
      if (typeof document === 'undefined' || !document.hidden) {
        try {
          const now = await stamp();
          if (live && now !== last && await refresh()) last = now;
        } catch { /* The next check tries again. */ }
      }
      if (live) timer = setTimeout(check, 2000);
    };
    timer = setTimeout(check, 2000);
    return () => { live = false; clearTimeout(timer); };
  }, [client, refresh]);
  // One object until something changes, so memoised views skip the shell's other renders.
  return useMemo(() => ({ ...library, loading, error, refresh, setFolder }), [library, loading, error, refresh, setFolder]);
}
export type ReplayLibraryState = ReturnType<typeof useReplayLibrary>;
