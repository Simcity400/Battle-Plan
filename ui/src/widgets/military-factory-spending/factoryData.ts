import type { FactoryMonth, MilitaryFactorySpendingReply } from '../../generated/dashboard';
import type { SunburstItem } from '../../charts/sunburst/layout';
import type { ColumnPeriod } from '../../charts/columns';
import { validateProductionSpendingReply } from '../production-spending/spendingData.ts';
import type { SpendingWindow } from '../production-spending/spendingData';
import { REMAINDER_BANDS, TYPE_NOTE, UNRESOLVED_REASON, colouredSplit, drawnAmount, exactOf, exactSum, purposeItems, splitLeaves, validAmount } from '../dockyard-spending/dockyardData.ts';
import type { Exact, PurposeSplit } from '../dockyard-spending/dockyardData';

/** What military factory IC was spent on, in the order the chart and columns show them. */
export const PURPOSES = [
  { id: 'new', label: 'New' },
  { id: 'converted', label: 'Converted' },
] as const;

const PURPOSE_NOTE = 'Whether some of it was converted was not recorded.';
const SPLIT: PurposeSplit = { purposes: [{ key: PURPOSES[0], leaf: 'new' }, { key: PURPOSES[1], leaf: 'converted' }], note: PURPOSE_NOTE };

const int = (value: unknown): value is number => Number.isSafeInteger(value);

/** Production spending's shape, its leaves split into new and converted
 * input, with the months the reply adds. */
export function validateFactorySpendingReply(data: unknown): MilitaryFactorySpendingReply {
  const reply = validateProductionSpendingReply(data) as MilitaryFactorySpendingReply;
  const ok = splitLeaves(reply)
    && Array.isArray(reply.months) && reply.months.every((month, index) => typeof month === 'object' && month != null
      && int(month.firstHour) && int(month.lastHour) && month.firstHour <= month.lastHour
      && (index === 0 || reply.months[index - 1].lastHour < month.firstHour)
      && validAmount(month.new) && validAmount(month.converted) && validAmount(month.unrecorded) && validAmount(month.other)
      && typeof month.purposeUnproven === 'boolean' && typeof month.unresolved === 'boolean');
  if (!ok) throw new Error('The military factory spending reply is malformed.');
  return reply;
}

/** New and converted equipment by equipment type and design. */
export function factoryItems(reply: MilitaryFactorySpendingReply, window: SpendingWindow, colours: readonly string[] = []): SunburstItem[] {
  return purposeItems(reply, window, colouredSplit(SPLIT, colours));
}

/** The months' bands: the purposes, then the remainders. */
export const BANDS = [...PURPOSES, ...REMAINDER_BANDS];

/** One column per month, its bands in [`PURPOSES`] then [`REMAINDER_BANDS`]
 * order, as exact amounts divided by the recorded scale for drawing. */
export function factoryColumns(months: readonly FactoryMonth[], decimals: number): ColumnPeriod[] {
  return months.map(month => {
    const purposes = month.purposeUnproven || month.unresolved;
    return { firstHour: month.firstHour, lastHour: month.lastHour,
      values: [month.new, month.converted, month.unrecorded, month.other].map(part => drawnAmount(part, decimals)),
      uncertain: [purposes, purposes, month.purposeUnproven, false] };
  });
}

/** A month's total, known only when every part is and no design that could
 * be its equipment is left out; an unrecorded purpose only splits it. */
export function factoryMonthTotal(month: FactoryMonth): Exact {
  if (month.unresolved) return { exact: null, reason: UNRESOLVED_REASON };
  return exactSum([month.new, month.converted, month.unrecorded, month.other].map(exactOf));
}

/** What makes a month's attribution incomplete. */
export function factoryMonthNote(month: FactoryMonth): string | undefined {
  if (month.purposeUnproven) return PURPOSE_NOTE;
  if (month.unresolved) return TYPE_NOTE;
  return undefined;
}
