import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { countryFromKey, countryKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet, setToken } from '../../dashboard/options.ts';
import type { FamilySources } from '../../families/contract.ts';
import {
  holderTagAt,
  nextHoldersChange,
  nextMembershipChange,
  nextRangesChange,
  type DeclaredFollowItem,
  type DeclaredTableContext,
  type HolderRun,
  type HourRanges,
  type TableWidgetDeclaration,
} from '../../families/table/declaration.ts';
import { lookAt, type CountryLooksMap } from '../../families/table/looks.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { encodeTableSettings, OFFERED, TABLE_SETTINGS_KEY, tableSettingsOf } from '../../families/table/settings.ts';
import {
  knownValue,
  notRecorded,
  partialValue,
  valueFromNamedAnswer,
  valueFromRawBigInt,
  type NumericScalar,
  type Value,
} from '../../families/value.ts';
import { defaultWidestSamples, type ValueFormat } from '../../families/valueFormat.ts';
import type { ScoreboardColumn, ScoreboardColumnsReply, ScoreboardReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS, TABLE_BASE } from './definition.ts';
import {
  partialSource,
  rankedTags,
  rankingColumn,
  rankingDescending,
  scoreboardColumnNote,
  scoreboardParams,
  scoreboardSort,
  selectedColumns,
  validateColumnsReply,
  validateScoreboardReply,
} from './scoreboardData.ts';

export const RANK_SOURCE = 'top:rank';

export interface ScoreboardProjectionOptions {
  readonly shown: readonly ScoreboardColumn[];
  readonly candidates: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
  readonly top: number | null;
  readonly rankMarkers: boolean;
  readonly looks: CountryLooksMap;
}

const columnFormat = (column: ScoreboardColumn): ValueFormat =>
  ({ decimals: column.decimals, percent: column.unit === 'percent' });

export function scoreboardColumnsSpec(shown: readonly ScoreboardColumn[], top: number | null): TableColumnSpec[] {
  return [
    {
      key: 'country',
      label: 'Country',
      as: 'flagName',
      offered: OFFERED.country,
      priority: 0,
      text: true,
      hideable: false,
      widest: ['United Kingdom'],
      hint: top != null ? 'Top countries, sorted by name.' : undefined,
    },
    ...shown.map((column, index): TableColumnSpec => {
      const format = columnFormat(column);
      return {
        key: column.key,
        label: column.label,
        as: 'faint',
        tone: 'blue',
        offered: OFFERED.number,
        priority: index + 1,
        numeric: true,
        format,
        widest: defaultWidestSamples(format),
        hideable: shown.length > 1,
        hint: scoreboardColumnNote(column),
      };
    }),
  ];
}

function countryNameValue(tag: string, looks: CountryLooksMap, hour: number) {
  const runs = looks[tag];
  const look = lookAt(runs, hour);
  if (look && look.name && look.name.trim()) return { value: knownValue(look.name), name: look.name, runs };
  return { value: notRecorded(look?.status ?? 'Country name unavailable'), name: tag, runs };
}

function scoreboardMissingReason(column: ScoreboardColumn, displayTag: string, rawReason: string | null | undefined): string {
  const family = column.family;
  if (!rawReason || rawReason === 'Not in the answer') {
    return `${displayTag}'s ${column.label} was not recorded at this hour`;
  }
  if (family && (
    rawReason.endsWith(` holds no row of ${family.kind} at this hour`)
    || rawReason.endsWith(`'s ${family.measure} was not recorded at this hour`)
  )) {
    return `${displayTag}'s ${column.label} was not recorded at this hour`;
  }
  return rawReason;
}

