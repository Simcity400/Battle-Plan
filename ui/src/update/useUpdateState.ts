import { useEffect, useState } from 'react';
import { errorMessage } from '../shell/errors';
import * as api from './api';
import type { UpdateState } from './api';
import { latestUpdateState } from './logic';

export function useUpdateState(): { state: UpdateState | null; error: string | null; retry(): void } {
  const [state, setState] = useState<UpdateState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!api.updatesAvailable) return;
    let active = true;
    const listening = api.onUpdateState(next => { if (active) { setState(current => latestUpdateState(current, next)); setError(null); } });
    void listening.then(() => api.updateState()).then(
      next => { if (active) { setState(current => latestUpdateState(current, next)); setError(null); } },
      cause => { if (active) setError(errorMessage(cause)); });
    return () => { active = false; void listening.then(stop => stop(), () => {}); };
  }, [attempt]);
  return { state, error, retry: () => { setError(null); setAttempt(count => count + 1); } };
}
