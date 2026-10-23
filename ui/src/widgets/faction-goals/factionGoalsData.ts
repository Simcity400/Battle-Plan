import type { FactionGoalDefinition, FactionGoalsReply, FactionRefusalDetail } from '../../generated/dashboard';
import { runAt } from '../../charts/series.ts';
import { pendingAt } from '../../dashboard/preparedPrefix.ts';

export type GoalFilter = 'all' | 'inProgress' | 'completed';
export type GoalState = 'inProgress' | 'completed' | 'cancelled' | 'other';

/** A runtime goal as the table reads it; a faction-details goal is one. */
export interface HeldGoal {
  ordinal: number; definition: FactionGoalDefinition; status: number;
  progressPercent: number | null; progressRaw: string | null;
}

/** One hour's goal lists, from the goal history or that hour's faction details. */
export interface GoalView {
  goals: readonly HeldGoal[]; completed: readonly FactionGoalDefinition[]; cancelled: readonly FactionGoalDefinition[];
  goalReason: string | null; warnings: readonly string[]; refusals: readonly FactionRefusalDetail[];
}

/** What the goal history holds at an hour: a view, or why it holds none. */
export type GoalLookup =
  | { kind: 'held'; view: GoalView }
  /** Not prepared yet: the hour's own details answer it. */
  | { kind: 'pending' }
  /** Not held, or not readable: the hour's own details answer it too. */
  | { kind: 'unrecorded' }
  | { kind: 'absent' };

export const ABSENT_FACTION = 'Faction is absent at this hour';

/** Whether the hour's own faction details answer it: the history cannot, or its proven absence was retried. */
export const asksDetails = (lookup: GoalLookup | null, failed: boolean, retried: boolean) =>
  failed || lookup?.kind === 'pending' || lookup?.kind === 'unrecorded' || (lookup?.kind === 'absent' && retried);

export interface GoalRow {
  key: string; name: string; definition: FactionGoalDefinition;
  /** The native term token, the group's identity, and its interface name when content has one. */
  category: string | null; categoryName: string | null;
  state: GoalState; statusText: string;
  /** Held progress as a fraction, from the game's cached percentage; null when not reported. */
  progress: number | null; progressRaw: string | null;
  /** True for goals only known from the faction's completed or cancelled lists, which carry no dates. */
  history: boolean;
}

/** The native goal states, in the game's order; others keep their recorded number. */
const STATES: readonly [GoalState, string][] = [['inProgress', 'In progress'], ['completed', 'Completed'], ['cancelled', 'Cancelled'],
  ['inProgress', 'Completing'], ['inProgress', 'Cancelling']];

/** A goal's name: the resolved in-game name, or why it is not shown. */
export const goalName = (definition: FactionGoalDefinition) => definition.name || (definition.reason || !definition.token ? 'Goal name unavailable' : 'Unnamed goal');

const byText = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;

/** The goal history's lists at `hour`: a local lookup, with the hour's goal-text refusals counted as faction details count them. */
export function goalsAt(reply: FactionGoalsReply, hour: number): GoalLookup {
  if (pendingAt(reply, hour)) return { kind: 'pending' };
  const run = runAt(reply.runs, hour);
  const list = run && reply.lists[run.list];
  if (!list) return { kind: 'unrecorded' };
  if (list.absent) return { kind: 'absent' };
  const named = [...list.goals.map(goal => goal.definition), ...list.completed, ...list.cancelled];
  if (named.some(index => !reply.definitions[index])) return { kind: 'unrecorded' };
  const definition = (index: number) => reply.definitions[index];
  const goals = list.goals.map(goal => {
    const progress = runAt(reply.progress.find(lane => lane.ordinal === goal.ordinal)?.runs ?? [], hour);
    return { ordinal: goal.ordinal, definition: definition(goal.definition), status: goal.status,
      progressPercent: progress?.progressPercent ?? null, progressRaw: progress?.progressRaw ?? null };
  });
  const completed = list.completed.map(definition), cancelled = list.cancelled.map(definition);
  const reasons = new Map<string, number>();
  for (const shown of [...goals.map(goal => goal.definition), ...completed, ...cancelled])
    if (shown.reason) reasons.set(shown.reason, (reasons.get(shown.reason) ?? 0) + 1);
  const counted = [...reasons].sort(([a], [b]) => byText(a, b));
  return { kind: 'held', view: { goals, completed, cancelled, goalReason: list.reason,
    warnings: [...(list.reason ? [`Faction goals: ${list.reason}`] : []), ...counted.map(([reason, count]) => `${count} faction goal texts: ${reason}`),
      ...reply.warnings],
    refusals: counted.filter(([reason]) => reason.startsWith('Faction goal artwork')).map(([reason, count]) => ({ fact: 'goal artwork', count, reason })) } };
}

/** Every goal the faction holds at the hour, then its completed and cancelled lists, in native order. */
export function goalRows(data: GoalView): GoalRow[] {
  const base = (definition: FactionGoalDefinition) => ({ name: goalName(definition), definition, category: definition.category, categoryName: definition.categoryName });
  return [
    ...data.goals.map(goal => {
      const [state, statusText] = STATES[goal.status] ?? ['other', `State ${goal.status}`];
      return { key: `goal:${goal.ordinal}`, ...base(goal.definition), state, statusText,
        progress: goal.progressPercent == null || !Number.isFinite(goal.progressPercent) ? null : goal.progressPercent / 100,
        progressRaw: goal.progressRaw, history: false };
    }),
    ...data.completed.map((definition, index) => ({ key: `completed:${index}`, ...base(definition), state: 'completed' as const,
      statusText: 'Completed', progress: null, progressRaw: null, history: true })),
    ...data.cancelled.map((definition, index) => ({ key: `cancelled:${index}`, ...base(definition), state: 'cancelled' as const,
      statusText: 'Cancelled', progress: null, progressRaw: null, history: true })),
  ];
}

/** In progress hides finished goals; Completed shows only completed ones; cancelled goals show under All. */
export function filterGoals(rows: readonly GoalRow[], filter: GoalFilter): GoalRow[] {
  return filter === 'all' ? [...rows] : rows.filter(row => filter === 'completed' ? row.state === 'completed' : row.state === 'inProgress' || row.state === 'other');
}

/** Groups in the order their terms first appear among the faction's goals. */
export function termOrder(rows: readonly GoalRow[]): Map<string, number> {
  const order = new Map<string, number>();
  for (const row of rows) { const key = row.category ?? ''; if (!order.has(key)) order.set(key, order.size); }
  return order;
}

export const completedCount = (rows: readonly GoalRow[]) => rows.filter(row => row.state === 'completed').length;