export const scoreboardProjection: TableProjection<ScoreboardProjectionOptions> = registerTableProjection({
  kind: 'scoreboard',
  family: 'table',
  nextChange(_sources, hour, options) {
    return Math.min(
      nextHoldersChange(options.holders, hour),
      nextRangesChange(options.unobserved, hour),
      nextMembershipChange(options.membership, hour),
    );
  },
  project(sources: FamilySources, hour: number, options: ScoreboardProjectionOptions): TableModel {
    const columns = scoreboardColumnsSpec(options.shown, options.top);
    const panel = sources.panels.get('main');

    if (panel !== undefined) {
      const reply: ScoreboardReply = validateScoreboardReply(panel, options.shown.length);
      if (reply.missing !== null) {
        return { columns, rows: [], hour: reply.hour, empty: reply.missing, emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers, warnings: reply.warnings };
      }
      if (options.top == null && options.unobserved?.some(([first, last]) => first <= reply.hour && reply.hour <= last)) {
        return { columns, rows: [], hour: reply.hour, empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers, warnings: reply.warnings };
      }
      const rows: TableRow[] = reply.rows.map(row => {
        const key = countryKey(row.country);
        const identity = countryNameValue(row.tag, options.looks, reply.hour);
        const cells: Record<string, TableCell> = {
          country: { value: identity.value, iconSlot: true, country: row.tag, ...(identity.runs ? { looks: identity.runs } : {}) },
        };
        let hasPartial = false;
        options.shown.forEach((column, at) => {
          const rawCell = row.cells[at]!;
          const format = columnFormat(column);
          const value: Value<bigint> = !column.available
            ? notRecorded(column.reason ?? 'Unavailable')
            : valueFromRawBigInt(
              rawCell.valueRaw,
              { kind: 'notRecorded', reason: scoreboardMissingReason(column, row.tag, rawCell.missing ?? column.reason) },
              { partial: rawCell.partial ? 'Some inputs were not recorded at this hour' : null },
            );
          if (value.kind === 'partial') hasPartial = true;
          cells[column.key] = { value, format, country: row.tag };
        });
        return {
          key,
          name: identity.name,
          cells,
          ...(hasPartial ? { tooltipNote: 'Partial values omit unrecorded inputs at this hour.' } : {}),
        };
      });
      return {
        columns,
        rows,
        hour: reply.hour,
        rank: true,
        rankMarkers: options.rankMarkers,
        warnings: reply.warnings,
        ...(!rows.length ? { empty: 'No countries available for this selection.', emptyKind: 'unavailable' } : {}),
      };
    }

    const rankAnswer = options.top != null ? sources.answers.get(RANK_SOURCE) : undefined;
    if (options.top != null) {
      if (!rankAnswer) return { columns, rows: [], hour, rank: true, rankMarkers: options.rankMarkers };
      if (rankAnswer.kind === 'notRecorded') {
        return { columns, rows: [], hour, empty: rankAnswer.reason, emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers };
      }
    } else if (options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return { columns, rows: [], hour, empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers };
    }

    const activeEntries: { readonly key: string; readonly displayTag: string; readonly wireName: string }[] = [];
    if (options.top != null && rankAnswer && rankAnswer.kind !== 'notRecorded') {
      for (const tag of rankedTags(rankAnswer)) {
        activeEntries.push({ key: tag, displayTag: tag, wireName: tag });
      }
    } else {
      for (const candidate of options.candidates) {
        const datedTag = options.holders
          ? holderTagAt(options.holders[candidate], hour)
          : (isSlotKey(candidate) ? null : candidate);
        if (datedTag === null) continue;
        if (options.membership) {
          const inMember = options.membership[candidate]?.some(([first, last]) => first <= hour && hour <= last)
            || options.membership[datedTag]?.some(([first, last]) => first <= hour && hour <= last);
          if (!inMember) continue;
        }
        activeEntries.push({ key: candidate, displayTag: datedTag, wireName: candidate });
      }
    }

    const rows: TableRow[] = activeEntries.map(({ key, displayTag, wireName }) => {
      const identity = countryNameValue(displayTag, options.looks, hour);
      const cells: Record<string, TableCell> = {
        country: { value: identity.value, iconSlot: true, country: displayTag, ...(identity.runs ? { looks: identity.runs } : {}) },
      };
      let hasPartial = false;
      for (const column of options.shown) {
        const format = columnFormat(column);
        if (!column.available || !column.family) {
          cells[column.key] = { value: notRecorded(column.reason ?? 'Unavailable'), format, country: displayTag };
          continue;
        }
        const answer = sources.answers.get(column.key);
        const partialAns = column.partial ? sources.answers.get(partialSource(column.key)) : undefined;
        let isPartialCell = false;
        if (partialAns) {
          const mark = valueFromNamedAnswer(partialAns, wireName);
          if (mark.kind === 'known' && mark.value !== 0 && mark.value !== 0n) isPartialCell = true;
        }
        const base = valueFromNamedAnswer(answer, wireName);
        let value: Value<NumericScalar> = base.kind === 'notRecorded'
          ? notRecorded(scoreboardMissingReason(column, displayTag, base.reason))
          : base;
        if (isPartialCell && (base.kind === 'known' || base.kind === 'zero')) {
          value = partialValue(base.value, 'Some inputs were not recorded at this hour');
          hasPartial = true;
        }
        cells[column.key] = { value, format, country: displayTag };
      }
      return {
        key,
        name: identity.name,
        cells,
        ...(hasPartial ? { tooltipNote: 'Partial values omit unrecorded inputs at this hour.' } : {}),
      };
    });

    return {
      columns,
      rows,
      hour,
      rank: true,
      rankMarkers: options.rankMarkers,
      ...(!rows.length ? { empty: 'No countries available for this selection.', emptyKind: 'unavailable' } : {}),
    };
  },
});

function scoreboardState(ctx: Omit<DeclaredTableContext<ScoreboardColumnsReply>, 'settings'>) {
  const registry = ctx.aux?.columns ?? [];
  const shown = selectedColumns(registry, ctx.options.columns, ctx.aux?.defaultColumns ?? []);
  const sort = scoreboardSort(registry, shown, tableSortOf(ctx.options.tableSort, DEFAULT_SORT), ctx.aux?.defaultSort ?? '');
  const rank = rankingColumn(registry, sort, ctx.options.topMeasure, ctx.aux?.defaultSort ?? '');
  const descending = rankingDescending(sort, rank);
  return { registry, shown, sort, rank, descending };
}

