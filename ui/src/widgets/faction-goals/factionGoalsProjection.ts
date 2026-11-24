import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection, type TooltipExtraRow } from '../../families/table/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, textValue, notApplicable, notRecorded, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import type { CountryFactionGoalsReply, CountryGoal, FactionGoalDefinition, GoalFilter, GoalState } from '../../generated/dashboard';
import type { StatusMarkKind } from '../../draw/kit.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';

const NO_OPTIONS: Record<string, never> = {};

interface GoalRow {
  readonly goal: CountryGoal;
  readonly definition: FactionGoalDefinition;
  readonly icon: string | undefined;
}

const STATE_TEXT: Readonly<Record<Exclude<GoalState, 'unrecognised'>, string>> = {
  inProgress: 'In progress', completing: 'Completing', cancelling: 'Cancelling', completed: 'Completed', cancelled: 'Cancelled',
};
const STATE_TONE: Readonly<Record<GoalState, string>> = { inProgress: 'blue', completing: 'blue', cancelling: 'blue', completed: 'green', cancelled: 'grey', unrecognised: 'grey' };
const STATE_MARK: Readonly<Record<GoalState, StatusMarkKind>> = { inProgress: 'running', completing: 'running', cancelling: 'running', completed: 'succeeded', cancelled: 'ended', unrecognised: 'ended' };

function stateValue(goal: CountryGoal): Value<string> {
  return goal.state === 'unrecognised'
    ? notRecorded(`State ${goal.unrecognisedStatus ?? '?'} is not one this version recognises`)
    : knownValue(STATE_TEXT[goal.state]);
}

export function goalFilter(raw: unknown): GoalFilter {
  return raw === 'inProgress' || raw === 'completed' ? raw : 'all';
}
const UNMEASURED = 92233720124192;
const PERCENT = { decimals: 0, maxDecimals: 1, percent: true } as const;

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);

const optionalText = (value: unknown) => value === null || typeof value === 'string';

export function validateCountryFactionGoals(value: unknown): CountryFactionGoalsReply {
  if (!object(value) || !optionalText(value.membershipMissing) || !optionalText(value.goalReason) || !optionalText(value.faction) || !Array.isArray(value.warnings)
    || !Array.isArray(value.definitions) || !Array.isArray(value.goals)
    || !(value.goals as unknown[]).every(goal => object(goal) && typeof goal.key === 'string' && Number.isSafeInteger(goal.definition)
      && (goal.definition as number) < (value.definitions as unknown[]).length && typeof goal.state === 'string')) throw new Error('The faction goals reply is malformed.');
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

function goalRows(reply: CountryFactionGoalsReply, readView: BoundedReadView): GoalRow[] {
  return reply.goals.map(goal => {
    const definition = reply.definitions[goal.definition]!;
    return { goal, definition, icon: goalIcon(definition, readView) };
  });
}

const [ICON, GOAL, PROGRESS, STATE] = COLUMNS;

const GOAL_COLUMNS: readonly DeclarativeColumnSpec<GoalRow, Record<string, never>>[] = [
  { ...ICON!, priority: 3, sortable: false, cellKind: 'kind', iconSlot: true, icon: row => row.icon, extra: row => ({ artShape: 'square', ...(row.definition.name ? { artName: row.definition.name } : {}) }),
    value: row => goalName(row.definition) },
  { ...GOAL!, priority: 0, text: true, cellKind: 'item', hideable: false, widest: ['Conquest and Domination'], value: row => goalName(row.definition) },
  { ...PROGRESS!, priority: 1, numeric: true, format: PERCENT, widest: ['100%'],
    value: (row, live) => goalProgress(live, row.goal.ordinal, row.goal.state),
    bar: (row, live) => ({ fraction: fraction(goalProgress(live, row.goal.ordinal, row.goal.state)) }) },
  { ...STATE!, priority: 2, text: true, widest: ['In progress'], tone: 'blue', status: () => 'dot', cellTone: row => STATE_TONE[row.goal.state], cellKind: 'status', extra: row => ({ statusKind: STATE_MARK[row.goal.state] }), value: row => stateValue(row.goal) },
];

export const factionGoalsProjection: TableProjection<Record<string, never>> = registerFamilyProjection(
  declareTableProjection<CountryFactionGoalsReply, GoalRow, Record<string, never>>({
    kind: 'faction-goals',
    validate: validateCountryFactionGoals,
    columns: GOAL_COLUMNS,
    rows: (reply, _live, _hour, _options, readView) => {
      const warnings = reply.warnings;
      if (reply.membershipMissing !== null) return { rows: [], empty: reply.membershipMissing, emptyKind: 'unavailable', warnings };
      if (reply.faction === null) return { rows: [], empty: 'In no faction at this date', emptyKind: 'empty', warnings };
      if (reply.goalReason !== null) return { rows: [], empty: reply.goalReason, emptyKind: 'unavailable', warnings };
      const rows = goalRows(reply, readView);
      return { rows, ...(!rows.length ? { empty: 'No faction goals', emptyKind: 'empty' as const } : {}), warnings };
    },
    rowKey: row => row.goal.key,
    rowName: row => row.definition.name ?? 'Goal name unavailable',
    tooltipExtra: (row): TooltipExtraRow[] => [
      ...(row.definition.categoryName ? [{ label: 'Term', value: knownValue(row.definition.categoryName), divider: true }] : []),
      ...(row.goal.ordinal === null ? [{ label: 'Held in', value: knownValue(row.goal.state === 'completed' ? 'The faction’s completed goals' : 'The faction’s cancelled goals') }] : []),
    ],
    tooltipNote: row => row.definition.description ?? undefined,
  }),
);

export const factionGoalsDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'faction-goals',
  label: 'Faction goals',
  subject: 'faction goals',
  emptyCampaign: 'Load a campaign to see faction goals.',
  icon: 'globe',
  countries: { mode: 'single', choose: 'Choose a country to see its faction’s goals.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, country: countryFromKey(ctx.countryTag), goals: goalFilter(ctx.options.goals) } : null;
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
  projectionOptions: () => NO_OPTIONS,
};
