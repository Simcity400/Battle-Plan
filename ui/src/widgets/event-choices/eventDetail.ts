import type { FamilyViewProps } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products.ts';
import { classifyFailure } from '../../dashboard/queryError.ts';
import { rowRequest, rowSource } from '../../families/table/rowSelection.ts';
import type { EventFeedCategory, EventFeedItem } from '../../generated/dashboard';
import { validateFeedRow } from '../event-feed/eventFeedData.ts';
import { occurrenceItem, validateOccurrence } from '../event-feed/scriptOccurrences.ts';
import type { EventDetailSubject } from './EventDetailWindow.tsx';

export interface EventDetail {
  readonly pending: boolean;
  readonly missing: string | null;
  readonly item: EventFeedItem | null;
}

export type DetailProduct = 'event-choices-tail' | 'event-feed-tail';

const NOT_IN_LOG = 'This event is not in the log at the hour it was read';
const CATEGORIES: EventFeedCategory[] = ['warsPeace', 'diplomacy', 'politics', 'milestones'];

export function openedItem(product: DetailProduct, data: unknown): EventDetail {
  const rows = (data as { readonly rows?: unknown } | null)?.rows;
  const first = Array.isArray(rows) ? rows[0] as Record<string, unknown> | undefined : undefined;
  if (!first) return { pending: false, missing: NOT_IN_LOG, item: null };
  try {
    const item = product === 'event-choices-tail'
      ? (first.occurrence ? occurrenceItem(validateOccurrence(first.occurrence)) : null)
      : validateFeedRow(first.row).item;
    return { pending: false, missing: null, item };
  } catch (error) {
    return { pending: false, missing: error instanceof Error ? error.message : NOT_IN_LOG, item: null };
  }
}

export function useEventDetail(runtime: FamilyViewProps['runtime'], trackId: string | undefined, product: DetailProduct, subject: EventDetailSubject | null, active: boolean): EventDetail | null {
  const track = runtime.tracks.find(item => item.id === trackId);
  const hour = subject ? (subject.asOfHour ?? subject.hour) : null;
  const at = track && subject && hour !== null ? rowSource(track, hour) : null;
  const request = !at || !subject
    ? null
    : product === 'event-choices-tail'
      ? rowRequest(at, product, { row: subject.key, limit: 1 })
      : rowRequest(at, product, { row: subject.key, limit: 1, categories: CATEGORIES });
  const { data, error } = useProduct<unknown>(runtime.client, request, active, undefined, track?.source);
  if (!subject) return null;
  if (error) return { pending: false, missing: classifyFailure(error)?.reason ?? error.message, item: null };
  if (data === undefined) return { pending: true, missing: null, item: null };
  return openedItem(product, data);
}
