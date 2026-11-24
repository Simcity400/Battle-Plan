import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { nextSeriesRaceChange, seriesRaceUnavailable, seriesRaceWarnings } from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, inHourRanges, type HolderRun, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { RaidTallyAtReply, RaidTallyAtRow, RaidTallyMetric } from '../../generated/dashboard';

const NOTE = 'Success includes critical success and excludes limited success.';

export interface RaidTallyRaceOptions {
  readonly metric: RaidTallyMetric;
  readonly top: number | null;
  readonly all: boolean;
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
}

const isObj = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

export function validateRaidTallyAt(value: unknown): RaidTallyAtReply {
  if (
    !isObj(value)
    || typeof value.branchId !== 'string'
    || !Number.isSafeInteger(value.hour)
    || !Number.isSafeInteger(value.firstHour)
    || !Number.isSafeInteger(value.lastHour)
    || (value.metric !== 'both' && value.metric !== 'launched' && value.metric !== 'succeeded')
    || !(value.available === null || typeof value.available === 'boolean')
    || !(value.unavailable === null || typeof value.unavailable === 'string')
    || !Array.isArray(value.rows)
    || !Number.isSafeInteger(value.unattributed)
    || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.warnings)
  ) {
    throw new Error('The raid tally reply is malformed.');
  }
  return value as unknown as RaidTallyAtReply;
}

function provenInterval(data: unknown): readonly [number, number] | null {
  const reply = data as Partial<RaidTallyAtReply> | null | undefined;
  return reply && typeof reply.firstHour === 'number' && typeof reply.lastHour === 'number' && reply.firstHour <= reply.lastHour
    ? [reply.firstHour, reply.lastHour]
    : null;
}

function metricOption(value: unknown): RaidTallyMetric {
  return value === 'launched' || value === 'succeeded' ? value : 'both';
}

function countText(value: number): string {
  return formatScalar(value, { decimals: 0 });
}

interface RaidTallyExactBase {
  readonly branchId: string;
  readonly top: number | null;
  readonly all: boolean;
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly metric: RaidTallyMetric;
}

function buildExactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as RaidTallyExactBase;
  if (cfg.top != null || cfg.all) {
    return {
      branchId: cfg.branchId,
      hour,
      countries: [],
      ...(cfg.top != null ? { top: cfg.top } : {}),
      metric: cfg.metric,
    };
  }
  const activeTags = cfg.membership ? cfg.tags.filter(tag => inHourRanges(cfg.membership![tag], hour)) : cfg.tags;
  if (!activeTags.length) return null;
  return {
    branchId: cfg.branchId,
    hour,
    countries: activeTags.map(countryFromKey),
    metric: cfg.metric,
  };
}

