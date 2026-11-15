import type { EventFeedItem } from '../../generated/dashboard';

/** Where the feed could not expand an item's scripted text (U+FFFC in the reply): never an authored character. */
export const SCRIPTED_GAP = '\uFFFC';
export const GAP_TEXT = 'not recorded';

/** A script item's reasons: why its name is missing, and why its text has gaps. */
export type ItemReasons = Pick<EventFeedItem, 'nameMissing' | 'gaps'>;
export const reasonsOf = (item: EventFeedItem | null | undefined): ItemReasons => item ?? {};

export type ScriptedPart = { text: string } | { gap: string };

/** `text` split at each gap; a gap carries `reason` for its tooltip. */
export function scriptedParts(text: string, reason: string | undefined): ScriptedPart[] {
  if (!text.includes(SCRIPTED_GAP)) return [{ text }];
  const why = `Not recorded: ${reason ?? 'its scripted text could not be expanded'}`;
  return text.split(SCRIPTED_GAP).flatMap((piece, index): ScriptedPart[] => [...index > 0 ? [{ gap: why }] : [], ...piece ? [{ text: piece }] : []]);
}

/** `text` as plain words, each gap written out: for tooltips, screen readers and sorting. */
export const plainScripted = (text: string): string => text.split(SCRIPTED_GAP).join(`[${GAP_TEXT}]`);

/** The tooltip rows naming why an item's name is missing or has gaps. */
export function reasonRows(item: EventFeedItem | null | undefined): { label: string; value: string; tone: 'muted' }[] {
  const { nameMissing, gaps } = reasonsOf(item);
  return [...!item?.name && nameMissing ? [{ label: 'Name not recorded', value: nameMissing, tone: 'muted' as const }] : [],
    ...gaps && [item?.name, item?.description, ...item?.options?.map(option => option.name) ?? []].some(text => text?.includes(SCRIPTED_GAP))
      ? [{ label: 'Text not recorded', value: gaps, tone: 'muted' as const }] : []];
}
