import { knownValue, partialValue, type Value } from '../../families/value.ts';
import type { EventFeedGap, EventFeedGapField, EventFeedItem } from '../../generated/dashboard';

const SCRIPTED_GAP = '\uFFFC';
const GAP_TEXT = 'not recorded';
const DEFAULT_REASON = 'its scripted text could not be expanded';

export type ItemReasons = Pick<EventFeedItem, 'nameMissing' | 'gaps'>;
export const reasonsOf = (item: EventFeedItem | null | undefined): ItemReasons => item ?? {};

export type ScriptedPart =
  | { readonly text: string }
  | { readonly gap: string; readonly inline: string; readonly key?: string; readonly reason?: string };

export function gapSummary(gap: EventFeedGap): string {
  return gap.key ? `${gap.key}: ${gap.reason}` : gap.reason;
}

export function gapsForField(
  gaps: readonly EventFeedGap[] | null | undefined,
  field: EventFeedGapField,
  ordinal?: number,
): readonly EventFeedGap[] {
  if (!gaps?.length) return [];
  return gaps.filter(gap => gap.field === field && (field !== 'option' || gap.ordinal === ordinal));
}

export function scriptedParts(text: string, gaps?: readonly EventFeedGap[]): ScriptedPart[] {
  if (!text.includes(SCRIPTED_GAP)) return [{ text }];
  return text.split(SCRIPTED_GAP).flatMap((piece, index): ScriptedPart[] => {
    if (index === 0) return piece ? [{ text: piece }] : [];
    const entry = gaps?.[index - 1];
    const summary = entry ? gapSummary(entry) : DEFAULT_REASON;
    const part: ScriptedPart = {
      gap: `Not recorded: ${summary}`,
      inline: `${GAP_TEXT}: ${summary}`,
      ...(entry ? { key: entry.key, reason: entry.reason } : {}),
    };
    return piece ? [part, { text: piece }] : [part];
  });
}

export function plainScripted(text: string, gaps?: readonly EventFeedGap[]): string {
  if (!text.includes(SCRIPTED_GAP)) return text;
  let cursor = 0;
  return text.split(SCRIPTED_GAP).reduce((acc, piece, index) => {
    if (index === 0) return piece;
    const entry = gaps?.[cursor++];
    const marker = entry ? `[${GAP_TEXT}: ${gapSummary(entry)}]` : `[${GAP_TEXT}]`;
    return `${acc}${marker}${piece}`;
  }, '');
}

export function scriptedValue(text: string, gaps?: readonly EventFeedGap[]): Value<string> {
  if (!text.includes(SCRIPTED_GAP)) return knownValue(text);
  const shown = plainScripted(text, gaps);
  const reason = gaps?.length ? gaps.map(gapSummary).join('; ') : DEFAULT_REASON;
  return partialValue(shown, reason);
}

export function isEventFeedGap(value: unknown): value is EventFeedGap {
  if (typeof value !== 'object' || value === null) return false;
  const gap = value as Record<string, unknown>;
  const validField = gap.field === 'title' || gap.field === 'description' || gap.field === 'option';
  const validOrdinal = gap.field === 'option'
    ? (typeof gap.ordinal === 'number' && Number.isInteger(gap.ordinal) && gap.ordinal >= 0)
    : gap.ordinal === undefined;
  return validField && validOrdinal && typeof gap.key === 'string' && typeof gap.reason === 'string';
}
