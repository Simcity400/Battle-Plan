import { tableSortOf, tableSortToken } from '../../charts/table/options.ts';
import { countryFromKey, countryKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { changedAnswerRows } from '../../dashboard/follow/tables.ts';
import { contentKey, contentSet, setToken } from '../../dashboard/options.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourceEntry, SourcePlan } from '../../families/contract.ts';
import {
  holderTagAt,
  nextHoldersChange,
  nextMembershipChange,
  nextRangesChange,
  type HolderRun,
  type HourRanges,
  type TableWidgetDeclaration,
} from '../../families/table/declaration.ts';
import { lookAt, nextLookChange, type CountryLooksMap } from '../../families/looks.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
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
import { DEFAULT_SORT, TABLE_BASE } from './definition.ts';
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
      showNamesDefault: true,
      pinned: true,
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
        barEligible: true,
        tone: 'country',
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

interface ScoreboardIncCache {
  readonly validFrom: number;
  readonly validUntil: number;
  readonly rankAnswer: FollowedAnswer | undefined;
  readonly answers: readonly (FollowedAnswer | undefined)[];
  readonly partials: readonly (FollowedAnswer | undefined)[];
  readonly entries: readonly { readonly key: string; readonly displayTag: string; readonly wireName: string }[];
  readonly wireToIndices: ReadonlyMap<string, readonly number[]>;
  readonly model: TableModel;
}

const scoreboardIncCache = new WeakMap<ScoreboardProjectionOptions, ScoreboardIncCache>();

function buildScoreboardRow(
  entry: { readonly key: string; readonly displayTag: string; readonly wireName: string },
  readView: BoundedReadView,
  hour: number,
  options: ScoreboardProjectionOptions,
): TableRow {
  const { key, displayTag, wireName } = entry;
  const identity = countryNameValue(displayTag, options.looks, hour);
  const cells: Record<string, TableCell> = {
    country: { value: identity.value, iconSlot: true, country: displayTag, ...(identity.runs ? { looks: identity.runs } : {}) },
  };
  let hasPartial = false;
  for (let i = 0; i < options.shown.length; i++) {
    const column = options.shown[i]!;
    const format = columnFormat(column);
    if (!column.available || !column.family) {
      cells[column.key] = { value: notRecorded(column.reason ?? 'Unavailable'), format, country: displayTag };
      continue;
    }
    const answer = readView.answer(column.key);
    const partialAns = column.partial ? readView.answer(partialSource(column.key)) : undefined;
    let isPartialCell = false;
    if (partialAns) {
      const mark = valueFromNamedAnswer(partialAns, wireName);
      if (mark.kind === 'known' && mark.value !== 0 && mark.value !== 0n) isPartialCell = true;
    }
    const base = valueFromNamedAnswer(answer, wireName);
    let value: Value<NumericScalar> = base;
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
}

function sameValue(a: Value, b: Value): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === 'known' || a.kind === 'zero') return a.value === (b as typeof a).value;
  if (a.kind === 'partial') return a.value === (b as typeof a).value && a.reason === (b as typeof a).reason && a.atLeast === (b as typeof a).atLeast;
  return a.reason === (b as typeof a).reason;
}

function sameRow(before: TableRow, after: TableRow): boolean {
  if (before.name !== after.name || before.tooltipNote !== after.tooltipNote) return false;
  for (const key in after.cells) {
    const a = before.cells[key];
    const b = after.cells[key]!;
    if (!a || a.country !== b.country || a.looks !== b.looks || !sameValue(a.value, b.value)) return false;
  }
  return true;
}

function sameRankedEntries(
  entries: readonly { readonly key: string }[],
  rankAnswer: FollowedAnswer | undefined,
): boolean {
  if (!rankAnswer || rankAnswer.kind === 'notRecorded') return false;
  const tags = rankedTags(rankAnswer);
  if (tags.length !== entries.length) return false;
  for (let i = 0; i < tags.length; i++) {
    if (entries[i]!.key !== tags[i]) return false;
  }
  return true;
}

