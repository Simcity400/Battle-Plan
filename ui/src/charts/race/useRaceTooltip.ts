import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { NameAt } from '../../dashboard/countryFacts';

/** Ask for country facts only when the tooltip's hour or countries change. */
export function useRaceTooltip(active: boolean, enabled = true, resetKey?: unknown) {
  const [tip, setTip] = useState<{ hour: number; tags: readonly string[] } | null>(null);
  const tipKey = useRef('');
  const showTip = useCallback((hour: number | null, tags: readonly string[] = []) => {
    const key = hour == null || !enabled ? '' : JSON.stringify([hour, tags]);
    if (key === tipKey.current) return;
    tipKey.current = key;
    setTip(key && hour != null ? { hour, tags } : null);
  }, [enabled]);
  const tipWanted = useMemo<NameAt[]>(() => tip ? tip.tags.map(tag => ({ hour: tip.hour, tag })) : [], [tip]);
  useEffect(() => { if (!active || !enabled) showTip(null); }, [active, enabled, showTip]);
  useEffect(() => { if (resetKey !== undefined) { tipKey.current = ''; setTip(null); } }, [resetKey]);
  return { showTip, tipWanted };
}
