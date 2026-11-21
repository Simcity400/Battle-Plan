import type { EventFeedItem } from '../../generated/dashboard';

export const SCRIPTED_GAP = '\uFFFC';
export const GAP_TEXT = 'not recorded';

export type ItemReasons = Pick<EventFeedItem, 'nameMissing' | 'gaps'>;
export const reasonsOf = (item: EventFeedItem | null | undefined): ItemReasons => item ?? {};

export type ScriptedPart = { text: string } | { gap: string };

export function scriptedParts(text: string, reason: string | undefined): ScriptedPart[] {
  if (!text.includes(SCRIPTED_GAP)) return [{ text }];
  const why = `Not recorded: ${reason ?? 'its scripted text could not be expanded'}`;
  return text.split(SCRIPTED_GAP).flatMap((piece, index): ScriptedPart[] => [...index > 0 ? [{ gap: why }] : [], ...piece ? [{ text: piece }] : []]);
}

export const plainScripted = (text: string): string => text.split(SCRIPTED_GAP).join(`[${GAP_TEXT}]`);

export function reasonRows(item: EventFeedItem | null | undefined): { label: string; value: string; tone: 'muted' }[] {
  const { nameMissing, gaps } = reasonsOf(item);
  return [...!item?.name && nameMissing ? [{ label: 'Name not recorded', value: nameMissing, tone: 'muted' as const }] : [],
    ...gaps && [item?.name, item?.description, ...item?.options?.map(option => option.name) ?? []].some(text => text?.includes(SCRIPTED_GAP))
      ? [{ label: 'Text not recorded', value: gaps, tone: 'muted' as const }] : []];
}
