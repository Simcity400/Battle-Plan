import { object } from '../../dashboard/validation.ts';
import type { DesignHistory, EquipmentDesignsReply, HourSpan } from '../../generated/dashboard';

const ordered = (spans: readonly HourSpan[]) => Array.isArray(spans)
  && spans.every((span, index) => Number.isInteger(span.firstHour) && Number.isInteger(span.lastHour) && span.firstHour <= span.lastHour && (index === 0 || spans[index - 1].lastHour < span.firstHour));

export function validateDesigns(value: unknown): EquipmentDesignsReply {
  const reply = value as EquipmentDesignsReply;
  if (!object(reply) || typeof reply.tag !== 'string' || !ordered(reply.known) || !Array.isArray(reply.warnings) || !Array.isArray(reply.icons)
    || !Array.isArray(reply.designs) || !reply.designs.every(design => object(design) && /^\d{1,20}$/.test(design.id) && ordered(design.lives) && ordered(design.obsolete)
      && object(design.name) && typeof design.name.token === 'string' && (design.name.name === null || typeof design.name.name === 'string'))) {
    throw new Error('Invalid equipment designs reply');
  }
  return reply;
}

const covering = (spans: readonly HourSpan[], hour: number) => spans.find(span => span.firstHour <= hour && hour <= span.lastHour) ?? null;

/** Whether the designs were known at `hour`: a holder of the tag was recorded then. */
export const designsKnown = (reply: Pick<EquipmentDesignsReply, 'known'>, hour: number) => covering(reply.known, hour) !== null;

/** When a design (or its obsolete flag) was first read, and how much earlier it may be: the designs were read the hour
 * before without it ('between-reads'), it was there at the first recorded hour ('first-read'), or the hour before
 * was not recorded ('after-gap'). */
export interface DesignSeen { hour: number; context: 'between-reads' | 'first-read' | 'after-gap' }

function seenAt(reply: Pick<EquipmentDesignsReply, 'known'>, hour: number): DesignSeen {
  if (reply.known.length && hour <= reply.known[0].firstHour) return { hour, context: 'first-read' };
  return { hour, context: designsKnown(reply, hour - 1) ? 'between-reads' : 'after-gap' };
}

/** One design of the country at the hour shown. */
export interface DesignRow {
  key: string;
  design: DesignHistory;
  name: string | null;
  type: string | null;
  icon: string | null;
  obsolete: boolean;
  firstSeen: DesignSeen;
  seenObsolete: DesignSeen | null;
}

/** The country's saved designs at `hour`, current and obsolete, each with when it and its obsolete flag were first read.
 * Designs whose type the included set leaves out are skipped. */
export function designsAt(reply: EquipmentDesignsReply, hour: number, included: ReadonlySet<string> | null): DesignRow[] {
  const rows: DesignRow[] = [];
  for (const design of reply.designs) {
    if (!covering(design.lives, hour) || design.lives.length === 0) continue;
    if (included && !included.has(design.name.token)) continue;
    const flagged = covering(design.obsolete, hour);
    rows.push({ key: design.id, design, name: design.name.name, type: design.name.typeName,
      icon: design.name.icon ? reply.icons.find(icon => icon.key === design.name.icon)?.image ?? null : null,
      obsolete: flagged !== null, firstSeen: seenAt(reply, design.lives[0].firstHour), seenObsolete: flagged && seenAt(reply, flagged.firstHour) });
  }
  return rows;
}

/** The last hour at or before `hour` at which any design's row changes: the rows at `hour` are those at it. */
export function designsSince(reply: EquipmentDesignsReply, hour: number): number {
  let since = Number.MIN_SAFE_INTEGER;
  for (const design of reply.designs) for (const span of [...design.lives, ...design.obsolete]) {
    if (span.firstHour <= hour) since = Math.max(since, span.firstHour);
    if (span.lastHour < hour) since = Math.max(since, span.lastHour + 1);
  }
  return since;
}

/** The read's date, "By" one when the fact may be older than the read. */
export const seenText = (seen: DesignSeen | null, label: (hour: number) => string) =>
  seen === null ? '—' : seen.context === 'between-reads' ? label(seen.hour) : `By ${label(seen.hour)}`;

export const seenNote = (seen: DesignSeen | null): string | null =>
  seen?.context === 'first-read' ? 'Already so at the start of the recorded designs'
    : seen?.context === 'after-gap' ? 'First shown after hours the designs were not recorded' : null;

/** Every equipment type the country's designs use, for the Included types setting. */
export function designTypes(reply: EquipmentDesignsReply): { value: string; label: string }[] {
  const types = new Map<string, string>();
  for (const design of reply.designs) {
    const key = design.name.token;
    if (key && !types.has(key)) types.set(key, design.name.typeName ?? 'Equipment type name unavailable');
  }
  return [...types].map(([value, label]) => ({ value, label })).sort((a, b) => a.label.localeCompare(b.label));
}

export const designRevision = (row: DesignRow) => [row.name, row.type, row.icon, row.obsolete, row.firstSeen.hour, row.firstSeen.context, row.seenObsolete?.hour, row.seenObsolete?.context];
