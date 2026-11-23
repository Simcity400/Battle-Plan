import { useMemo } from 'react';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt.ts';
import { useHeldTimeline } from '../../dashboard/countryNamesAt.ts';
import { lookStarts } from '../../dashboard/countryTimeline.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useCountryGroups } from '../../dashboard/useCountryGroups.ts';
import { buildDatedCountryLooks } from '../sources.ts';
import type { LogLooksMap } from './declaration.ts';

const NO_TAGS: readonly string[] = [];
const NO_LOOKS: LogLooksMap = {};

export function useLogLooks(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  firstHour: number,
  lastHour: number,
  active: boolean,
): LogLooksMap {
  const groups = useCountryGroups(runtime, track, active && !!track);
  const allTagsText = useMemo(
    () => (groups.data?.countryIdentities ?? []).map(history => history.tag).sort().join(','),
    [groups.data],
  );
  const allTags = useMemo(() => (allTagsText ? allTagsText.split(',') : NO_TAGS), [allTagsText]);
  const revision = useContentRevision(runtime.client, active && !!track);
  const timeline = useHeldTimeline(runtime.client, active ? track : undefined, revision, active && !!track);
  const startsText = active && track && allTags.length > 0
    ? JSON.stringify(allTags.map(tag => [tag, lookStarts(timeline, tag, firstHour, lastHour)]))
    : '[]';
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const wantedLabels = useMemo(
    () => (active && track ? starts.flatMap(([tag, hours]) => hours.flatMap(([at, held]) => (held ? [{ tag, hour: at }] : []))) : []),
    [active, track, starts],
  );
  const { exactAt } = useCountryLabelsAt(runtime.client, active ? track : undefined, wantedLabels, active && !!track, Infinity, true);
  const builtLooks = active && track && allTags.length > 0
    ? buildDatedCountryLooks(timeline, allTags, firstHour, lastHour, exactAt, false)
    : NO_LOOKS;
  const looksText = active && track && allTags.length > 0 ? JSON.stringify(builtLooks) : '{}';
  return useMemo<LogLooksMap>(
    () => (looksText === '{}' ? NO_LOOKS : JSON.parse(looksText) as LogLooksMap),
    [looksText],
  );
}
