import { liveKey } from '../../charts/live.ts';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import {
  holderTagAt,
  nextHoldersChange,
  nextMembershipChange,
  nextRangesChange,
  type DeclaredFactionRun,
  type HolderRun,
  type HourRanges,
  type TableWidgetDeclaration,
} from '../../families/table/declaration.ts';
import { inHourRanges, nextFactionsChange } from '../../families/holders.ts';
import { lookAt, type CountryLooksMap } from '../../families/looks.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableColumnSpec,
  type TableGroupSpec,
  type TableProjection,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, valueFromLive, type Value } from '../../families/value.ts';
import type { GameScoreRow } from '../../generated/dashboard';
import { OPTIONS, TABLE_BASE } from './definition.ts';
import { SCORE_LABELS, SCORE_PARTS, scoreHint, scorePart, validateGameScore, type GameScoreReply, type ScorePart } from './gameScoreData.ts';

export type FactionRun = DeclaredFactionRun;

export interface GameScoreProjectionOptions {
  readonly all: boolean;
  readonly chosenTags?: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
  readonly top: number | null;
  readonly by: ScorePart;
  readonly factions: boolean;
  readonly factionRuns: readonly FactionRun[] | null;
  readonly factionUnobserved?: HourRanges;
  readonly factionIncomplete?: HourRanges;
  readonly rankMarkers: boolean;
  readonly labels: CountryLooksMap;
}

interface ResolvedFaction extends TableGroupSpec {
  readonly missingReason?: string;
}

const factionRunsByTag = new WeakMap<readonly FactionRun[], Map<string, FactionRun[]>>();

function runsForTag(runs: readonly FactionRun[], tag: string): readonly FactionRun[] | undefined {
  let byTag = factionRunsByTag.get(runs);
  if (!byTag) {
    byTag = new Map();
    for (let i = 0; i < runs.length; i++) {
      const run = runs[i]!;
      let list = byTag.get(run.tag);
      if (!list) byTag.set(run.tag, list = [run]);
      else list.push(run);
    }
    factionRunsByTag.set(runs, byTag);
  }
  return byTag.get(tag);
}

function factionAt(
  runs: readonly FactionRun[] | null,
  tag: string,
  hour: number,
  unobserved?: HourRanges,
  incomplete?: HourRanges,
): ResolvedFaction {
  if (!runs) return { key: 'faction-not-recorded', label: 'Faction not known yet', order: 2, missingReason: 'Faction membership not loaded' };
  if (inHourRanges(unobserved, hour)) {
    return { key: 'faction-not-recorded', label: 'Faction not recorded', order: 2, missingReason: 'Faction membership was not recorded at this hour' };
  }
  const tagRuns = runsForTag(runs, tag);
  if (tagRuns) {
    for (let i = 0; i < tagRuns.length; i++) {
      const run = tagRuns[i]!;
      if (run.firstHour <= hour && hour <= run.lastHour) {
        const entry = run.names.find(n => n.firstHour <= hour && hour <= n.lastHour);
        const name = entry?.name && entry.name.trim() ? entry.name : null;
        return {
          key: `faction:${run.id}`,
          label: name ?? 'Name not recorded',
          order: name === null ? 1 : 0,
          orderKey: BigInt(run.id),
          ...(name === null ? { missingReason: 'Faction name not recorded at this hour' } : {}),
        };
      }
    }
  }
  if (inHourRanges(incomplete, hour)) {
    return { key: 'faction-not-recorded', label: 'Faction not recorded', order: 2, missingReason: 'Faction roster was incomplete at this hour' };
  }
  return { key: 'no-faction', label: 'No faction', order: 3 };
}

function nextGameScoreChange(hour: number, options: GameScoreProjectionOptions): number {
  return Math.min(
    nextHoldersChange(options.holders, hour),
    nextRangesChange(options.unobserved, hour),
    nextMembershipChange(options.membership, hour),
    nextFactionsChange(options.factionRuns, hour, options.factionUnobserved, options.factionIncomplete),
  );
}

