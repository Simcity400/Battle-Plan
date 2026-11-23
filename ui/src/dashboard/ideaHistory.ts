import { countryFromKey } from './countryKey.ts';
import { useMemo } from 'react';
import { defaultCountryKey } from './countrySelection.ts';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { settledHour, trackHourOf } from './settledHour.ts';
import { useAtHead } from './atHead.ts';
import { useProduct } from './products.ts';
import { validateIdeaHistoryReply } from './ideaHistoryReply.ts';
import type { IdeaHistoryReply } from '../generated/dashboard';
import { productRequest } from './request.ts';

export function useIdeaHistory(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const request = track && tag ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'idea-history', params: { branchId: track.branch.id, country: countryFromKey(tag) } }) : null;
  const { data, error, retry } = useProduct<IdeaHistoryReply>(runtime.client, request, active, validateIdeaHistoryReply);
  return { track, tag, data, error, retry };
}

export function ideaChangeHours(reply: Pick<IdeaHistoryReply, 'spans' | 'unobserved' | 'identityStarts'>): number[] {
  const hours = new Set<number>(reply.identityStarts);
  for (const span of [...reply.spans, ...reply.unobserved]) { hours.add(span.firstHour); hours.add(span.lastHour + 1); }
  return [...hours].sort((a, b) => a - b);
}

export function useIdeaHour(widget: WidgetSpec, runtime: WidgetRuntime, track: BoundTrack | undefined, data: IdeaHistoryReply | undefined, active: boolean): number | null {
  const live = widget.time.mode === 'follow';
  const changes = useMemo(() => data ? ideaChangeHours(data) : [], [data]);
  const followed = useAtHead(runtime.clock, active && live && !!data, clockHour => {
    const hour = trackHourOf(widget, track, runtime.tracks, clockHour);
    return hour === null ? null : settledHour(changes, hour);
  });
  return live ? followed : trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour());
}