export const scoreboardDeclaration: TableWidgetDeclaration<ScoreboardProjectionOptions, ScoreboardColumnsReply> = {
  family: 'table',
  kind: 'scoreboard',
  label: 'Scoreboard',
  subject: 'scoreboard',
  emptyCampaign: 'Load a campaign to compare its countries.',
  icon: 'grid',
  className: 'scoreboard',
  optionFields: OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  aux: {
    product: 'scoreboard-columns',
    params: track => ({ branchId: track.branch.id }),
    validate: validateColumnsReply,
  },
  source: {
    kind: 'multi',
    follow: ctx => {
      const { shown, rank, descending } = scoreboardState(ctx);
      const top = ctx.countries.top;
      const subs: DeclaredFollowItem[] = [];
      if (top != null) {
        if (!rank?.family) return { subs: [], unavailable: 'No measure to rank the top countries by is in this recording.' };
        subs.push({ name: RANK_SOURCE, query: { product: 'series-state', params: { branchId: ctx.branch.id, family: rank.family, top, least: descending ? undefined : true } } });
      }
      const reads = top != null ? 'every' : ctx.countries.tags.length ? ctx.countries.tags.map(tag => ({ country: countryFromKey(tag) })) : null;
      if (reads) {
        for (const column of shown) {
          if (column.family) {
            subs.push({ name: column.key, query: { product: 'series-state', params: { branchId: ctx.branch.id, family: column.family, reads } } });
          }
          if (column.partial) {
            subs.push({ name: partialSource(column.key), query: { product: 'series-state', params: { branchId: ctx.branch.id, family: column.partial, reads } } });
          }
        }
      }
      return { subs, allowEmptyFollow: top == null && subs.length === 0 };
    },
    exactProduct: 'scoreboard',
    exactParams: (ctx, hour) => {
      const { shown, rank, descending } = scoreboardState(ctx);
      const tags = ctx.countries.top == null ? ctx.countries.rankedAt(hour) : [];
      return scoreboardParams(ctx.branch.id, hour, shown, tags, ctx.countries.top, rank, descending);
    },
    dynamicKeys: ['hour', 'countries'],
    validate: (raw, ctx) => validateScoreboardReply(raw, scoreboardState(ctx).shown.length),
  },
  projection: scoreboardProjection,
  columns: ctx => scoreboardColumnsSpec(scoreboardState(ctx).shown, ctx.countries.top),
  settings: {
    kind: 'custom',
    resolve: (ctx, columns) => {
      const { sort } = scoreboardState(ctx);
      const stored = tableSettingsOf(ctx.options.tableSettings, columns, TABLE_BASE);
      return { ...stored, order: columns.map(col => col.key), sort };
    },
    onChange: (next, current, ctx, setOption) => {
      const { registry, shown } = scoreboardState(ctx);
      if (next.sort.column !== current.sort.column || next.sort.direction !== current.sort.direction) {
        const measure = next.sort.column === 'country' ? current.sort.column : next.sort.column;
        if (registry.some(column => column.key === measure && column.available)) setOption('topMeasure', setToken(measure));
        setOption('tableSort', tableSortToken(next.sort));
        return;
      }
      const hidden = shown.filter(column => next.columns[column.key]?.show === false);
      if (hidden.length) {
        setOption('columns', shown.filter(column => !hidden.includes(column)).map(column => setToken(column.key)).join(' '));
        return;
      }
      setOption(TABLE_SETTINGS_KEY, encodeTableSettings(next));
    },
  },
  projectionOptions: ctx => {
    const { shown } = scoreboardState(ctx);
    const top = ctx.countries.top;
    return {
      shown,
      candidates: top != null ? ctx.countries.every : ctx.countries.tags,
      holders: top == null ? ctx.countries.holders : null,
      membership: top == null ? ctx.countries.membership : null,
      unobserved: top == null ? ctx.countries.unobserved : [],
      top,
      rankMarkers: ctx.options.rankMarkers === true,
      looks: ctx.countries.labels,
    };
  },
  unavailable: ctx => {
    const { shown, rank } = scoreboardState(ctx);
    if (ctx.aux && !shown.length) return 'The selected columns are not in this recording. Choose columns in settings.';
    if (ctx.countries.top != null && !rank) return 'No measure to rank the top countries by is in this recording.';
    if (ctx.countries.top == null && ctx.following && !ctx.countries.tags.length && ctx.countries.membership === null) {
      return 'No countries available for this selection.';
    }
    return null;
  },
  warnings: (ctx, followRefused) => {
    const registry = ctx.aux?.columns ?? [];
    const absent = contentSet(ctx.options.columns).map(contentKey).filter(key => !registry.some(column => column.key === key));
    const unknown = ctx.aux && absent.length
      ? [`${absent.length} selected ${absent.length === 1 ? 'column is' : 'columns are'} not in this recording: ${absent.join(', ')}.`]
      : [];
    const unfollowed = followRefused ? [`Values are not followed here: ${followRefused}.`] : [];
    return [...(ctx.aux?.warnings ?? []), ...unknown, ...unfollowed];
  },
  keyByBranch: true,
};
