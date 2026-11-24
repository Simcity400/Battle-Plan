import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { contentSet } from '../../dashboard/options.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsLegendItem, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { nextSeriesRaceChange, seriesRaceUnavailable, seriesRaceWarnings } from '../../families/bars/seriesRace.ts';
import { buildShareRowsModel, type ShareRowEntry } from '../../families/bars/shareRows.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, inHourRanges, type HolderRun, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import { formatScalar, formatShare } from '../../families/valueFormat.ts';
import type { FactoryMixPart, FactoryMixReply, FactoryMixRow } from '../../generated/dashboard';
import { COUNT_MODES, FACTORY_GROUPS, countMode, type CountMode } from './factoriesData.ts';
import type { FactoriesRowMeta } from './factoriesProjection.ts';

const PART_SURFACE = DEFAULT_CARD_SURFACE;

export interface FactoriesMixOptions {
  readonly mode: CountMode;
  readonly modeLabel: string;
  readonly top: number | null;
  readonly all: boolean;
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
}

const isObj = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

export function validateFactoryMixReply(value: unknown): FactoryMixReply {
  if (
    !isObj(value)
    || !Number.isSafeInteger(value.hour)
    || typeof value.mode !== 'string'
    || !Array.isArray(value.parts)
    || !Array.isArray(value.rows)
    || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.warnings)
  ) {
    throw new Error('The factory mix reply is malformed.');
  }
  const partsOk = value.parts.every(
    part => isObj(part)
      && typeof part.key === 'string'
      && (part.name === null || typeof part.name === 'string')
      && typeof part.damaged === 'boolean'
      && typeof part.group === 'string',
  );
  const rowsOk = value.rows.every(
    row => isObj(row)
      && (typeof row.country === 'string' || typeof row.country === 'number')
      && typeof row.tag === 'string'
      && (row.total === null || Number.isSafeInteger(row.total))
      && Array.isArray(row.values)
      && row.values.every(v => v === null || Number.isSafeInteger(v))
      && (row.missing === null || typeof row.missing === 'string')
      && (row.partsMissing === null || typeof row.partsMissing === 'string'),
  );
  if (!partsOk || !rowsOk) throw new Error('The factory mix reply is malformed.');
  return value as unknown as FactoryMixReply;
}

function groupIndex(group: string): number {
  const index = FACTORY_GROUPS.findIndex(item => item.value === group);
  return index >= 0 ? index : 0;
}

function partLabel(part: FactoryMixPart): string {
  if (part.name) return part.name;
  const fallback = FACTORY_GROUPS.find(item => item.value === part.group)?.label ?? part.group;
  return part.damaged ? `Damaged ${fallback}` : fallback;
}

function formatWhole(value: number): string {
  return formatScalar(value, { decimals: 0 });
}

