import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useState } from 'react';
import type { FocusEntry, FocusReply } from '../../generated/dashboard';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { validateFocusReply } from '../national-focus/focusData';
import { loadPreparedFocusStaticText, waitForFocusPreparation } from '../national-focus/preparedFocusData';
import type { FocusPreparedClient } from '../national-focus/preparedFocusData';
import { historyMarks } from './markers';
import type { FocusMarks } from './markers';

/** Preparation's dated completions with the focus content's warnings, or null before it has
 * prepared the whole branch. */
export function validateFocusCompletions(value: unknown): { completions: FocusEntry[]; warnings: string[] } | null {
  if (value === null) return null;
  const reply = value as { completions?: unknown; warnings?: unknown };
  if (!reply || !Array.isArray(reply.completions) || !reply.completions.every(entry => entry && Number.isInteger(entry.ordinal) && entry.ordinal >= 0
    && typeof entry.token === 'string' && entry.token !== '' && Number.isInteger(entry.completedByHour))
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('Prepared focus completions are malformed.');
  }
  return reply as { completions: FocusEntry[]; warnings: string[] };
}

/** `tag`'s completed focuses for its markers: preparation's once it holds the whole branch (the
 * dated completions of every order the focus history holds), else the focus history's. */
export function useFocusMarks(client: FocusPreparedClient, track: BoundTrack | undefined, tag: string | undefined, active: boolean) {
  const source = track?.source, branchId = track?.branch.id;
  const content = useContentRevision(client);
  const key = JSON.stringify([source?.id, source?.revision, branchId, tag, content]);
  const [read, setRead] = useState<{ key: string; marks: FocusMarks | null; warnings: string[]; error?: Failure }>();
  const current = read?.key === key ? read : undefined;
  // Settled once preparation's marks or a failure are read; the history answers while it prepares.
  const settled = current !== undefined && (current.marks !== null || current.error !== undefined);
  useEffect(() => {
    if (!active || !client.preparedQuery || !source || !branchId || !tag || settled) return;
    const controller = new AbortController(), signal = controller.signal;
    const query = async () => validateFocusCompletions(await client.preparedQuery!<unknown>(source, 'focus-completions', { branchId, tag }, { signal }));
    void (async () => {
      let reply = await query();
      if (reply === null) {
        if (!signal.aborted) setRead({ key, marks: null, warnings: [] });
        // Read again once the focus family has prepared; preparation that stops leaves the history.
        try { await waitForFocusPreparation(client, source, signal); } catch { return; }
        reply = await query();
        if (reply === null) return;
      }
      const tokens = [...new Set(reply.completions.map(entry => entry.token))].sort();
      const { text, warnings } = await loadPreparedFocusStaticText(client, source, 'national', tokens, signal);
      if (!signal.aborted) setRead({ key, marks: { completed: reply.completions, staticText: text }, warnings: [...new Set([...reply.warnings, ...warnings])] });
    })().catch(error => { if (!signal.aborted) setRead({ key, marks: null, warnings: [], error: asFailure(error) }); });
    return () => controller.abort();
  }, [active, client, source, branchId, tag, key, settled]);
  // Until preparation holds the whole branch, the history answers.
  const history = !client.preparedQuery || (current !== undefined && current.marks === null && current.error === undefined);
  const reply = useProduct<FocusReply>(client, history && track && tag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'national-focus', params: { branchId: track.branch.id, tag } } : null, active, validateFocusReply);
  const historyData = useMemo(() => reply.data && { ...historyMarks(reply.data), warnings: reply.data.warnings }, [reply.data]);
  if (history) return { data: historyData, error: reply.error };
  return { data: current?.marks ? { ...current.marks, warnings: current.warnings } : undefined, error: current?.error };
}