export const raidTallyRaceProjection: BarsProjection<RaidTallyRaceOptions> = registerFamilyProjection({
  kind: 'raid-tally',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, { ...options, candidates: [] });
  },
  project(readView: BoundedReadView, options: RaidTallyRaceOptions): BarsModel {
    const hour = readView.hour;
    const ranked = options.top != null || options.all;
    const unit = options.metric === 'succeeded' ? 'Succeeded raids' : options.metric === 'launched' ? 'Launched raids' : 'Raids';
    const baseModel = {
      form: 'ordered' as const,
      hour,
      animateOrder: true,
      showAxis: true,
      scaleUnit: unit,
      scaleFormat: { decimals: 0 },
      ...(options.metric === 'both' ? { caption: 'Solid: succeeded · hatched: other launched' } : {}),
    };

    if (!ranked && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return { ...baseModel, rows: [], empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable' };
    }
    const refused = readView.refused('main');
    if (refused) {
      return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
    }
    const panel = readView.panel('main');
    if (panel === undefined) {
      return { ...baseModel, rows: [] };
    }
    const reply = validateRaidTallyAt(panel);
    const warnings = [
      ...reply.warnings,
      ...(reply.unattributed > 0 ? [`${formatScalar(reply.unattributed, { decimals: 0 })} raids have no named raider and are not counted.`] : []),
    ];
    if (reply.available === false) {
      return {
        ...baseModel,
        rows: [],
        empty: 'No raid types available.',
        ...(reply.unavailable ? { emptyDetail: reply.unavailable } : {}),
        emptyKind: 'unavailable',
        warnings,
      };
    }
    if (reply.missing !== null) {
      return { ...baseModel, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings };
    }

    const rows: BarsDataRow[] = [];
    for (const item of reply.rows as readonly RaidTallyAtRow[]) {
      const key = countryKey(item.country);
      const displayTag = options.holders ? (holderTagAt(options.holders[key], hour) ?? item.tag) : item.tag;
      if (!ranked && options.membership) {
        const inMember = inHourRanges(options.membership[key], hour) || inHourRanges(options.membership[displayTag], hour);
        if (!inMember) continue;
      }
      const pres = countryRowPresentation(readView, displayTag);
      if (item.missing !== null) {
        rows.push({
          kind: 'data',
          key,
          subject: displayTag,
          label: pres.label,
          labelMissing: pres.labelMissing,
          media: pres.media,
          ...(pres.color ? { color: pres.color } : {}),
          value: { value: notRecorded(item.missing), format: { decimals: 0 } },
          numeric: null,
          tooltip: {
            title: pres.labelMissing ? 'Name not recorded' : pres.label,
            hour,
            missingReason: item.missing,
            items: [{ label: unit, missingReason: item.missing }],
            note: NOTE,
          },
        });
        continue;
      }

      const primary = options.metric === 'succeeded' ? item.succeeded : item.launched;
      const value = primary === 0 ? zeroValue() : knownValue(primary);
      const barFill = pres.color ?? colour(DEFAULT_CARD_SURFACE, 0);
      let segments: BarsSegment[] | undefined;
      if (options.metric === 'both' && item.launched > 0) {
        const succeeded = Math.min(item.launched, item.succeeded);
        const other = item.launched - succeeded;
        segments = [];
        if (succeeded > 0) {
          segments.push({ key: 'succeeded', label: 'Succeeded', share: succeeded / item.launched, value: knownValue(succeeded), format: { decimals: 0 }, color: barFill });
        }
        if (other > 0) {
          segments.push({ key: 'other', label: 'Other launched', share: other / item.launched, value: knownValue(other), format: { decimals: 0 }, color: barFill, hatched: true });
        }
      }
      const tooltipItems: TooltipListItem[] = [
        { label: 'Launches', value: countText(item.launched) },
        { label: 'Successes', value: countText(item.succeeded) },
      ];
      rows.push({
        kind: 'data',
        key,
        subject: displayTag,
        label: pres.label,
        labelMissing: pres.labelMissing,
        media: pres.media,
        ...(pres.color ? { color: pres.color } : {}),
        value: { value, format: { decimals: 0 }, label: unit },
        numeric: primary,
        ...(segments?.length ? { segments } : {}),
        readout: {
          primary: options.metric === 'both'
            ? `${countText(item.launched)} \u00b7 ${countText(item.succeeded)}`
            : countText(primary),
        },
        tooltip: {
          title: pres.labelMissing ? 'Name not recorded' : pres.label,
          hour,
          items: tooltipItems,
          note: NOTE,
        },
      });
    }

    if (!ranked) {
      rows.sort((a, b) => {
        const aMiss = a.numeric === null || a.numeric === undefined;
        const bMiss = b.numeric === null || b.numeric === undefined;
        if (aMiss !== bMiss) return aMiss ? 1 : -1;
        if (!aMiss && !bMiss && a.numeric !== b.numeric) return b.numeric! - a.numeric!;
        return compareTags(a.subject ?? a.key, b.subject ?? b.key);
      });
    }

    const empty = !rows.length
      ? (ranked
        ? `No raids ${options.metric === 'both' ? 'launched or succeeded' : options.metric} yet.`
        : 'No raid counts recorded at this time.')
      : undefined;

    return {
      ...baseModel,
      rows,
      ...(empty ? { empty, emptyKind: 'empty' as const } : {}),
      warnings,
    };
  },
}, 'race');

function raidTallyRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const metric = metricOption(ctx.options.metric);
  const top = ctx.countries.top;
  const all = ctx.countries.all;
  const emptySelection = top == null && !all && !ctx.countries.tags.length;
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : all
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const base: RaidTallyExactBase = {
    branchId: ctx.branch.id,
    top,
    all,
    tags: ctx.countries.tags,
    membership: top == null && !all ? ctx.countries.membership : null,
    metric,
  };
  return {
    owner: `raid-tally:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${metric}`,
    entries: emptySelection ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'raid-tally-at',
        exactProduct: 'raid-tally-at',
        followQuery: null,
        params: base as unknown as Readonly<Record<string, unknown>>,
        buildExactParams,
        dynamicKeys: ['hour'],
        provenInterval,
        validate: validateRaidTallyAt,
      },
    }],
    allowEmptyFollow: emptySelection,
  };
}

export const raidTallyRaceDeclaration: BarsWidgetDeclaration<RaidTallyRaceOptions> = {
  family: 'bars',
  kind: 'raid-tally',
  label: 'Raid tally',
  subject: 'raids',
  emptyCampaign: 'Load a campaign to see its raid tally.',
  icon: 'target',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: raidTallyRaceSources,
  projection: raidTallyRaceProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      metric: metricOption(ctx.options.metric),
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
