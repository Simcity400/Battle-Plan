import type { FamilyViewProps } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products.ts';
import { classifyFailure } from '../../dashboard/queryError.ts';
import { rowRequest, rowSource } from '../../families/table/rowSelection.ts';
import type { EventFeedCategory, EventFeedItem } from '../../generated/dashboard';
import { validateFeedRow } from '../event-feed/eventFeedData.ts';
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
      : validateFeedRow(first.row).item;
    return { pending: false, missing: null, item, ...(effectsMissing.length ? { effectsMissing } : {}) };
  } catch (error) {
    return { pending: false, missing: error instanceof Error ? error.message : NOT_IN_LOG, item: null };
  }
}

export function useEventDetail(runtime: FamilyViewProps['runtime'], trackId: string | undefined, product: DetailProduct, subject: DetailSubject | null, active: boolean): EventDetail | null {
  const track = runtime.tracks.find(item => item.id === trackId);
  const hour = subject ? (subject.asOfHour ?? subject.hour) : null;
  const at = track && subject && hour !== null ? rowSource(track, hour) : null;
  const request = !at || !subject
    ? null
    : product === 'event-feed-tail'
      ? rowRequest(at, product, { row: subject.key, limit: 1, categories: CATEGORIES })
      : rowRequest(at, product, { row: subject.key, limit: 1 });
  const { data, error } = useProduct<unknown>(runtime.client, request, active, undefined, track?.source);
  if (!subject) return null;
  if (error) return { pending: false, missing: classifyFailure(error)?.reason ?? error.message, item: null };
  if (data === undefined) return { pending: true, missing: null, item: null };
  return openedItem(product, data);
}
