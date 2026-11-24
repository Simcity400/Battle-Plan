import { colour } from '../../charts/palette.ts';
import { validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryFromKey, countryKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { CELL, cellNumber, cellState, cellValue, rowName, type FollowedAnswer } from '../../dashboard/follow/message.ts';
import { contentSet } from '../../dashboard/options.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourceEntry, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, nextHoldersChange, nextMembershipChange, nextRangesChange, type HolderRun, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, notRecorded, valueFromNamedAnswer, valueFromRawBigInt, zeroValue, type Value } from '../../families/value.ts';
import type { FactoryGroupsReply, SeriesStateReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { COUNT_MODES, chosenGroups, countMode, factoryMeasure, groupName, validateFactoryGroups, type CountMode } from './factoriesData.ts';

const PART_SURFACE = '#1e1e1e';

export interface FactoryPartSpec {
  readonly id: string;
  readonly label: string;
  readonly source: string;
  readonly damagedSource?: string;
  readonly color: string;
}

export interface FactoriesProjectionOptions {
  readonly mode: CountMode;
  readonly modeLabel: string;
  readonly split: boolean;
  readonly parts: readonly FactoryPartSpec[];
  readonly candidates: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
  readonly top: number | null;
}

export interface FactoriesRowMeta {
  readonly tag: string;
  readonly key: string;
}

interface FactoriesExactBase {
  readonly branchId: string;
  readonly measure: string;
  readonly top: number | null;
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
}

function partSourceKey(id: string): string {
  return `part:${id}`;
}

function damagedSourceKey(id: string): string {
  return `damaged:${id}`;
}

function partSpecs(mode: CountMode, types: readonly string[], groups: FactoryGroupsReply | undefined): FactoryPartSpec[] {
  return chosenGroups(types).map((group, index) => ({
    id: group.value,
    label: groupName(group.value, groups),
    source: partSourceKey(group.value),
    ...(mode === 'built' ? { damagedSource: damagedSourceKey(group.value) } : {}),
    color: colour(PART_SURFACE, index),
  }));
}

function buildFactoriesExactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as FactoriesExactBase;
  const family = { kind: 'country-factories', measure: cfg.measure };
  if (cfg.top != null) {
    return { branchId: cfg.branchId, family, hour, top: cfg.top };
  }
  const activeTags = cfg.membership
    ? cfg.tags.filter(tag => cfg.membership![tag]?.some(([first, last]) => first <= hour && hour <= last))
    : cfg.tags;
  return {
    branchId: cfg.branchId,
    family,
    reads: activeTags.map(tag => ({ country: countryFromKey(tag), hour })),
  };
}

function numericOf(value: Value<bigint | number>): number | null {
  if (value.kind === 'known' || value.kind === 'partial') return Number(value.value);
  if (value.kind === 'zero') return 0;
  return null;
}

function scalarOf(raw: bigint | number): Value<bigint | number> {
  return raw === 0 || raw === 0n ? zeroValue() : knownValue(raw);
}

function buildPartSegments(
  readView: BoundedReadView,
  wireName: string,
  total: number,
  options: FactoriesProjectionOptions,
  baseColor: string | undefined,
): { readonly segments?: readonly BarsSegment[]; readonly tooltipItems: TooltipListItem[] } {
  if (!options.split && options.mode !== 'built') return { tooltipItems: [] };
  if (total <= 0) return { tooltipItems: [] };

  const tooltipItems: TooltipListItem[] = [];
  if (!options.split && options.mode === 'built') {
    let damagedTotal = 0;
    let sawAny = false;
    for (const part of options.parts) {
      if (!part.damagedSource) continue;
      const ans = readView.answer(part.damagedSource);
      if (!ans) continue;
      const val = valueFromNamedAnswer(ans, wireName);
      const n = numericOf(val);
      if (n !== null) {
        sawAny = true;
        damagedTotal += Math.max(0, n);
      }
    }
    if (!sawAny || damagedTotal <= 0) return { tooltipItems };
    const clampedDamaged = Math.min(total, damagedTotal);
    const healthy = Math.max(0, total - clampedDamaged);
    const fill = baseColor ?? colour(PART_SURFACE, 0);
    const segments: BarsSegment[] = [];
    if (healthy > 0) {
      segments.push({ key: 'healthy', label: 'Undamaged', share: healthy / total, value: scalarOf(healthy), format: { decimals: 0 }, color: fill });
    }
    segments.push({ key: 'damaged', label: 'Damaged', share: clampedDamaged / total, value: scalarOf(clampedDamaged), format: { decimals: 0 }, color: fill, hatched: true });
    tooltipItems.push({ label: 'Damaged', value: Math.round(clampedDamaged).toLocaleString('en-US') });
    return { segments, tooltipItems };
  }

  const segments: BarsSegment[] = [];
  let sumParts = 0;
  const rawParts: { readonly spec: FactoryPartSpec; readonly amount: number; readonly damaged: number }[] = [];
  for (const spec of options.parts) {
    const ans = readView.answer(spec.source);
    if (!ans) continue;
    const val = valueFromNamedAnswer(ans, wireName);
    const amount = numericOf(val);
    if (amount === null) {
      tooltipItems.push({ label: spec.label, swatch: spec.color, missingReason: val.kind === 'notRecorded' ? val.reason : 'Not recorded' });
      continue;
    }
    let damaged = 0;
    if (spec.damagedSource) {
      const dmgAns = readView.answer(spec.damagedSource);
      if (dmgAns) {
        const dmgVal = valueFromNamedAnswer(dmgAns, wireName);
        damaged = Math.max(0, numericOf(dmgVal) ?? 0);
      }
    }
    const clamped = Math.max(0, amount);
    sumParts += clamped;
    rawParts.push({ spec, amount: clamped, damaged: Math.min(clamped, damaged) });
    tooltipItems.push({ label: spec.label, swatch: spec.color, value: Math.round(clamped).toLocaleString('en-US') });
  }
  const denom = sumParts > 0 ? sumParts : total;
  if (denom <= 0 || !rawParts.length) return { tooltipItems };
  for (const item of rawParts) {
    if (item.amount <= 0) continue;
    const healthy = Math.max(0, item.amount - item.damaged);
    if (healthy > 0) {
      segments.push({
        key: item.spec.id,
        label: item.spec.label,
        share: healthy / denom,
        value: scalarOf(healthy),
        format: { decimals: 0 },
        color: item.spec.color,
      });
    }
    if (item.damaged > 0) {
      segments.push({
        key: `damaged:${item.spec.id}`,
        label: `Damaged ${item.spec.label}`,
        share: item.damaged / denom,
        value: scalarOf(item.damaged),
        format: { decimals: 0 },
        color: item.spec.color,
        hatched: true,
      });
    }
  }
  return { ...(segments.length ? { segments } : {}), tooltipItems };
}

function makeRow(
  readView: BoundedReadView,
  key: string,
  displayTag: string,
  wireName: string,
  value: Value<bigint | number>,
  options: FactoriesProjectionOptions,
): BarsDataRow {
  const pres = countryRowPresentation(readView, displayTag);
  const numeric = numericOf(value);
  const parts = numeric !== null ? buildPartSegments(readView, wireName, numeric, options, pres.color) : { tooltipItems: [] };
  const totalItem: TooltipListItem = value.kind === 'notRecorded'
    ? { label: options.modeLabel, missingReason: value.reason }
    : value.kind === 'notApplicable'
      ? { label: options.modeLabel, value: '\u2014' }
      : { label: options.modeLabel, value: Math.round(Number(value.kind === 'zero' ? 0 : value.value)).toLocaleString('en-US') };
  const meta: FactoriesRowMeta = { tag: displayTag, key };
  return {
    kind: 'data',
    key,
    subject: displayTag,
    label: pres.label,
    labelMissing: pres.labelMissing,
    media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    value: {
      value,
      format: { decimals: 0 },
      label: options.modeLabel,
    },
    numeric,
    ...(parts.segments ? { segments: parts.segments } : {}),
    tooltip: {
      title: pres.labelMissing ? 'Name not recorded' : pres.label,
      hour: readView.hour,
      items: [totalItem, ...parts.tooltipItems],
    },
    meta,
  };
}

function compareExplicitRows(a: BarsDataRow, b: BarsDataRow): number {
  const aMissing = a.numeric === null || a.numeric === undefined;
  const bMissing = b.numeric === null || b.numeric === undefined;
  if (aMissing !== bMissing) return aMissing ? 1 : -1;
  if (!aMissing && !bMissing && a.numeric !== b.numeric) return b.numeric! - a.numeric!;
  return compareTags(a.subject ?? a.key, b.subject ?? b.key);
}

function rowsFromFollowedTop(
  answer: FollowedAnswer,
  readView: BoundedReadView,
  options: FactoriesProjectionOptions,
): BarsDataRow[] {
  if (answer.kind === 'notRecorded') return [];
  const rows: BarsDataRow[] = [];
  for (let i = 0; i < answer.rows; i++) {
    const at = i * answer.columns;
    const tag = rowName(answer, i);
    if (!tag) continue;
    const state = cellState(answer, at);
    let value: Value<bigint | number>;
    if (state === CELL.value) {
      const safe = cellNumber(answer, at);
      value = scalarOf(safe ?? cellValue(answer, at));
    } else {
      value = notRecorded('Not recorded at this hour');
    }
    rows.push(makeRow(readView, tag, tag, tag, value, options));
  }
  return rows;
}

export const factoriesProjection: BarsProjection<FactoriesProjectionOptions> = registerBarsProjection({
  kind: 'factories',
  family: 'bars',
  nextChange(readView, options) {
    const hour = readView.hour;
    return Math.min(
      nextHoldersChange(options.holders, hour),
      nextRangesChange(options.unobserved, hour),
      nextMembershipChange(options.membership, hour),
    );
  },
  project(readView: BoundedReadView, options: FactoriesProjectionOptions): BarsModel {
    const hour = readView.hour;
    const baseModel = {
      form: 'ordered' as const,
      hour,
      animateOrder: true,
      showAxis: true,
      scaleUnit: 'Factories',
      scaleFormat: { decimals: 0 },
    };

    if (options.top == null && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return {
        ...baseModel,
        rows: [],
        empty: 'Group membership is unavailable at this time.',
        emptyKind: 'unavailable',
      };
    }

    const panel = readView.panel('main');
    if (panel !== undefined) {
      const reply: SeriesStateReply = validateRankReply(panel);
      const rows: BarsDataRow[] = [];
      for (const state of reply.states) {
        const key = countryKey(state.country);
        const displayTag = options.holders ? (holderTagAt(options.holders[key], hour) ?? key) : key;
        if (options.top == null && options.membership) {
          const inMember = options.membership[key]?.some(([first, last]) => first <= hour && hour <= last)
            || options.membership[displayTag]?.some(([first, last]) => first <= hour && hour <= last);
          if (!inMember) continue;
        }
        const value = valueFromRawBigInt(
          state.run?.valueRaw ?? null,
          { kind: 'notRecorded', reason: state.missing ?? 'Not recorded at this hour' },
        );
        rows.push(makeRow(readView, key, displayTag, key, value, options));
      }
      if (options.top == null) rows.sort(compareExplicitRows);
      return {
        ...baseModel,
        rows,
        ...(!rows.length ? { empty: 'No factories recorded at this date.', emptyKind: 'empty' as const } : {}),
      };
    }

    const answer = readView.answer('main');
    if (!answer) {
      return { ...baseModel, rows: [] };
    }
    if (answer.kind === 'notRecorded') {
      return {
        ...baseModel,
        rows: [],
        empty: answer.reason,
        emptyKind: 'unavailable',
      };
    }

    let rows: BarsDataRow[];
    if (options.top != null) {
      rows = rowsFromFollowedTop(answer, readView, options);
    } else {
      rows = [];
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
        const value = valueFromNamedAnswer(answer, candidate);
        rows.push(makeRow(readView, candidate, datedTag, candidate, value, options));
      }
      rows.sort(compareExplicitRows);
    }

    return {
      ...baseModel,
      rows,
      ...(!rows.length ? { empty: 'No factories recorded at this date.', emptyKind: 'empty' as const } : {}),
    };
  },
});

