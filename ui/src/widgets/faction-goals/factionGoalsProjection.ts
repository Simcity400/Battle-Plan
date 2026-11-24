import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateFactionDetails } from '../../dashboard/factionDetails.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, textValue, notApplicable, notRecorded, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import type { CountryFactionGoalsReply, FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';
import type { StatusMarkKind } from '../../draw/kit.ts';
import { COLUMNS, TABLE_BASE, OPTIONS } from './definition.ts';

export type GoalFilter = 'all' | 'inProgress' | 'completed';
export interface FactionGoalsOptions { readonly goals: GoalFilter }

type GoalState = 'inProgress' | 'completed' | 'cancelled';

interface GoalRow {
  readonly key: string;
  readonly definition: FactionGoalDefinition;
  readonly icon: string | undefined;
  readonly state: GoalState;
  readonly stateText: string;
  readonly ordinal: number | null;
}

const NATIVE_STATES: readonly (readonly [GoalState, string])[] = [
  ['inProgress', 'In progress'], ['completed', 'Completed'], ['cancelled', 'Cancelled'], ['inProgress', 'Completing'], ['inProgress', 'Cancelling'],
];
const STATE_TONE: Readonly<Record<GoalState, string>> = { inProgress: 'blue', completed: 'green', cancelled: 'grey' };
const STATE_MARK: Readonly<Record<GoalState, StatusMarkKind>> = { inProgress: 'running', completed: 'succeeded', cancelled: 'ended' };
const UNMEASURED = 92233720124192;
const PERCENT = { decimals: 0, maxDecimals: 1, percent: true } as const;

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

export function validateCountryFactionGoals(value: unknown): CountryFactionGoalsReply {
  if (!object(value) || !(value.membershipMissing === null || typeof value.membershipMissing === 'string') || !Array.isArray(value.warnings)
    || !(value.faction === null || object(value.faction))) throw new Error('The faction goals reply is malformed.');
  if (value.faction !== null) validateFactionDetails(value.faction);
  return value as unknown as CountryFactionGoalsReply;
}

export function goalProgress(live: Parameters<typeof valueFromLive>[0], ordinal: number | null, state: GoalState): Value<number> {
  if (ordinal === null) return notApplicable(state === 'completed' ? 'A completed goal holds no progress' : 'A cancelled goal holds no progress');
  const raw = valueFromLive(live, liveKey(`goal:${ordinal}`, 'progress'));
  if (raw.kind !== 'known' && raw.kind !== 'zero') return raw.kind === 'partial' ? notRecorded(raw.reason) : raw as Value<never>;
  const number = Number(raw.value);
  if (number === UNMEASURED) return notApplicable('This goal has no progress measure');
  if (!Number.isFinite(number) || Math.abs(number) >= 2 ** 53) return notRecorded('Goal progress not recorded');
  return number === 0 ? zeroValue(0) : knownValue(number / 1000);
}

const fraction = (value: Value<number>): Value<number> => value.kind === 'known' ? knownValue(Math.max(0, Math.min(1, value.value / 100))) : value;

const goalName = (definition: FactionGoalDefinition): Value<string> =>
  textValue(definition.name, definition.reason ?? 'Goal name unavailable');

function goalIcon(definition: FactionGoalDefinition, readView: BoundedReadView): string | undefined {
  if (definition.icon === null) return undefined;
  const art = readView.art({ kind: 'url', url: definition.icon });
  return art.status === 'ready' ? art.url : undefined;
}

function goalRows(detail: FactionDetailReply, readView: BoundedReadView, filter: GoalFilter): GoalRow[] {
  const row = (index: number, key: string, state: GoalState, stateText: string, ordinal: number | null): GoalRow => {
    const definition = detail.definitions[index]!;
    return { key, definition, icon: goalIcon(definition, readView), state, stateText, ordinal };
  };
  const rows = [
    ...detail.goals.map(goal => {
      const [state, text] = NATIVE_STATES[goal.status] ?? (['inProgress', `State ${goal.status}`] as const);
      return row(goal.definition, `goal:${goal.ordinal}`, state, text, goal.ordinal);
    }),
    ...detail.completed.map((index, at) => row(index, `completed:${at}`, 'completed', 'Completed', null)),
    ...detail.cancelled.map((index, at) => row(index, `cancelled:${at}`, 'cancelled', 'Cancelled', null)),
  ];
  return filter === 'all' ? rows : rows.filter(item => filter === 'completed' ? item.state === 'completed' : item.state === 'inProgress');
}

