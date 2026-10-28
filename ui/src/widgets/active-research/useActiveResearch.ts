import { defaultCountryTag } from '../../dashboard/countrySelection.ts';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import type { WidgetRuntime } from '../../dashboard/definitions.ts';
import { resolveWindow, type WidgetSpec } from '../../dashboard/model.ts';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries.ts';
import { useHeldFailure } from '../../dashboard/heldFailures.ts';
import { useViewRead } from '../../dashboard/viewRead.ts';
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
  const lane = track && tag ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, content]) : null;
  // One read at a time while playback moves (`useViewRead`): the last hour's slots stay shown while the newest is read.
  const read = useViewRead<number, ReturnType<typeof activeResearchReply>>(lane, hour ?? null, String(hour), active,
    (at, signal, publish, restart) => preparedSeriesQuery(client, track!.source, 'series-state', { branchId: track!.branch.id, family: { kind: 'research' },
      reads: [{ tag: tag!, hour: at }] }, signal, restart, undefined, undefined, value => publish(activeResearchReply(value, tag!, at)))
      .then(value => activeResearchReply(value, tag!, at)), client);
  const error = useHeldFailure(read.errorKey === String(hour) ? read.error : undefined, read.data !== undefined, read.retry);
  // The hour the shown slots answer, which an older read still shown keeps.
  const answeredHour = read.viewKey === undefined ? undefined : Number(read.viewKey);
  return { track, tag, data: read.data?.data, answeredHour, grants: read.data?.grants, error, historyPreparing: read.data?.historyPreparing ?? true,
    countdownPreparing: read.data?.countdownPreparing ?? false, retry: read.retry };
}