export const factoriesMixProjection: BarsProjection<FactoriesMixOptions> = registerFamilyProjection({
  kind: 'factories',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, { ...options, candidates: [] });
  },
  project(readView: BoundedReadView, options: FactoriesMixOptions): BarsModel {
    const hour = readView.hour;
    const ranked = options.top != null || options.all;
    if (!ranked && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return buildShareRowsModel({ hour, rows: [], unit: 'Factories', empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable' });
    }
    const refused = readView.refused('main');
    if (refused) {
      return buildShareRowsModel({ hour, rows: [], unit: 'Factories', empty: refused, emptyKind: 'unavailable' });
    }
    const panel = readView.panel('main');
    if (panel === undefined) {
      return buildShareRowsModel({ hour, rows: [], unit: 'Factories' });
    }
    const reply = validateFactoryMixReply(panel);
    if (reply.missing !== null) {
      return buildShareRowsModel({ hour, rows: [], unit: 'Factories', empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
    }

    const partInfo = reply.parts.map(part => ({
      part,
      label: partLabel(part),
      color: colour(PART_SURFACE, groupIndex(part.group)),
    }));
    const usedParts = new Set<number>();
    const rowsWithSort: { readonly entry: ShareRowEntry; readonly numeric: number | null; readonly tag: string }[] = [];

    for (const item of reply.rows as readonly FactoryMixRow[]) {
      const key = countryKey(item.country);
      const displayTag = options.holders ? (holderTagAt(options.holders[key], hour) ?? item.tag) : item.tag;
      if (!ranked && options.membership) {
        const inMember = inHourRanges(options.membership[key], hour) || inHourRanges(options.membership[displayTag], hour);
        if (!inMember) continue;
      }
      const pres = countryRowPresentation(readView, displayTag);
      const meta: FactoriesRowMeta = { tag: displayTag, key };

      if (item.total === null) {
        const reason = item.missing ?? 'Not recorded at this hour';
        rowsWithSort.push({
          numeric: null,
          tag: displayTag,
          entry: {
            key,
            subject: displayTag,
            label: pres.label,
            labelMissing: pres.labelMissing,
            media: pres.media,
            segments: [],
            missing: notRecorded(reason),
            format: { decimals: 0 },
            tooltip: {
              title: pres.labelMissing ? 'Name not recorded' : pres.label,
              hour,
              missingReason: reason,
              items: [{ label: options.modeLabel, missingReason: reason }],
            },
            meta,
          },
        });
        continue;
      }

      const countText = formatWhole(item.total);
      const tooltipItems: TooltipListItem[] = [{ label: options.modeLabel, value: countText }];

      if (item.total <= 0) {
        rowsWithSort.push({
          numeric: item.total,
          tag: displayTag,
          entry: {
            key,
            subject: displayTag,
            label: pres.label,
            labelMissing: pres.labelMissing,
            media: pres.media,
            segments: [],
            countText,
            tooltip: { title: pres.labelMissing ? 'Name not recorded' : pres.label, hour, items: tooltipItems },
            meta,
          },
        });
        continue;
      }

      const hasValidParts = item.partsMissing === null
        && item.values.length === partInfo.length
        && item.values.every(v => v !== null && v >= 0);

      if (!hasValidParts) {
        const reason = item.partsMissing ?? 'Factory breakdown not recorded at this hour';
        tooltipItems.push({ label: 'Factory types', missingReason: reason });
        rowsWithSort.push({
          numeric: item.total,
          tag: displayTag,
          entry: {
            key,
            subject: displayTag,
            label: pres.label,
            labelMissing: pres.labelMissing,
            media: pres.media,
            segments: [],
            countText,
            missing: notRecorded(reason),
            format: { decimals: 0 },
            tooltip: { title: pres.labelMissing ? 'Name not recorded' : pres.label, hour, missingReason: reason, items: tooltipItems },
            meta,
          },
        });
        continue;
      }

      const sumParts = item.values.reduce<number>((acc, v) => acc + (v ?? 0), 0);
      if (sumParts !== item.total) {
        const reason = 'Factory breakdown does not reconcile with the recorded total at this hour';
        tooltipItems.push({ label: 'Factory types', missingReason: reason });
        rowsWithSort.push({
          numeric: item.total,
          tag: displayTag,
          entry: {
            key,
            subject: displayTag,
            label: pres.label,
            labelMissing: pres.labelMissing,
            media: pres.media,
            segments: [],
            countText,
            missing: notRecorded(reason),
            format: { decimals: 0 },
            tooltip: { title: pres.labelMissing ? 'Name not recorded' : pres.label, hour, missingReason: reason, items: tooltipItems },
            meta,
          },
        });
        continue;
      }

      const segments: BarsSegment[] = [];
      for (let i = 0; i < partInfo.length; i++) {
        const raw = item.values[i]!;
        if (raw <= 0) continue;
        const info = partInfo[i]!;
        const share = raw / item.total;
        usedParts.add(i);
        const pctText = formatShare(share);
        segments.push({
          key: info.part.key,
          label: info.label,
          share,
          value: knownValue(raw),
          format: { decimals: 0 },
          color: info.color,
          ...(info.part.damaged ? { hatched: true } : {}),
          inlineLabel: `${info.label} ${pctText}`,
        });
        tooltipItems.push({
          label: info.label,
          swatch: info.color,
          value: `${formatWhole(raw)} (${pctText})`,
        });
      }

      rowsWithSort.push({
        numeric: item.total,
        tag: displayTag,
        entry: {
          key,
          subject: displayTag,
          label: pres.label,
          labelMissing: pres.labelMissing,
          media: pres.media,
          segments,
          countText,
          tooltip: { title: pres.labelMissing ? 'Name not recorded' : pres.label, hour, items: tooltipItems },
          meta,
        },
      });
    }

    if (!ranked) {
      rowsWithSort.sort((a, b) => {
        const aMiss = a.numeric === null;
        const bMiss = b.numeric === null;
        if (aMiss !== bMiss) return aMiss ? 1 : -1;
        if (!aMiss && !bMiss && a.numeric !== b.numeric) return b.numeric! - a.numeric!;
        return compareTags(a.tag, b.tag);
      });
    }

    const legend: BarsLegendItem[] = [];
    for (let i = 0; i < partInfo.length; i++) {
      if (!usedParts.has(i)) continue;
      const info = partInfo[i]!;
      legend.push({
        key: info.part.key,
        label: info.label,
        color: info.color,
        ...(info.part.damaged ? { hatched: true } : {}),
      });
    }

    const rows = rowsWithSort.map(item => item.entry);
    return buildShareRowsModel({
      hour,
      rows,
      legend,
      unit: 'Factories',
      warnings: reply.warnings,
      ...(!rows.length ? { empty: 'No factories recorded at this date.', emptyKind: 'empty' as const } : {}),
    });
  },
}, 'mix');

function factoriesMixSources(ctx: FamilyPlanContext): SourcePlan {
  const mode = countMode(ctx.options.count);
  const types = contentSet(ctx.options.types);
  const top = ctx.countries.top;
  const all = ctx.countries.all;
  const countries = top == null && !all ? ctx.countries.tags.map(countryFromKey) : [];
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : all
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const params = {
    branchId: ctx.branch.id,
    countries,
    ...(top != null ? { top } : {}),
    mode,
    types,
  };
  return {
    owner: `factories:mix:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${mode}:${types.join(',')}`,
    entries: top == null && !all && !countries.length ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage,
        product: 'factory-mix',
        followQuery: { panel: 'factory-mix', ...params },
        params,
        validate: validateFactoryMixReply,
      },
    }],
    allowEmptyFollow: top == null && !all && !countries.length,
  };
}

export const factoriesMixDeclaration: BarsWidgetDeclaration<FactoriesMixOptions> = {
  family: 'bars',
  kind: 'factories',
  label: 'Factory composition',
  subject: 'factories',
  emptyCampaign: 'Load a campaign to see factories.',
  icon: 'factory',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: factoriesMixSources,
  projection: factoriesMixProjection,
  projectionOptions: ctx => {
    const mode = countMode(ctx.options.count);
    const modeTitle = COUNT_MODES.find(item => item.value === mode)?.label.toLowerCase() ?? 'available';
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      mode,
      modeLabel: `Factories, ${modeTitle}`,
      top: ctx.countries.top,
      all: ctx.countries.all,
      holders: ranked ? null : ctx.countries.holders,
      membership: ranked ? null : ctx.countries.membership,
      unobserved: ranked ? [] : ctx.countries.unobserved,
    };
  },
  unavailable: seriesRaceUnavailable,
  warnings: (_ctx, followRefused) => seriesRaceWarnings(followRefused),
  jumpToDate: true,
  keyByBranch: true,
};
