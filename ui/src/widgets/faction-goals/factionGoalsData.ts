import { FIXED5_MAX, goalProgress } from '../../dashboard/factionDetails.ts';
import type { GoalProgress } from '../../dashboard/factionDetails';
import type { LiveValues } from '../../charts/live';
import type { SortValue } from '../../charts/table/logic';
import type { FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';

export type GoalFilter = 'all' | 'inProgress' | 'completed';
export type GoalState = 'inProgress' | 'completed' | 'cancelled' | 'other';

export interface GoalRow {
  key: string; name: string; definition: FactionGoalDefinition;
  category: string | null; categoryName: string | null;
  state: GoalState; statusText: string;
  ordinal: number | null;
  history: boolean;
}

const STATES: readonly [GoalState, string][] = [['inProgress', 'In progress'], ['completed', 'Completed'], ['cancelled', 'Cancelled'],
  ['inProgress', 'Completing'], ['inProgress', 'Cancelling']];

export const goalName = (definition: FactionGoalDefinition) => definition.name || (definition.reason || !definition.token ? 'Goal name unavailable' : 'Unnamed goal');

export function goalRows(data: FactionDetailReply): GoalRow[] {
  const base = (index: number) => {
    const definition = data.definitions[index];
    return { name: goalName(definition), definition, category: definition.category, categoryName: definition.categoryName };
  };
  return [
    ...data.goals.map(goal => {
      const [state, statusText] = STATES[goal.status] ?? ['other', `State ${goal.status}`];
      return { key: `goal:${goal.ordinal}`, ...base(goal.definition), state, statusText, ordinal: goal.ordinal, history: false };
    }),
    ...data.completed.map((index, at) => ({ key: `completed:${at}`, ...base(index), state: 'completed' as const, statusText: 'Completed', ordinal: null, history: true })),
    ...data.cancelled.map((index, at) => ({ key: `cancelled:${at}`, ...base(index), state: 'cancelled' as const, statusText: 'Cancelled', ordinal: null, history: true })),
  ];
}

export function rowProgress(values: LiveValues<number>, row: Pick<GoalRow, 'ordinal'>): GoalProgress {
  return row.ordinal === null ? { kind: 'missing', reason: 'Not recorded' } : goalProgress(values, row.ordinal);
}

export function progressText(progress: GoalProgress, digits: number): string {
  return progress.kind === 'percent' ? `${progress.percent.toFixed(digits)}%` : progress.kind === 'unmeasured' ? 'No progress measure' : progress.reason;
}

export function goalTableValues(values: LiveValues<number>): LiveValues<SortValue> {
  return {
    get: key => {
      const raw = values.get(key);
      return raw === undefined || raw === FIXED5_MAX || !Number.isFinite(raw) ? undefined : raw / 100_000;
    },
    missing: key => values.get(key) === FIXED5_MAX ? 'No progress measure' : values.missing(key) || 'Not recorded',
  };
}

export function filterGoals(rows: readonly GoalRow[], filter: GoalFilter): GoalRow[] {
  return filter === 'all' ? [...rows] : rows.filter(row => filter === 'completed' ? row.state === 'completed' : row.state === 'inProgress' || row.state === 'other');
}

export function termOrder(rows: readonly GoalRow[]): Map<string, number> {
  const order = new Map<string, number>();
  for (const row of rows) { const key = row.category ?? ''; if (!order.has(key)) order.set(key, order.size); }
  return order;
}

export const completedCount = (rows: readonly GoalRow[]) => rows.filter(row => row.state === 'completed').length;

export function goalDiagnostics(data: FactionDetailReply, rows: readonly GoalRow[]): string[] {
  const unnamed = rows.filter(row => row.definition.reason).length;
  const unnamedTerms = rows.filter(row => row.category && !row.categoryName).length;
  return [...data.warnings, ...data.refusals.map(refusal => `${refusal.fact} (${refusal.count}): ${refusal.reason}`),
    ...(data.goalReason ? [`Goals unavailable: ${data.goalReason}`] : []),
    ...(unnamed ? [`${unnamed} goal ${unnamed === 1 ? 'name is' : 'names are'} unavailable.`] : []),
    ...(unnamedTerms ? [`${unnamedTerms} goal ${unnamedTerms === 1 ? 'term has' : 'terms have'} no name in the recorded content.`] : [])];
}
