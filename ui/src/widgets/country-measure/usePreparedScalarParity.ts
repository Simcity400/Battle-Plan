import { useEffect, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useParityRows } from '../../dashboard/parityRows';
import type { ParityList } from '../../dashboard/parityRows';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { validateMeasureStates } from './preparedMeasureData';
import { isMember } from '../../dashboard/groups';
import type { HourSpan } from '../../generated/dashboard';

/** Full histories are collected only when the browser parity handshake requests them. */
export function usePreparedScalarParity(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  tags: readonly string[], extent: readonly [number, number], within?: ReadonlyMap<string, readonly HourSpan[]>, config = { family: 'country-stability', measures: ['stability', 'warSupport'], label: 'Stability chart values' }) {
  const root = useRef<HTMLDivElement>(null);
  const { requested, prepare } = useParityPreparation();
  const { label, family, measures } = config;
  const key = JSON.stringify([source?.id, source?.revision, branchId, tags, extent, within && [...within], config]);
  const [answer, setAnswer] = useState<{ key: string; client: DashboardClient; model?: ParityList; error?: string }>();
  useEffect(() => {
    if (!requested || !source || !branchId || !client.preparedQuery) return;
    const controller = new AbortController();
    void (async () => {
      const rows: unknown[] = [];
      for (const tag of tags) for (const measure of measures) for (let first = Math.floor(extent[0]); first <= extent[1]; first += 128) {
        const hours = Array.from({ length: Math.min(128, Math.floor(extent[1]) - first + 1) }, (_, index) => first + index).filter(hour => isMember(within?.get(tag), hour));
        if (!hours.length) continue;
        const data = validateMeasureStates(await preparedSeriesQuery(client, source, 'series-state', { branchId,
          family: { kind: family, measure }, reads: hours.map(hour => ({ tag, hour })) }, controller.signal, true));
        for (const hour of hours) {
          const states = data.states.filter(state => state.hour === hour);
          rows.push({ tag, measure, hour, valueRaw: states[0]?.run?.valueRaw ?? null, readHour: states[0]?.readHour ?? null, ...(family === 'country-experience' ? { gainUnknown: states[0]?.experience?.gainUnknown ?? null, changeUnknown: states[0]?.experience?.changeUnknown ?? null } : {}) });
        }
      }
      if (!controller.signal.aborted) setAnswer({ key, client, model: { kind: 'chart', label, complete: true, total: rows.length, rows } });
    })().catch(error => { if (!controller.signal.aborted) setAnswer({ key, client, error: String(error) }); });
    return () => controller.abort();
  }, [requested, key, client]);
  const current = answer?.key === key && answer.client === client ? answer : undefined;
  useParityRows(root, () => current?.model ?? { kind: 'chart', label, complete: false, total: 0, rows: [] },
    { prepare, ready: () => !!current?.model, error: () => current?.error ?? null });
  return root;
}
