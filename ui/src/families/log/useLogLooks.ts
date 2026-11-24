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
  fallbackTags: readonly string[] = NO_TAGS,
): LogLooksMap {
  const groups = useCountryGroups(runtime, track, active && !!track);
  const revision = useContentRevision(runtime.client, active && !!track);
  const timeline = useHeldTimeline(runtime.client, active ? track : undefined, revision, active && !!track);
  const fallbackKey = fallbackTags.length > 0 ? [...new Set(fallbackTags)].sort().join(',') : '';
  const allTagsText = useMemo(() => {
    const merged = new Set<string>();
    for (const history of groups.data?.countryIdentities ?? []) merged.add(history.tag);
    if (timeline) {
      for (const tag of timeline.runs.keys()) merged.add(tag);
    }
    if (fallbackKey) {
      for (const tag of fallbackKey.split(',')) if (tag) merged.add(tag);
    }
    return [...merged].sort().join(',');
  }, [groups.data, timeline, fallbackKey]);
  const allTags = useMemo(() => (allTagsText ? allTagsText.split(',') : NO_TAGS), [allTagsText]);
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