export const scoreboardProjection: TableProjection<ScoreboardProjectionOptions> = registerFamilyProjection({
  kind: 'scoreboard',
  family: 'table',
  nextChange(readView, options) {
    const hour = readView.hour;
    return Math.min(
      nextLookChange(options.looks, hour),
      nextHoldersChange(options.holders, hour),
      nextRangesChange(options.unobserved, hour),
      nextMembershipChange(options.membership, hour),
    );
  },
  project(readView: BoundedReadView, options: ScoreboardProjectionOptions): TableModel {
    const hour = readView.hour;
    const panel = readView.panel('main');

    if (panel !== undefined) {
      const columns = scoreboardColumnsSpec(options.shown, options.top);
      const reply: ScoreboardReply = validateScoreboardReply(panel, options.shown.length);
      if (reply.missing !== null) {
        return { columns, rows: [], hour: reply.hour, rowTimeBoundaries: false, empty: reply.missing, emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers, warnings: reply.warnings };
      }
      if (options.top == null && options.unobserved?.some(([first, last]) => first <= reply.hour && reply.hour <= last)) {
        return { columns, rows: [], hour: reply.hour, rowTimeBoundaries: false, empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers, warnings: reply.warnings };
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
              { kind: 'notRecorded', reason: rawCell.missing ?? column.reason ?? 'Not recorded at this hour' },
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
        rowTimeBoundaries: false,
        rank: true,
        rankMarkers: options.rankMarkers,
        warnings: reply.warnings,
        ...(!rows.length ? { empty: 'No countries available for this selection.', emptyKind: 'unavailable' } : {}),
      };
    }

    const rankAnswer = options.top != null ? readView.answer(RANK_SOURCE) : undefined;
    const prevInc = scoreboardIncCache.get(options);
    if (
      prevInc
      && hour >= prevInc.validFrom
      && hour < prevInc.validUntil
      && prevInc.entries.length > 0
      && (
        options.top == null
        || prevInc.rankAnswer === rankAnswer
        || sameRankedEntries(prevInc.entries, rankAnswer)
      )
    ) {
      const nextAnswers = options.shown.map(col => (col.available && col.family ? readView.answer(col.key) : undefined));
      const nextPartials = options.shown.map(col => (col.available && col.family && col.partial ? readView.answer(partialSource(col.key)) : undefined));
      const touchedIndices = new Set<number>();
      let canPatch = true;
      const collectDiff = (prevAns: FollowedAnswer | undefined, nextAns: FollowedAnswer | undefined) => {
        if (prevAns === nextAns) return;
        if (!prevAns || !nextAns || prevAns.kind !== 'rows' || nextAns.kind !== 'rows') {
          canPatch = false;
          return;
        }
        const diff = changedAnswerRows(prevAns, nextAns);
        if (diff === null) {
          canPatch = false;
          return;
        }
        for (let d = 0; d < diff.length; d++) {
          const name = rowName(nextAns, diff[d]!);
          if (name === null) continue;
          const indices = prevInc.wireToIndices.get(name);
          if (!indices) continue;
          for (let k = 0; k < indices.length; k++) touchedIndices.add(indices[k]!);
        }
      };
      for (let i = 0; i < options.shown.length && canPatch; i++) {
        collectDiff(prevInc.answers[i], nextAnswers[i]);
        if (canPatch) collectDiff(prevInc.partials[i], nextPartials[i]);
      }
      if (canPatch) {
        if (touchedIndices.size === 0) {
          const nextModel: TableModel = { ...prevInc.model, hour, changedKeys: new Set(), changedRows: new Map() };
          scoreboardIncCache.set(options, {
            ...prevInc,
            rankAnswer,
            answers: nextAnswers,
            partials: nextPartials,
            model: nextModel,
          });
          return nextModel;
        }
        {
          const nextRows = prevInc.model.rows.slice();
          const changedKeys = new Set<string>();
          const changedRows = new Map<string, TableRow>();
          for (const idx of touchedIndices) {
            const built = buildScoreboardRow(prevInc.entries[idx]!, readView, hour, options);
            nextRows[idx] = built;
            changedKeys.add(built.key);
            changedRows.set(built.key, built);
          }
          const nextModel: TableModel = {
            ...prevInc.model,
            hour,
            rows: nextRows,
            changedKeys,
            changedRows,
          };
          scoreboardIncCache.set(options, {
            ...prevInc,
            rankAnswer,
            answers: nextAnswers,
            partials: nextPartials,
            model: nextModel,
          });
          return nextModel;
        }
      }
    }

    const columns = scoreboardColumnsSpec(options.shown, options.top);
    if (options.top != null) {
      if (!rankAnswer) return { columns, rows: [], hour, rowTimeBoundaries: false, rank: true, rankMarkers: options.rankMarkers };
      if (rankAnswer.kind === 'notRecorded') {
        return { columns, rows: [], hour, rowTimeBoundaries: false, empty: rankAnswer.reason, emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers };
      }
    } else if (options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return { columns, rows: [], hour, rowTimeBoundaries: false, empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable', rank: true, rankMarkers: options.rankMarkers };
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

    const held = prevInc ? new Map(prevInc.model.rows.map(row => [row.key, row])) : null;
    const rows: TableRow[] = activeEntries.map(entry => {
      const built = buildScoreboardRow(entry, readView, hour, options);
      const before = held?.get(built.key);
      return before && sameRow(before, built) ? before : built;
    });
    const model: TableModel = {
      columns,
      rows,
      hour,
      rowTimeBoundaries: false,
      rank: true,
      rankMarkers: options.rankMarkers,
      ...(!rows.length ? { empty: 'No countries available for this selection.', emptyKind: 'unavailable' } : {}),
    };
    if (activeEntries.length > 0) {
      const wireToIndices = new Map<string, number[]>();
      for (let i = 0; i < activeEntries.length; i++) {
        const w = activeEntries[i]!.wireName;
        let list = wireToIndices.get(w);
        if (!list) wireToIndices.set(w, list = [i]);
        else list.push(i);
      }
      const validUntil = Math.min(
        nextLookChange(options.looks, hour),
        nextHoldersChange(options.holders, hour),
        nextRangesChange(options.unobserved, hour),
        nextMembershipChange(options.membership, hour),
      );
      scoreboardIncCache.set(options, {
        validFrom: hour,
        validUntil,
        rankAnswer,
        answers: options.shown.map(col => (col.available && col.family ? readView.answer(col.key) : undefined)),
        partials: options.shown.map(col => (col.available && col.family && col.partial ? readView.answer(partialSource(col.key)) : undefined)),
        entries: activeEntries,
        wireToIndices,
        model,
      });
    }
    return model;
  },
});

function scoreboardState(ctx: FamilyPlanContext<ScoreboardColumnsReply>) {
  const registry = ctx.aux?.columns ?? [];
  const shown = selectedColumns(registry, ctx.options.columns, ctx.aux?.defaultColumns ?? []);
  const sort = scoreboardSort(registry, shown, tableSortOf(ctx.options.tableSort, DEFAULT_SORT), ctx.aux?.defaultSort ?? '');
  const rank = rankingColumn(registry, sort, ctx.options.topMeasure, ctx.aux?.defaultSort ?? '');
  const descending = rankingDescending(sort, rank);
  return { registry, shown, sort, rank, descending };
}

interface ScoreboardExactBase {
  readonly branchId: string;
  readonly shown: readonly ScoreboardColumn[];
  readonly top: number | null;
  readonly rank: ScoreboardColumn | undefined;
  readonly descending: boolean;
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
}

function buildScoreboardExactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as ScoreboardExactBase;
  let tags: readonly string[] = [];
  if (cfg.top == null) {
    if (cfg.membership) {
      tags = cfg.tags.filter(tag => cfg.membership![tag]?.some(([first, last]) => first <= hour && hour <= last));
    } else {
      tags = cfg.tags;
    }
  }
  return scoreboardParams(cfg.branchId, hour, cfg.shown, tags, cfg.top, cfg.rank, cfg.descending);
}

function scoreboardSources(ctx: FamilyPlanContext<ScoreboardColumnsReply>): SourcePlan {
  const { shown, rank, descending } = scoreboardState(ctx);
  const top = ctx.countries.top;
  const owner = `scoreboard:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`;
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: descending ? 'top' : 'least', limit: top }
    : { mode: 'keyed', keys: ctx.countries.tags };

  const exactBase: ScoreboardExactBase = {
    branchId: ctx.branch.id,
    shown,
    top,
    rank,
    descending,
    tags: ctx.countries.tags,
    membership: ctx.countries.membership,
  };
  const exactEntry: SourceEntry = {
    name: 'main',
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'point',
      channel: 'followed',
      coverage,
      product: 'scoreboard',
      exactProduct: 'scoreboard',
      followQuery: null,
      params: exactBase as unknown as Readonly<Record<string, unknown>>,
      buildExactParams: buildScoreboardExactParams,
      dynamicKeys: ['hour', 'countries'],
      validate: raw => validateScoreboardReply(raw, shown.length),
    },
  };

  if (top != null && !rank?.family) {
    return {
      owner,
      entries: [],
      staticEntry: exactEntry,
      exactEntry,
      unavailable: 'No measure to rank the top countries by is in this recording.',
    };
  }

  const entries: SourceEntry[] = [];
  if (top != null && rank?.family) {
    entries.push({
      name: RANK_SOURCE,
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'series-state',
        followQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family: rank.family, top, least: descending ? undefined : true } },
        params: null,
      },
    });
  }

  const reads = top != null ? 'every' : ctx.countries.tags.length ? ctx.countries.tags.map(tag => ({ country: countryFromKey(tag) })) : null;
  const rowCoverage = top == null ? { coverage: 'row' } : {};
  if (reads) {
    for (const column of shown) {
      if (column.family) {
        entries.push({
          name: column.key,
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'followed',
            coverage,
            product: 'series-state',
            followQuery: { product: 'series-state', ...rowCoverage, params: { branchId: ctx.branch.id, family: column.family, reads } },
            params: null,
          },
        });
      }
      if (column.partial) {
        entries.push({
          name: partialSource(column.key),
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'followed',
            coverage,
            product: 'series-state',
            followQuery: { product: 'series-state', ...rowCoverage, params: { branchId: ctx.branch.id, family: column.partial, reads } },
            params: null,
          },
        });
      }
    }
  }

  return {
    owner,
    entries,
    staticEntry: exactEntry,
    exactEntry,
    allowEmptyFollow: top == null && entries.length === 0,
  };
}

export const scoreboardDeclaration: TableWidgetDeclaration<ScoreboardProjectionOptions, ScoreboardColumnsReply> = {
  family: 'table',
  kind: 'scoreboard',
  label: 'Scoreboard',
  subject: 'scoreboard',
  emptyCampaign: 'Load a campaign to compare its countries.',
  icon: 'grid',
  className: 'scoreboard',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  aux: {
    product: 'scoreboard-columns',
    params: track => ({ branchId: track.branch.id }),
    validate: validateColumnsReply,
  },
  sources: scoreboardSources,
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
