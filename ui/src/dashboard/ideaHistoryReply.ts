import { isOrderedSpans } from '../charts/series.ts';
import { artworkEntry, keepDrawableIcons } from './artwork.ts';
import { contentTitle } from './contentTitles.ts';
import type { Artwork, IdeaHistoryReply, IdeaInfo } from '../generated/dashboard';

export type IdeaTooltipData = Pick<IdeaHistoryReply, 'ideas' | 'artwork' | 'modifiers' | 'texts'>;

export const NAME_NOT_RECORDED = 'Name not recorded';

/** An idea's name: its artwork's, else "Name not recorded" where the recording says why (`ideaNameMissing`), else
 * `unavailable`. The reason names lookup keys, so it belongs in a tooltip, never in the name. */
export function ideaName(art: Artwork | undefined, info: IdeaInfo | undefined, unavailable: string): string {
  return contentTitle(art?.name, art?.token ?? info?.token, info?.nameMissing ? NAME_NOT_RECORDED : unavailable);
}

/** Why an idea shown without a name has none, for its tooltip; null when it is named or no reason is given. */
export function ideaNameMissing(art: Artwork | undefined, info: IdeaInfo | undefined): string | null {
  return art?.name == null && info?.nameMissing ? `${NAME_NOT_RECORDED}: ${info.nameMissing}` : null;
}

/** Shared tooltip inputs can accompany another history product. Validate their
 * effect arrays before the tooltip walks them; unrenderable art keeps its text. */
function isIdeaTooltipData(data: unknown): data is IdeaTooltipData {
  const reply = data as IdeaTooltipData;
  const object = (value: unknown): value is Record<string, unknown> => value != null && typeof value === 'object';
  const nullableText = (value: unknown) => value === null || typeof value === 'string';
  const strings = (value: unknown) => Array.isArray(value) && value.every(item => typeof item === 'string');
  const line = (value: unknown) => object(value) && typeof value.key === 'string' && typeof value.fixed5 === 'string'
    && /^-?\d{1,39}$/.test(value.fixed5) && typeof value.hidden === 'boolean';
  const group = (value: unknown) => object(value) && nullableText(value.target) && Array.isArray(value.lines)
    && value.lines.every(line) && strings(value.tooltips) && typeof value.instant === 'boolean';
  const trait = (value: unknown) => object(value) && typeof value.token === 'string' && nullableText(value.name)
    && typeof value.known === 'boolean' && nullableText(value.unknown) && Array.isArray(value.lines) && value.lines.every(line)
    && Array.isArray(value.targeted) && value.targeted.every(group) && Array.isArray(value.equipment) && value.equipment.every(group) && strings(value.tooltips);
  const ok = object(reply) && Array.isArray(reply.ideas) && reply.ideas.every(info => object(info) && typeof info.token === 'string'
    && nullableText(info.category) && nullableText(info.slot) && typeof info.law === 'boolean' && typeof info.designer === 'boolean'
    && typeof info.effectsKnown === 'boolean' && nullableText(info.unknown)
    && nullableText(info.nameMissing) && Array.isArray(info.effects) && info.effects.every(line)
    && Array.isArray(info.research) && info.research.every(line) && Array.isArray(info.targeted) && info.targeted.every(group)
    && Array.isArray(info.equipment) && info.equipment.every(group) && Array.isArray(info.traits) && info.traits.every(trait) && strings(info.tooltips))
    && Array.isArray(reply.artwork) && reply.artwork.every(artworkEntry)
    && Array.isArray(reply.modifiers) && reply.modifiers.every(format => object(format) && typeof format.key === 'string'
      && Number.isInteger(format.flags) && Number.isInteger(format.precision) && Number.isInteger(format.feature)
      && (format.index === null || Number.isInteger(format.index)) && nullableText(format.label) && nullableText(format.postfix))
    && Array.isArray(reply.texts) && reply.texts.every(item => object(item) && typeof item.key === 'string' && nullableText(item.text));
  return ok;
}

export function validateIdeaTooltipData(data: unknown): IdeaTooltipData {
  if (!isIdeaTooltipData(data)) throw new Error('The idea tooltip data is malformed.');
  keepDrawableIcons(data.artwork);
  return data;
}

/** Checks the reply's shape before the widget reads it. */
export function validateIdeaHistoryReply(data: unknown): IdeaHistoryReply {
  const reply = data as IdeaHistoryReply;
  const ok = typeof reply === 'object' && reply != null && Array.isArray(reply.spans) && Array.isArray(reply.holders)
    && reply.holders.every(holder => typeof holder?.token === 'string' && typeof holder.name === 'string' && typeof holder.characterToken === 'string' && Number.isInteger(holder.character) && typeof holder.artwork === 'object' && holder.artwork != null)
    && Array.isArray(reply.categories) && Array.isArray(reply.unobserved) && isOrderedSpans(reply.unobserved) && Array.isArray(reply.warnings)
    && Array.isArray(reply.identityStarts) && reply.identityStarts.every(hour => Number.isInteger(hour))
    && isIdeaTooltipData(reply)
    && reply.spans.every(span => typeof span.token === 'string' && Number.isInteger(span.firstHour) && Number.isInteger(span.lastHour) && span.firstHour <= span.lastHour);
  if (!ok) throw new Error('The idea history reply is malformed.');
  keepDrawableIcons(reply.artwork);
  return reply;
}
