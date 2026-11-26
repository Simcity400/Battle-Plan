import { countryFromKey } from './countryKey.ts';
import type { BoundedReadView } from '../families/contract.ts';
import type { PointSourceRead } from '../families/plan.ts';
import { artworkArt } from '../families/art/provider.ts';
import { unavailableArt, type ArtResolution } from '../families/art/types.ts';
import { isOrderedSpans } from '../charts/series.ts';
import { artworkEntry, keepDrawableIcons } from './artwork.ts';
import { contentTitle } from './contentTitles.ts';
import { formatDateOnly } from '../families/valueFormat.ts';
import type { Artwork, IdeaHistoryReply, IdeaHolding, IdeaInfo, IdeaLook } from '../generated/dashboard';

export const NAME_NOT_RECORDED = 'Name not recorded';

export function ideaName(art: Artwork | undefined, info: IdeaInfo | undefined, unavailable: string): string {
  return contentTitle(art?.name, art?.token ?? info?.token, info?.nameMissing ? NAME_NOT_RECORDED : unavailable);
}

export function ideaNameMissing(art: Artwork | undefined, info: IdeaInfo | undefined): string | null {
  return art?.name == null && info?.nameMissing ? `${NAME_NOT_RECORDED}: ${info.nameMissing}` : null;
}

export function ideaArtwork(readView: BoundedReadView, look: IdeaLook): ArtResolution {
  if (look.artwork.icon === null && look.artworkMissing) {
    return unavailableArt(look.artworkMissing);
  }
  return artworkArt(readView, look.artwork);
}

export function ideaArtMissing(look: IdeaLook, icon: ArtResolution): string | null {
  if (icon.status === 'unavailable' || icon.status === 'confirmedMissing' || icon.status === 'refused' || icon.status === 'failed') {
    return icon.reason;
  }
  return look.artwork.icon === null && look.artworkMissing ? look.artworkMissing : null;
}

export function lookOf(reply: IdeaHistoryReply, holding: IdeaHolding): IdeaLook {
  return reply.looks[holding.look]!;
}

export function holdingLabel(look: IdeaLook, holding: IdeaHolding): string {
  return holding.character ? look.artwork.name ?? 'Holder not recorded' : ideaName(look.artwork, look.info, 'Idea name unavailable');
}

export function holdingSince(holding: IdeaHolding): { readonly label: string; readonly value: string } {
  return { label: holding.start === 'heldBefore' ? 'Held before' : 'Since', value: formatDateOnly(holding.firstHour) };
}

export function holdingUntil(holding: IdeaHolding): { readonly label: string; readonly value: string } {
  switch (holding.end.kind) {
    case 'held': return { label: 'Until', value: 'Held now' };
    case 'ended': return { label: 'Until', value: formatDateOnly(holding.lastHour) };
    case 'endedBy': return { label: 'Until', value: formatDateOnly(holding.end.hour) };
    case 'lastSeen': return { label: 'Last seen', value: formatDateOnly(holding.lastHour) };
  }
}

export function holdingSpanFlags(holding: IdeaHolding, hour: number): { readonly lastHour: number; readonly open: boolean; readonly endUnseen: boolean; readonly heldBefore?: true; readonly startBy?: true } {
  const open = holding.end.kind === 'held';
  return {
    lastHour: open ? hour : holding.lastHour,
    open,
    endUnseen: holding.end.kind === 'endedBy' || holding.end.kind === 'lastSeen',
    ...(holding.start === 'heldBefore' ? { heldBefore: true as const } : holding.start === 'firstObserved' ? { startBy: true as const } : {}),
  };
}

export function unobservedThrough(reply: IdeaHistoryReply, hour: number): IdeaHistoryReply['unobserved'] {
  const last = reply.unobserved.at(-1);
  if (!last || !reply.unrecorded || last.lastHour !== reply.hour) return reply.unobserved;
  return [...reply.unobserved.slice(0, -1), { firstHour: last.firstHour, lastHour: Math.max(last.lastHour, hour) }];
}

type Shape = Record<string, unknown>;
const object = (value: unknown): value is Shape => value != null && typeof value === 'object';
const nullableText = (value: unknown) => value === null || typeof value === 'string';
const strings = (value: unknown) => Array.isArray(value) && value.every(item => typeof item === 'string');
const line = (value: unknown) => object(value) && typeof value.key === 'string' && typeof value.fixed5 === 'string'
  && /^-?\d{1,39}$/.test(value.fixed5) && typeof value.hidden === 'boolean';