function factoriesSources(ctx: FamilyPlanContext<FactoryGroupsReply>): SourcePlan {
  const mode = countMode(ctx.options.count);
  const types = contentSet(ctx.options.types);
  const measureName = factoryMeasure(mode, types);
  const family = { kind: 'country-factories', measure: measureName };
  const top = ctx.countries.top;
  const split = ctx.options.split === true;
  const owner = `factories:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${measureName}:${split ? 1 : 0}`;
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : { mode: 'keyed', keys: ctx.countries.tags };

  const exactBase: FactoriesExactBase = {
    branchId: ctx.branch.id,
    measure: measureName,
    top,
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
      product: 'series-state',
      exactProduct: 'series-state',
      followQuery: null,
      params: exactBase as unknown as Readonly<Record<string, unknown>>,
      buildExactParams: buildFactoriesExactParams,
      dynamicKeys: ['hour', 'reads'],
      validate: validateRankReply,
    },
  };

  const entries: SourceEntry[] = [];
  if (top != null) {
    entries.push({
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'series-state',
        followQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family, top } },
        params: null,
      },
    });
  } else if (ctx.countries.tags.length) {
    entries.push({
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'series-state',
        followQuery: {
          product: 'series-state',
          coverage: 'row',
          params: { branchId: ctx.branch.id, family, reads: ctx.countries.tags.map(tag => ({ country: countryFromKey(tag) })) },
        },
        params: null,
      },
    });
  }

  if (split || mode === 'built') {
    const reads = top != null ? 'every' : ctx.countries.tags.length ? ctx.countries.tags.map(tag => ({ country: countryFromKey(tag) })) : null;
    const rowCoverage = top == null ? { coverage: 'row' } : {};
    if (reads) {
      for (const group of chosenGroups(types)) {
        if (split) {
          entries.push({
            name: partSourceKey(group.value),
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'followed',
              coverage,
              product: 'series-state',
              followQuery: {
                product: 'series-state',
                ...rowCoverage,
                params: { branchId: ctx.branch.id, family: { kind: 'country-factories', measure: `${mode}${group.bit}` }, reads },
              },
              params: null,
            },
          });
        }
        if (mode === 'built') {
          entries.push({
            name: damagedSourceKey(group.value),
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'followed',
              coverage,
              product: 'series-state',
              followQuery: {
                product: 'series-state',
                ...rowCoverage,
                params: { branchId: ctx.branch.id, family: { kind: 'country-factories', measure: `damaged${group.bit}` }, reads },
              },
              params: null,
            },
          });
        }
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

export const factoriesDeclaration: BarsWidgetDeclaration<FactoriesProjectionOptions, FactoryGroupsReply> = {
  family: 'bars',
  kind: 'factories',
  label: 'Factories',
  subject: 'factories',
  emptyCampaign: 'Load a campaign to see factories.',
  icon: 'factory',
  optionFields: OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  aux: {
    product: 'factory-groups',
    params: track => ({ branchId: track.branch.id }),
    validate: validateFactoryGroups,
  },
  sources: factoriesSources,
  projection: factoriesProjection,
  projectionOptions: ctx => {
    const mode = countMode(ctx.options.count);
    const types = contentSet(ctx.options.types);
    const modeTitle = COUNT_MODES.find(item => item.value === mode)?.label.toLowerCase() ?? 'available';
    const top = ctx.countries.top;
    return {
      mode,
      modeLabel: `Factories, ${modeTitle}`,
      split: ctx.options.split === true,
      parts: partSpecs(mode, types, ctx.aux),
      candidates: top != null ? ctx.countries.every : ctx.countries.tags,
      holders: top == null ? ctx.countries.holders : null,
      membership: top == null ? ctx.countries.membership : null,
      unobserved: top == null ? ctx.countries.unobserved : [],
      top,
    };
  },
  unavailable: ctx => {
    if (ctx.countries.top == null && ctx.following && !ctx.countries.tags.length && ctx.countries.membership === null) {
      return 'No countries available for this selection.';
    }
    return null;
  },
  warnings: (ctx, followRefused) => {
    const list: string[] = [];
    if (ctx.aux?.missing) list.push(ctx.aux.missing);
    if (ctx.aux?.warnings.length) list.push(...ctx.aux.warnings);
    if (followRefused) list.push(`Values are not followed here: ${followRefused}.`);
    return list;
  },
  jumpToDate: true,
  keyByBranch: true,
};
