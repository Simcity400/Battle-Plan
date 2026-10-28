import type { DesignHistory, ProductionCatalogueReply } from '../../generated/dashboard';
import { object } from '../../dashboard/validation.ts';
import { isEconomyRuns, isHolderMap, lastRunBy, runAt, tagAt, validateEconomyReply, variantNameAt } from '../../dashboard/economyHistory.ts';

/** Whether the designs were read complete at `hour`. */
export const designsKnown = (reply: Pick<ProductionCatalogueReply, 'known'>, hour: number) => runAt(reply.known, hour) !== null;

export function validateDesigns(value: unknown): ProductionCatalogueReply {
  return validateEconomyReply<ProductionCatalogueReply>(value, reply => reply.view === 'designs' && isEconomyRuns(reply.known) && isHolderMap(reply.holders)
    && (reply.designsFrom === null || Number.isSafeInteger(reply.designsFrom))
    && Array.isArray(reply.designs) && reply.designs.every(row => object(row) && typeof row.id === 'string' && isEconomyRuns(row.creator)
      && isEconomyRuns(row.obsolete) && typeof row.token === 'string' && (row.archetype === null || typeof row.archetype === 'string')
      && (row.archetypeName === null || typeof row.archetypeName === 'string') && Array.isArray(row.names)), 'equipment designs');
}

/** When a design (or its obsolete flag) was first read, and what that read proves.
 * The recording names no creation and no replacement design, so these are bounds:
 * the fact held by `hour`. `context` says how much earlier it may be: the designs
 * were read the hour before without it ('between-reads'), it was already there at
 * the designs' first complete read ('first-read'), or the read before it was
 * missing or incomplete ('after-gap'). */
export interface DesignSeen { hour: number; context: 'between-reads' | 'first-read' | 'after-gap' }

/** One design of the country at the hour shown. */
export interface DesignRow {
  key: string;
  design: DesignHistory;
  name: string | null;
  type: string | null;
  obsolete: boolean;
  firstSeen: DesignSeen | null;
  seenObsolete: DesignSeen | null;
}

/** How far back a fact first read at `hour` may go: the designs were read complete
 * the hour before without it, or it was there at their first read, or neither. */
function seenAt(reply: Pick<ProductionCatalogueReply, 'known' | 'designsFrom'>, hour: number, absentBefore: boolean): DesignSeen {
  if (reply.designsFrom !== null && hour <= reply.designsFrom) return { hour, context: 'first-read' };
  return { hour, context: absentBefore && designsKnown(reply, hour - 1) ? 'between-reads' : 'after-gap' };
}

/** When the obsolete flag was first read in its current stretch: the start of the
 * run holding it at `hour`, walked back over adjoining earlier runs that held it too.
 * Only a current flag read the hour before bounds it from below. */
function obsoleteSeen(reply: ProductionCatalogueReply, runs: DesignHistory['obsolete'], hour: number): DesignSeen | null {
  let index = runs.findIndex(run => run[0] <= hour && hour <= run[1]);
  if (index < 0 || runs[index][2] !== '1') return null;
  while (index > 0 && runs[index - 1][2] === '1' && runs[index - 1][1] === runs[index][0] - 1) index--;
  const before = runs[index - 1];
  return seenAt(reply, runs[index][0], !!before && before[2] === '0' && before[1] === runs[index][0] - 1);
}

/** The first read of the design as a saved design. */
function firstSeenOf(reply: ProductionCatalogueReply, runs: DesignHistory['creator']): DesignSeen | null {
  return runs.length ? seenAt(reply, runs[0][0], true) : null;
}

/** The country's saved designs (by creator) at `hour`: current and obsolete, each
 * with when it and its obsolete flag were first read. Designs the included types
 * leave out are skipped. */
export function designsAt(reply: ProductionCatalogueReply, tag: string, hour: number, included: ReadonlySet<string> | null): DesignRow[] {
  const rows: DesignRow[] = [];
  for (const design of reply.designs) {
    const creator = runAt(design.creator, hour);
    if (!creator || tagAt(reply.holders[creator[2]], hour) !== tag) continue;
    if (included && !included.has(design.archetype ?? design.token)) continue;
    const obsolete = runAt(design.obsolete, hour)?.[2] === '1';
    rows.push({ key: design.id, design, name: variantNameAt(design.names, hour), type: design.archetypeName,
      obsolete, firstSeen: firstSeenOf(reply, design.creator), seenObsolete: obsolete ? obsoleteSeen(reply, design.obsolete, hour) : null });
  }
  return rows;
}

/** The read's date, "By" one when the fact may be older than the read. */
export const seenText = (seen: DesignSeen | null, label: (hour: number) => string) =>
  seen === null ? '—' : seen.context === 'between-reads' ? label(seen.hour) : `By ${label(seen.hour)}`;

/** Why a "By" date may be older than its read, for the tooltip. */
export const seenNote = (seen: DesignSeen | null): string | null =>
  seen?.context === 'first-read' ? 'Already so at the start of the recorded designs'
    : seen?.context === 'after-gap' ? 'First shown after hours the designs were not recorded' : null;

/** Every equipment type the country's designs use, for the Included types setting. */
export function designTypes(reply: ProductionCatalogueReply): { value: string; label: string }[] {
  const types = new Map<string, string>();
  for (const design of reply.designs) {
    const key = design.archetype ?? design.token;
    if (key && !types.has(key)) types.set(key, design.archetypeName ?? 'Equipment type name unavailable');
  }
  return [...types].map(([value, label]) => ({ value, label })).sort((a, b) => a.label.localeCompare(b.label));
}

/** The last hour a design was the country's, for the menu's jump. */
export const lastSeen = (design: DesignHistory, hour: number) => lastRunBy(design.creator, hour)?.[1] ?? null;
