import type { FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';

export type GoalFilter = 'all' | 'inProgress' | 'completed';
export type GoalState = 'inProgress' | 'completed' | 'cancelled' | 'other';

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

/** Every goal the faction holds at the hour, then its completed and cancelled lists, in native order. */
export function goalRows(data: FactionDetailReply): GoalRow[] {
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