const group = (value: unknown) => object(value) && nullableText(value.target) && Array.isArray(value.lines)
  && value.lines.every(line) && strings(value.tooltips) && typeof value.instant === 'boolean';
const trait = (value: unknown) => object(value) && typeof value.token === 'string' && nullableText(value.name)
  && typeof value.known === 'boolean' && nullableText(value.unknown) && Array.isArray(value.lines) && value.lines.every(line)
  && Array.isArray(value.targeted) && value.targeted.every(group) && Array.isArray(value.equipment) && value.equipment.every(group) && strings(value.tooltips);
const info = (value: unknown) => object(value) && typeof value.token === 'string'
  && nullableText(value.category) && nullableText(value.slot) && typeof value.law === 'boolean' && typeof value.designer === 'boolean'
  && typeof value.effectsKnown === 'boolean' && nullableText(value.unknown)
  && nullableText(value.nameMissing) && Array.isArray(value.effects) && value.effects.every(line)
  && Array.isArray(value.research) && value.research.every(line) && Array.isArray(value.targeted) && value.targeted.every(group)
  && Array.isArray(value.equipment) && value.equipment.every(group) && Array.isArray(value.traits) && value.traits.every(trait) && strings(value.tooltips);
const formats = (value: unknown) => Array.isArray(value) && value.every(format => object(format) && typeof format.key === 'string'
  && Number.isInteger(format.flags) && Number.isInteger(format.precision) && Number.isInteger(format.feature)
  && (format.index === null || Number.isInteger(format.index)) && nullableText(format.label) && nullableText(format.postfix));
const texts = (value: unknown) => Array.isArray(value) && value.every(item => object(item) && typeof item.key === 'string' && nullableText(item.text));

const STARTS = new Set(['seen', 'heldBefore', 'firstObserved']);
const ENDS = new Set(['held', 'ended', 'lastSeen']);

function end(value: unknown, last: number, hour: number): boolean {
  if (!object(value)) return false;
  if (value.kind === 'endedBy') return Number.isInteger(value.hour) && (value.hour as number) > last + 1 && (value.hour as number) <= hour;
  return typeof value.kind === 'string' && ENDS.has(value.kind) && (value.kind !== 'held' || last === hour);
}

function isIdeaHistoryReply(data: unknown): data is IdeaHistoryReply {
  if (!object(data) || !Number.isInteger(data.hour) || !(data.nextChange === null || (Number.isInteger(data.nextChange) && (data.nextChange as number) > (data.hour as number)))) return false;
  const hour = data.hour as number;
  const looks = data.looks;
  if (!Array.isArray(looks) || !looks.every(look => object(look) && info(look.info) && artworkEntry(look.artwork) && (look.artworkMissing === undefined || typeof look.artworkMissing === 'string'))) return false;
  const holdings = data.holdings;
  const holding = (value: unknown) => object(value) && typeof value.token === 'string' && Number.isInteger(value.firstHour)
    && Number.isInteger(value.lastHour) && (value.firstHour as number) <= (value.lastHour as number) && (value.lastHour as number) <= hour
    && typeof value.start === 'string' && STARTS.has(value.start) && end(value.end, value.lastHour as number, hour)
    && Number.isInteger(value.look) && (value.look as number) >= 0 && (value.look as number) < looks.length;
  if (!Array.isArray(holdings) || !holdings.every(holding)) return false;
  const held = data.held;
  return Array.isArray(held) && held.every(at => Number.isInteger(at) && at >= 0 && at < holdings.length && (holdings[at] as IdeaHolding).end.kind === 'held')
    && nullableText(data.unrecorded) && Array.isArray(data.categories) && Array.isArray(data.unobserved) && isOrderedSpans(data.unobserved)
    && strings(data.warnings) && formats(data.modifiers) && texts(data.texts);
}

export function validateIdeaHistoryReply(data: unknown): IdeaHistoryReply {
  if (!isIdeaHistoryReply(data)) throw new Error('The idea history reply is malformed.');
  keepDrawableIcons(data.looks.map(look => look.artwork));
  return data;
}

export function ideaHistoryRead(branchId: string, tag: string): PointSourceRead {
  const params = { branchId, country: countryFromKey(tag) };
  return {
    shape: 'point', channel: 'panel', coverage: { mode: 'keyed', keys: [tag] },
    product: 'idea-history', params, followQuery: { panel: 'idea-history', ...params },
    validate: validateIdeaHistoryReply,
  };
}