export function gameScoreColumns(hints?: GameScoreReply['hints']): TableColumnSpec[] {
  return [
    {
      key: 'country',
      label: SCORE_LABELS.country,
      as: 'flagName',
      showNamesDefault: true,
      pinned: true,
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
      as: 'faint',
      barEligible: true,
      tone: 'country',
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
    nextChange: (readView, options) => nextGameScoreChange(readView.hour, options),
    incrementalLive: options => options.top == null,
    rank: true,
    rankMarkers: options => options.rankMarkers,
    columns: (reply): readonly DeclarativeColumnSpec<GameScoreRow, GameScoreProjectionOptions>[] => {
      const specs = gameScoreColumns(reply.hints);
      return [
        { ...specs[0]!, countryField: 'tag' },
        ...SCORE_PARTS.map((part, idx): DeclarativeColumnSpec<GameScoreRow, GameScoreProjectionOptions> => ({
          ...specs[idx + 1]!,
          country: 'tag',
          value: (row, live): Value => (row[part].missing !== null ? notRecorded(row[part].missing) : valueFromLive(live, liveKey(row.tag, part))),
        })),
      ];
    },
    rows: (reply, _live, hour, options) => {
      if (reply.missing !== null) {
        return { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
      }
      if (options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
        return { rows: [], empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable', warnings: reply.warnings };
      }
      let filtered: GameScoreRow[];
      if (options.top != null) {
        if (!reply.top) return { rows: [], empty: 'The top countries were not answered for this reading.', emptyKind: 'unavailable', warnings: reply.warnings };
        if (reply.top.missing !== null) return { rows: [], empty: reply.top.missing, emptyKind: 'unavailable', warnings: reply.warnings };
        const byTag = new Map(reply.rows.map(row => [row.tag, row]));
        filtered = reply.top.tags.flatMap(tag => byTag.get(tag) ?? []);
      } else if (options.all) {
        filtered = reply.rows;
      } else if (options.membership) {
        filtered = reply.rows.filter(row => inHourRanges(options.membership![row.tag], hour));
      } else if (options.holders) {
        const activeTags = new Set<string>();
        for (const runs of Object.values(options.holders)) {
          const tag = holderTagAt(runs, hour);
          if (tag !== null) activeTags.add(tag);
        }
        filtered = reply.rows.filter(row => activeTags.has(row.tag));
      } else {
        const chosen = new Set(options.chosenTags ?? []);
        filtered = reply.rows.filter(row => chosen.has(row.tag));
      }
      if (!filtered.length) {
        return { rows: [], empty: 'No country’s scores recorded at this time.', emptyKind: 'unavailable', warnings: reply.warnings };
      }
      return { rows: filtered, warnings: reply.warnings };
    },
    rowKey: row => row.tag,
    rowName: (row, hour, options) => lookAt(options.labels[row.tag], hour)?.name ?? row.tag,
    group: (row, hour, options) => {
      if (!options.factions) return undefined;
      const f = factionAt(options.factionRuns, row.tag, hour, options.factionUnobserved, options.factionIncomplete);
      return { key: f.key, label: f.label, order: f.order, ...(f.orderKey !== undefined ? { orderKey: f.orderKey } : {}) };
    },
    tooltipExtra: (row, reply, _live, hour, options): TooltipExtraRow[] => {
      const f = factionAt(options.factionRuns, row.tag, hour, options.factionUnobserved, options.factionIncomplete);
      return [
        { label: 'Faction', value: f.missingReason ? notRecorded(f.missingReason) : knownValue(f.label), divider: true },
        ...reply.multipliers.map((multiplier, index): TooltipExtraRow => ({
          label: `${SCORE_LABELS[scorePart(multiplier.part)]} multiplier`,
          value: multiplier.raw === null ? notRecorded(multiplier.missing ?? 'Not recorded') : knownValue(formatRaw(multiplier.raw, 5, 5)),
          ...(index === 0 ? { divider: true } : {}),
        })),
      ];
    },
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
    colours: true,
    factions: true,
    allowSlots: true,
    allTagsForLabels: true,
    requireGroupsWhen: (widget, options) => options.factions === true || (widget.countries.kind === 'group' && widget.countries.group.type !== 'top'),
  },
  sources: ctx => {
    const top = ctx.countries.top;
    const asked = top != null ? { branchId: ctx.branch.id, top: { count: top, by: scorePart(ctx.options.topMeasure) } } : { branchId: ctx.branch.id };
    return {
      owner: `game-score:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: top != null ? { mode: 'population', form: 'top', limit: top } : { mode: 'population', form: 'all' },
          product: 'game-score',
          followQuery: { panel: 'game-score', ...asked },
          params: asked,
          validate: validateGameScore,
        },
      }],
    };
  },
  projection: gameScoreProjection,
  columns: () => gameScoreColumns(),
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({
    all: ctx.countries.all,
    holders: ctx.countries.holders,
    membership: ctx.countries.membership,
    unobserved: ctx.countries.unobserved,
    top: ctx.countries.top,
    by: scorePart(ctx.options.topMeasure),
    factions: ctx.options.factions === true,
    factionRuns: ctx.countries.factions,
    factionUnobserved: ctx.countries.factionUnobserved,
    factionIncomplete: ctx.countries.factionIncomplete,
    rankMarkers: ctx.options.rankMarkers === true,
    labels: ctx.countries.labels,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