const [ICON, GOAL, PROGRESS, STATE] = COLUMNS;

const GOAL_COLUMNS: readonly DeclarativeColumnSpec<GoalRow, FactionGoalsOptions>[] = [
  { ...ICON!, priority: 3, sortable: false, cellKind: 'kind', iconSlot: true, icon: row => row.icon, extra: row => ({ artShape: 'square', ...(row.definition.name ? { artName: row.definition.name } : {}) }),
    value: row => goalName(row.definition) },
  { ...GOAL!, priority: 0, text: true, hideable: false, widest: ['Conquest and Domination'], value: row => goalName(row.definition) },
  { ...PROGRESS!, priority: 1, numeric: true, format: PERCENT, widest: ['100%'],
    value: (row, live) => goalProgress(live, row.ordinal, row.state),
    bar: (row, live) => ({ fraction: fraction(goalProgress(live, row.ordinal, row.state)) }) },
  { ...STATE!, priority: 2, text: true, widest: ['In progress'], tone: 'blue', status: () => 'dot', cellTone: row => STATE_TONE[row.state], cellKind: 'status', extra: row => ({ statusKind: STATE_MARK[row.state] }), value: row => knownValue(row.stateText) },
];

export const factionGoalsProjection: TableProjection<FactionGoalsOptions> = registerTableProjection(
  declareTableProjection<CountryFactionGoalsReply, GoalRow, FactionGoalsOptions>({
    kind: 'faction-goals',
    validate: validateCountryFactionGoals,
    columns: GOAL_COLUMNS,
    rows: (reply, _live, _hour, options, readView) => {
      if (reply.membershipMissing !== null) return { rows: [], empty: reply.membershipMissing, emptyKind: 'unavailable', warnings: reply.warnings };
      const detail = reply.faction;
      if (detail === null) return { rows: [], empty: 'In no faction at this date', emptyKind: 'empty', warnings: reply.warnings };
      const warnings = [...reply.warnings, ...detail.warnings];
      if (detail.goalReason !== null) return { rows: [], empty: detail.goalReason, emptyKind: 'unavailable', warnings };
      const rows = goalRows(detail, readView, options.goals);
      return { rows, ...(!rows.length ? { empty: 'No faction goals', emptyKind: 'empty' as const } : {}), warnings };
    },
    rowKey: row => row.key,
    rowName: row => row.definition.name ?? 'Goal name unavailable',
    tooltipExtra: (row): TooltipExtraRow[] => [
      ...(row.definition.categoryName ? [{ label: 'Term', value: knownValue(row.definition.categoryName), divider: true }] : []),
      ...(row.ordinal === null ? [{ label: 'Held in', value: knownValue(row.state === 'completed' ? 'The faction’s completed goals' : 'The faction’s cancelled goals') }] : []),
    ],
    tooltipNote: row => row.definition.description ?? undefined,
  }),
);

const goalsCache = new Map<GoalFilter, FactionGoalsOptions>();
const goalsOption = (raw: unknown): FactionGoalsOptions => {
  const goals: GoalFilter = raw === 'inProgress' || raw === 'completed' ? raw : 'all';
  let held = goalsCache.get(goals);
  if (!held) goalsCache.set(goals, held = { goals });
  return held;
};

export const factionGoalsDeclaration: TableWidgetDeclaration<FactionGoalsOptions> = {
  family: 'table',
  kind: 'faction-goals',
  label: 'Faction goals',
  subject: 'faction goals',
  emptyCampaign: 'Load a campaign to see faction goals.',
  icon: 'target',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its faction’s goals.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag) } : null;
    return {
      owner: `faction-goals:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'panel',
              coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
              product: 'country-faction-goals',
              followQuery: { panel: 'country-faction-goals', ...params },
              params,
              validate: validateCountryFactionGoals,
            },
          }]
        : [],
    };
  },
  projection: factionGoalsProjection,
  columns: GOAL_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => goalsOption(ctx.options.goals),
};
