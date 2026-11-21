import { useEffect, useState } from 'react';
import { errorMessage } from '../shell/errors';
import * as api from './api';
import type { UpdateState } from './api';

export function useUpdateState(): { state: UpdateState | null; error: string | null } {
  const [state, setState] = useState<UpdateState | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!api.updatesAvailable) return;
    let active = true;
    const listening = api.onUpdateState(next => { if (active) { setState(next); setError(null); } });
    void listening.then(() => api.updateState()).then(
      next => { if (active) setState(current => current ?? next); },
      cause => { if (active) setError(errorMessage(cause)); });
    return () => { active = false; void listening.then(stop => stop(), () => {}); };
  }, []);
  return { state, error };
}
