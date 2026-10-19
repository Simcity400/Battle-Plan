import { useEffect, useState } from 'react';
import { defaultCountryTag } from '../../dashboard/countrySelection.ts';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import type { WidgetRuntime } from '../../dashboard/definitions.ts';
import { resolveWindow, type WidgetSpec } from '../../dashboard/model.ts';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';
import type { ResearchReply, GrantInventory } from '../../generated/dashboard.ts';
import { validateResearchReply } from '../research/researchData.ts';
import { researchGrants } from './researchGrants.ts';

export function activeResearchReply(value: unknown, tag: string, hour: number) {
  const reply = value as { states?: { tag: string; hour: number; data: unknown; grants: unknown; preparing: string[] }[] };
  if (!reply || !Array.isArray(reply.states) || reply.states.length !== 1 || reply.states[0].tag !== tag || reply.states[0].hour !== hour
    || !Array.isArray(reply.states[0].preparing) || !reply.states[0].preparing.every(field => typeof field === 'string')) throw new Error('Invalid active research state.');
  return { data: validateResearchReply(reply.states[0].data), grants: researchGrants(reply.states[0].grants), historyPreparing: reply.states[0].preparing.includes('assignment') || reply.states[0].preparing.includes('breakdown'), countdownPreparing: reply.states[0].preparing.includes('remainingDays') };
}

/** The reached slot state, independent of assignment history and its pages. */
export function useActiveResearch(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window && Math.floor(window.lastHour);
  const { client } = runtime;
  const content = useContentRevision(client);
  const key = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag, hour, content]);
  const [state, setState] = useState<{ client: typeof client; key: string; data?: ResearchReply; grants?: GrantInventory; historyPreparing?: boolean; countdownPreparing?: boolean; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!active || !track || !tag || hour == null) return;
    const controller = new AbortController();
    const publish = (value: unknown) => {
      const result = activeResearchReply(value, tag, hour);
      if (!controller.signal.aborted) setState({ client, key, ...result });
    };
    void preparedSeriesQuery(client, track.source, 'series-state', { branchId: track.branch.id, family: { kind: 'research' }, reads: [{ tag, hour }] },
      controller.signal, attempt > 0, undefined, undefined, publish).then(publish).catch(error => {
      if (!controller.signal.aborted) setState({ client, key, error: String(error) });
    });
    return () => controller.abort();
  }, [client, key, active, attempt]);
  const current = state?.client === client && state.key === key ? state : undefined;
  return { track, tag, data: current?.data, grants: current?.grants, error: current?.error, historyPreparing: current?.historyPreparing ?? true, countdownPreparing: current?.countdownPreparing ?? false,
    retry: () => { setState(undefined); setAttempt(value => value + 1); } };
}
