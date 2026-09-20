import { useCallback, useEffect, useRef, useState } from 'react';
import type { DashboardClient, ReplayLibrary } from './client';

export function useReplayLibrary(client: DashboardClient) {
  const [library, setLibrary] = useState<ReplayLibrary>({ folder: null, files: [] });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const saving = useRef(false);
  const refresh = useCallback(async () => {
    if (saving.current) return;
    controller.current?.abort();
    const request = new AbortController(); controller.current = request;
    setLoading(true); setError(null);
    try {
      if (!client.replayLibrary) throw new Error('Replay folders are unavailable.');
      const result = await client.replayLibrary(request.signal);
      if (!request.signal.aborted) setLibrary(result);
    } catch (error) { if (!request.signal.aborted) setError(String(error)); }
    finally { if (!request.signal.aborted) setLoading(false); }
  }, [client]);
  const setFolder = useCallback(async (folder: string) => {
    if (saving.current) return;
    saving.current = true;
    controller.current?.abort();
    const request = new AbortController(); controller.current = request;
    setLoading(true); setError(null);
    try {
      if (!client.setReplayFolder) throw new Error('Replay folders are unavailable.');
      const result = await client.setReplayFolder(folder);
      if (!request.signal.aborted) { setLibrary(result); return result; }
    } catch (error) { if (!request.signal.aborted) { setError(String(error)); throw error; } }
    finally { saving.current = false; if (!request.signal.aborted) setLoading(false); }
  }, [client]);
  useEffect(() => { void refresh(); return () => controller.current?.abort(); }, [refresh]);
  return { ...library, loading, error, refresh, setFolder };
}
export type ReplayLibraryState = ReturnType<typeof useReplayLibrary>;
