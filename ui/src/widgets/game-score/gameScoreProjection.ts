import { dayLabel } from '../../charts/draw.ts';
import { liveKey } from '../../charts/live.ts';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import type { DeclaredFactionRun, HourRanges, TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { lookAt, type CountryLooksMap } from '../../families/table/looks.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableColumnSpec,
  type TableGroupSpec,
  type TableProjection,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED } from '../../families/table/settings.ts';
import { knownValue, notRecorded, valueFromLive, valueNumber, type Value } from '../../families/value.ts';
import type { GameScoreRow } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition.ts';
import { SCORE_LABELS, SCORE_PARTS, scoreHint, scorePart, validateGameScore, type GameScoreReply, type ScorePart } from './gameScoreData.ts';

export type FactionRun = DeclaredFactionRun;

export interface GameScoreProjectionOptions {
  readonly all: boolean;
  readonly chosenTags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly top: number | null;
  readonly by: ScorePart;
  readonly factions: boolean;
  readonly factionRuns: readonly FactionRun[] | null;
  readonly rankMarkers: boolean;
  readonly cellBars: boolean;
  readonly labels: CountryLooksMap;
}

const inRanges = (ranges: HourRanges | undefined, hour: number) => !!ranges?.some(([first, last]) => first <= hour && hour <= last);

function factionAt(runs: readonly FactionRun[] | null, tag: string, hour: number): TableGroupSpec {
  if (!runs) return { key: 'faction-not-recorded', label: 'Faction not known yet', order: 2 };
  for (const run of runs) {
    if (run.tag === tag && run.firstHour <= hour && hour <= run.lastHour) {
      const name = run.names.find(n => n.firstHour <= hour && hour <= run.lastHour)?.name ?? run.fallbackName;
      return { key: `faction:${run.id}`, label: name ?? 'Name not recorded', order: name === null ? 1 : 0, orderKey: BigInt(run.id) };
    }
  }
  return { key: 'no-faction', label: 'No faction', order: 3 };
}

export function gameScoreColumns(hints?: GameScoreReply['hints'], cellBars = false): TableColumnSpec[] {
  const numDisplay = cellBars ? 'faint' : 'plain';
  return [
    {
      key: 'country',
      label: SCORE_LABELS.country,
      as: 'flagName',
      offered: OFFERED.country,
      priority: 0,
      text: true,
      hideable: false,
      widest: ['United Kingdom'],
      hint: scoreHint(hints ? { hints } : undefined, 'country'),
    },
    ...SCORE_PARTS.map((part, index): TableColumnSpec => ({
      key: part,
      label: SCORE_LABELS[part],
      as: numDisplay,
      tone: 'blue',
      offered: OFFERED.number,
      priority: index + 1,
      numeric: true,
      format: { decimals: 0 },
      widest: ['99,999'],
      hint: scoreHint(hints ? { hints } : undefined, part),
    })),
  ];
}

export const gameScoreProjection: TableProjection<GameScoreProjectionOptions> = registerTableProjection(
  declareTableProjection<GameScoreReply, GameScoreRow, GameScoreProjectionOptions>({
    kind: 'game-score',
    validate: validateGameScore,
    rank: true,
    rankMarkers: options => options.rankMarkers,
    columns: (reply, options): readonly DeclarativeColumnSpec<GameScoreRow, GameScoreProjectionOptions>[] => {
      const specs = gameScoreColumns(reply.hints, options.cellBars);
      return [
        { ...specs[0]!, countryField: 'tag' },
        ...SCORE_PARTS.map((part, idx): DeclarativeColumnSpec<GameScoreRow, GameScoreProjectionOptions> => ({
          ...specs[idx + 1]!,
          country: 'tag',
          value: (row, live): Value => (row[part].missing !== null ? notRecorded(row[part].missing) : valueFromLive(live, liveKey(row.tag, part))),
        })),
      ];
    },
    rows: (reply, live, hour, options) => {
      if (reply.missing !== null) {
        return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      let filtered: GameScoreRow[];
      if (options.top != null) {
        const scored: { row: GameScoreRow; val: number }[] = [];
        for (const row of reply.rows) {
          if (row[options.by].missing !== null) continue;
          const v = valueNumber(valueFromLive(live, liveKey(row.tag, options.by)));
          if (v !== undefined) scored.push({ row, val: v });
        }
        scored.sort((a, b) => b.val - a.val || (a.row.tag < b.row.tag ? -1 : a.row.tag > b.row.tag ? 1 : 0));
        filtered = scored.slice(0, options.top).map(item => item.row);
      } else if (options.all) {
        filtered = reply.rows;
      } else if (options.membership) {
        filtered = reply.rows.filter(row => inRanges(options.membership![row.tag], hour));
      } else {
        const chosen = new Set(options.chosenTags);
        filtered = reply.rows.filter(row => chosen.has(row.tag));
      }
      if (!filtered.length) {
        return { rows: [], empty: `No country’s scores recorded on ${dayLabel(hour)}.`, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      return { rows: filtered, warnings: reply.warnings };
    },
    rowKey: row => row.tag,
    rowName: (row, hour, options) => lookAt(options.labels[row.tag], hour)?.name ?? row.tag,
    group: (row, hour, options) => (options.factions ? factionAt(options.factionRuns, row.tag, hour) : undefined),
    tooltipExtra: (row, reply, _live, hour, options): TooltipExtraRow[] => [
      { label: 'Faction', value: knownValue(factionAt(options.factionRuns, row.tag, hour).label), divider: true },
      ...reply.multipliers.map((multiplier, index): TooltipExtraRow => ({
        label: `${SCORE_LABELS[scorePart(multiplier.part)]} multiplier`,
        value: multiplier.raw === null ? notRecorded(multiplier.missing ?? 'Not recorded') : knownValue(formatRaw(multiplier.raw, 5, 5)),
        ...(index === 0 ? { divider: true } : {}),
      })),
    ],
  }),
);

export const gameScoreDeclaration: TableWidgetDeclaration<GameScoreProjectionOptions> = {
  family: 'table',
  kind: 'game-score',
  label: 'Game score',
  subject: 'game scores',
  emptyCampaign: 'Load a campaign to see its game scores.',
  icon: 'grid',
  optionFields: OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    factions: true,
    allowSlots: true,
    allTagsForLabels: true,
    requireGroupsWhen: (widget, options) => options.factions === true || (widget.countries.kind === 'group' && widget.countries.group.type !== 'top'),
  },
  source: {
    kind: 'panel',
    product: 'game-score',
    params: ctx => ({ branchId: ctx.branch.id }),
    validate: validateGameScore,
  },
  projection: gameScoreProjection,
  columns: ctx => gameScoreColumns(undefined, ctx.options.cellBars === true),
  settings: { kind: 'options', defaultSort: DEFAULT_SORT },
  projectionOptions: ctx => ({
    all: ctx.countries.all,
    chosenTags: ctx.countries.chosenTags,
    membership: ctx.countries.membership,
    top: ctx.countries.top,
    by: scorePart(ctx.options.topMeasure),
    factions: ctx.options.factions === true,
    factionRuns: ctx.countries.factions,
    rankMarkers: ctx.options.rankMarkers === true,
    cellBars: ctx.options.cellBars === true,
    labels: ctx.countries.labels,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
