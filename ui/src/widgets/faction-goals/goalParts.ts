import type { DashboardClient } from '../../dashboard/client';
import type { FactionGoalsReply } from '../../generated/dashboard';

/** One asked range of a faction's goal history; a null end is open. */
export interface GoalPart { firstHour: number | null; lastHour: number | null; data?: FactionGoalsReply; error?: string }

const OVER_BUDGET = /exceeds the transport reply budget/;
export const WHOLE: readonly GoalPart[] = [{ firstHour: null, lastHour: null }];

export const rangeKey = (part: GoalPart) => `${part.firstHour}:${part.lastHour}`;

export const partAt = (parts: readonly GoalPart[], hour: number) =>
  parts.find(part => (part.firstHour === null || part.firstHour <= hour) && (part.lastHour === null || hour <= part.lastHour));

/** A range refused as over the reply budget, asked again as its two halves; null when it cannot split. */
export function splitPart(part: GoalPart, error: string, branch: { firstHour: number; lastHour: number }): GoalPart[] | null {
  if (!OVER_BUDGET.test(error)) return null;
  const first = part.firstHour ?? branch.firstHour, last = part.lastHour ?? branch.lastHour;
  if (last <= first) return null;
  const middle = first + Math.floor((last - first) / 2);
  return [{ firstHour: part.firstHour, lastHour: middle }, { firstHour: middle + 1, lastHour: part.lastHour }];
}

export type HeldParts = { client: DashboardClient; identity: string; attempt: number; parts: readonly GoalPart[] };
/** The parts held for this client, faction and attempt; another's are never shown. */
export const currentParts = (held: HeldParts | undefined, client: DashboardClient, identity: string, attempt: number): readonly GoalPart[] =>
  held && held.client === client && held.identity === identity && held.attempt === attempt ? held.parts : WHOLE;
