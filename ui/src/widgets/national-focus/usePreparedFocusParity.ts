import { useEffect, useRef, useState } from 'react';
import type { DashboardSource } from '../../dashboard/client';
import { useParityRows } from '../../dashboard/parityRows';
import type { ParityList } from '../../dashboard/parityRows';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { gatherPreparedFocusParity } from './preparedFocusParity';
import type { FocusPreparedClient, PreparedFocusSnapshot } from './preparedFocusData';

/** Called under the immutable parity build-mode guard. */
export function usePreparedFocusParity(client: FocusPreparedClient, source: DashboardSource | undefined,
  branchId: string | undefined, tag: string | undefined, hour: number, moment: boolean, identity: string, enabled: boolean, openedOrdinals: readonly number[], snapshot?: PreparedFocusSnapshot) {
  const root = useRef<HTMLDivElement>(null);
  const { requested, prepare } = useParityPreparation();
  const key = JSON.stringify([identity, hour, moment, enabled, openedOrdinals]);
  const [answer, setAnswer] = useState<{ key: string; client: FocusPreparedClient; model?: ParityList; error?: string } | null>(null);
  useEffect(() => {
    if (!requested || !source || !branchId || !tag || !enabled) return;
    const controller = new AbortController();
    void gatherPreparedFocusParity(client, source, branchId, tag, hour, moment, controller.signal, openedOrdinals, snapshot).then(
      model => { if (!controller.signal.aborted) setAnswer({ key, client, model }); },
      error => { if (!controller.signal.aborted) setAnswer({ key, client, error: String(error) }); });
    return () => controller.abort();
  }, [requested, source, branchId, tag, hour, moment, key, client, enabled, snapshot]);
  const current = answer?.key === key && answer.client === client ? answer : null;
  useParityRows(root, () => current?.model ?? { kind: 'windowed', label: 'Completed focuses in order', complete: false, total: 0, rows: [] },
    { prepare, ready: () => !!current?.model, error: () => current?.error ?? null });
  return root;
}
