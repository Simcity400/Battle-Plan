import { useCallback, useMemo, useRef, useState } from 'react';
import type { FamilyViewProps } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { PlaybackController } from '../../dashboard/playback.ts';
import { useProduct } from '../../dashboard/products.ts';
import { classifyFailure } from '../../dashboard/queryError.ts';
import { rowRequest, rowSource } from '../../families/table/rowSelection.ts';
import type { EventFeedCategory, EventFeedItem, ScriptEdge } from '../../generated/dashboard';
import { effectiveEventItem, validateFeedRow } from '../event-feed/eventFeedData.ts';
import type { HoverText } from '../event-feed/hoverText.ts';
import { occurrenceItem, validateOccurrence } from '../event-feed/scriptOccurrences.ts';

export interface EventDetail {
  readonly pending: boolean;
  readonly missing: string | null;
  readonly item: EventFeedItem | null;
  readonly effectsMissing?: readonly string[];
}

export type DetailProduct = 'decisions-tail' | 'event-choices-tail' | 'event-feed-tail';

export interface DetailSubject {
  readonly key: string;
  readonly hour: number;
  readonly asOfHour?: number;
}

const NOT_IN_LOG = 'This event is not in the log at the hour it was read';
const CATEGORIES: EventFeedCategory[] = ['warsPeace', 'diplomacy', 'politics', 'milestones'];

export function openedItem(product: DetailProduct, data: unknown): EventDetail {
  const reply = data as { readonly rows?: unknown; readonly unavailable?: unknown } | null;
  const rows = reply?.rows;
  const effectsMissing = Array.isArray(reply?.unavailable)
    ? reply.unavailable.filter((entry): entry is { kind: string; reason: string } => typeof entry?.kind === 'string' && typeof entry?.reason === 'string' && entry.kind === 'scriptEffectDescription').map(entry => entry.reason)
    : [];
  const first = Array.isArray(rows) ? rows[0] as Record<string, unknown> | undefined : undefined;
  if (!first) return { pending: false, missing: NOT_IN_LOG, item: null };
  try {
    const item = product !== 'event-feed-tail'
      ? (first.occurrence ? occurrenceItem(validateOccurrence(first.occurrence)) : null)
      : effectiveEventItem(validateFeedRow(first.row), (first.choice as ScriptEdge | null | undefined) ?? null);
    return { pending: false, missing: null, item, ...(effectsMissing.length ? { effectsMissing } : {}) };
  } catch (error) {
    return { pending: false, missing: error instanceof Error ? error.message : NOT_IN_LOG, item: null };
  }
}

export function detailRequest(track: BoundTrack | undefined, product: DetailProduct, subject: DetailSubject | null) {
  if (!track || !subject) return null;
  const at = rowSource(track, subject.asOfHour ?? subject.hour);
  return product === 'event-feed-tail'
    ? rowRequest(at, product, { row: subject.key, limit: 1, categories: CATEGORIES })
    : rowRequest(at, product, { row: subject.key, limit: 1 });
}

export function useEventDetail(runtime: FamilyViewProps['runtime'], trackId: string | undefined, product: DetailProduct, subject: DetailSubject | null, active: boolean): EventDetail | null {
  const track = runtime.tracks.find(item => item.id === trackId);
  const request = detailRequest(track, product, subject);
  const { data, error } = useProduct<unknown>(runtime.client, request, active, undefined, track?.source);
  if (!subject) return null;
  if (error) return { pending: false, missing: classifyFailure(error)?.reason ?? error.message, item: null };
  if (data === undefined) return { pending: true, missing: null, item: null };
  return openedItem(product, data);
}

export function hoverSubject(track: BoundTrack | undefined, clock: Pick<PlaybackController, 'getHour'>, following: boolean, names: (key: string) => boolean, subject: string | null): DetailSubject | null {
  if (subject === null || !track || !names(subject)) return null;
  const { firstHour, lastHour } = track.branch;
  return { key: subject, hour: following ? Math.min(lastHour, Math.max(firstHour, Math.floor(clock.getHour()) + track.axisOffset)) : lastHour };
}

export function useHoverText(
  runtime: FamilyViewProps['runtime'],
  trackId: string | undefined,
  product: DetailProduct,
  following: boolean,
  active: boolean,
  names: (key: string) => boolean = () => true,
): { readonly onHoverRow: (subject: string | null) => void; readonly text: HoverText | null } {
  const track = runtime.tracks.find(item => item.id === trackId);
  const scope = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : '';
  const [hovered, setHovered] = useState<{ readonly scope: string; readonly subject: DetailSubject } | null>(null);
  const latest = useRef({ track, scope, clock: runtime.clock, following, names });
  latest.current = { track, scope, clock: runtime.clock, following, names };
  const onHoverRow = useCallback((subject: string | null) => {
    const now = latest.current;
    const held = hoverSubject(now.track, now.clock, now.following, now.names, subject);
    setHovered(held ? { scope: now.scope, subject: held } : null);
  }, []);
  const shown = hovered?.scope === scope ? hovered.subject : null;
  const detail = useEventDetail(runtime, trackId, product, shown, active && shown !== null);
  const answered = shown && detail && !detail.pending ? detail : null;
  const key = answered ? shown!.key : null;
  const description = answered?.item?.description ?? null;
  const gaps = answered?.item?.gaps ?? null;
  const named = answered?.item?.options ? JSON.stringify(answered.item.options.map(option => option.name)) : null;
  const missing = answered?.missing ?? null;
  const text = useMemo<HoverText | null>(
    () => (key === null ? null : { key, description, gaps, options: named === null ? null : JSON.parse(named) as (string | null)[], missing }),
    [key, description, gaps, named, missing],
  );
  return { onHoverRow, text };
}
