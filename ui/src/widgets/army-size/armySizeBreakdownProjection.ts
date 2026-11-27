import { colour, nudgeApart, readableColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { auditRow, type FamilyAuditRow } from '../../families/auditRow.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import { validateForceAtReply } from '../../families/bars/forceAtRace.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartArtSlot, ChartGap, ChartHoverDetail, ChartStackBand } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  exactHoverReadings,
  headReading,
  missingSpansOf,
} from '../../families/time-chart/seriesHelpers.ts';
import { formatScalar, notRecordedLabel } from '../../families/valueFormat.ts';
import type {
  ArmyAttribution,
  ArmyBreakdownReply,
  ArmyMeasure,
  ArmyMixRun,
  ArmyRun,
  ForceAtReply,
} from '../../generated/dashboard';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import {
  armyBreakdownProof,
  armyGroupLabel,
  armyMeasure,
  armyMetadataParams,
  measureName,
  validateArmyBreakdownReply,
  validateArmyMetadata,
  type ArmyMetadataReply,
} from './armySizeMeta.ts';
import { identityNameOr } from '../../families/table/identityCell.ts';

const SERIES_MEASURES: Readonly<Record<ArmyMeasure, string>> = {
  manpower: 'Manpower',
  divisions: 'Divisions',
  strength: 'Strength',
  battalions: 'Battalions',
};

const UNATTRIBUTED_KEY = '__unattributed__';
const UNATTRIBUTED_LABEL = 'Unattributed';

export interface ArmySizeBreakdownOptions {
  readonly subject: string | null;
  readonly measure: ArmyMeasure | null;
  readonly unavailableReason: string | null;
  readonly countFor: ArmyAttribution;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly yMin: number | null;
  readonly yMax: number | null;
  readonly log: boolean;
  readonly auxWarnings: readonly string[];
}

function numericOption(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

const armyCountFor = (value: unknown): ArmyAttribution => (value === 'host' ? 'host' : 'owner');

type ResolvedMeasure =
  | { readonly kind: 'pending' }
  | { readonly kind: 'unavailable'; readonly reason: string }
  | { readonly kind: 'ready'; readonly measure: ArmyMeasure };

function resolveBreakdownMeasure(rawOption: unknown, aux: ArmyMetadataReply | undefined): ResolvedMeasure {
  if (!aux) return { kind: 'pending' };
  if (!aux.available) return { kind: 'unavailable', reason: aux.missing[0]?.reason ?? 'Not in this recording' };
  if (typeof rawOption === 'string' && rawOption.length > 0) {
    if (aux.measures.includes(rawOption as ArmyMeasure)) {
      return { kind: 'ready', measure: rawOption as ArmyMeasure };
    }
    return { kind: 'unavailable', reason: 'Not in this recording' };
  }
  const chosen = aux.measures.includes('battalions') ? 'battalions' : armyMeasure('', aux.measures);
  return { kind: 'ready', measure: chosen };
}

function runAtHour(runs: readonly ArmyRun[], hour: number): ArmyRun | null {
  for (const run of runs) {
    const last = run[2] ?? run[0];
    if (run[0] <= hour && hour <= last) return run;
  }
  return null;
}

function mixAtHour(runs: readonly ArmyMixRun[], hour: number): ArmyMixRun | null {
  for (const run of runs) {
    if (run.firstHour <= hour && hour <= run.lastHour) return run;
  }
  return null;
}

function missingAtHour(reply: ArmyBreakdownReply, hour: number): string | null {
  for (const span of reply.missing) {
    if (span.firstHour <= hour && hour <= span.lastHour) return span.reason;
  }
  return null;
}

function assignGroupColours(keys: readonly string[], surface: string): ReadonlyMap<string, string> {
  const out = new Map<string, string>();
  const bg = parseColor(surface)!;
  const rawRgbs = keys.map((_, index) => parseColor(colour(surface, index)) ?? { r: 120, g: 160, b: 220 });
  const distinct = nudgeApart(rawRgbs, bg);
  keys.forEach((key, index) => out.set(key, toHex(distinct[index]!)));
  return out;
}

function buildBattalionHoverDetail(
  readView: BoundedReadView,
  subject: string,
  breakdown: ArmyBreakdownReply,
  colours: ReadonlyMap<string, string>,
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<ForceAtReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !hover.data) return null;
  const { hour, data: reply } = hover;
  if (reply.missing !== null) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Battalions', missingReason: reply.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const expected = countryFromKey(subject);
  const row = reply.rows.find(candidate => (
    typeof expected === 'number'
      ? candidate.slot === expected || candidate.country === expected
      : candidate.country === expected
  ));
  if (!row) {
    const miss = missingAtHour(breakdown, Math.floor(hour));
    return {
      hour,
      replacesSeries: true,
      items: [miss
        ? { label: 'Battalions', missingReason: miss, tone: 'muted' }
        : { label: 'Battalions', value: '0', tone: 'muted' }],
    };
  }
  if (row.missing !== null) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Battalions', missingReason: row.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const totalText = row.total === null ? notRecordedLabel() : formatScalar(row.total, 'integer');
  const items: TooltipListItem[] = [
    row.total === null
      ? { label: 'Battalions', missingReason: notRecordedLabel(), tone: 'muted' }
      : { label: 'Battalions', value: totalText, strong: true },
  ];
  if (row.partsMissing !== null) {
    items.push({ label: 'Unit groups', missingReason: row.partsMissing, tone: 'muted', divider: true });
    return { hour, replacesSeries: true, items };
  }
  let partsSum = 0;
  const partItems: TooltipListItem[] = [];
  reply.parts.forEach((part, index) => {
    const val = row.values[index] ?? null;
    if (val === null) {
      partItems.push({ label: armyGroupLabel(part), missingReason: notRecordedLabel(), tone: 'muted' });
      return;
    }
    partsSum += val;
    if (val > 0) {
      const swatch = colours.get(part.key);
      partItems.push({
        label: armyGroupLabel(part),
        value: formatScalar(val, 'integer'),
        ...(swatch ? { swatch } : {}),
      });
    }
  });
  if (row.total !== null && row.total > partsSum) {
    const swatch = colours.get(UNATTRIBUTED_KEY);
    partItems.push({
      label: UNATTRIBUTED_LABEL,
      value: formatScalar(row.total - partsSum, 'integer'),
      ...(swatch ? { swatch } : {}),
    });
  }
  items.push(...partItems.map((item, idx) => (idx === 0 ? { ...item, divider: true } : item)));
  return { hour, replacesSeries: true, items };
}

function projectBattalionsStack(readView: BoundedReadView, options: ArmySizeBreakdownOptions) {
  const hour = readView.hour;
  const floorHour = Math.floor(hour);
  const baseAxis = {
    format: 'compact' as const,
    decimals: 0,
    ...(options.yMin !== null ? { min: options.yMin } : {}),
    ...(options.yMax !== null ? { max: options.yMax } : {}),
  };
  const reply = readView.panel<ArmyBreakdownReply>('breakdown');
  if (!reply) {
    return {
      kind: 'army-size',
      measureTitle: 'Battalions by unit group',
      mode: 'stack' as const,
      hour,
      extent: options.extent,
      yAxis: baseAxis,
      series: [],
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      warnings: [...options.auxWarnings],
      auditRows: [],
    };
  }

  const warnings = [...new Set([...options.auxWarnings, ...reply.warnings])];
  if (!reply.available && reply.runs.length === 0) {
    return {
      kind: 'army-size',
      measureTitle: 'Battalions by unit group',
      mode: 'stack' as const,
      hour,
      extent: options.extent,
      yAxis: baseAxis,
      series: [],
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      warnings,
      empty: { message: reply.missing[0]?.reason ?? 'Army breakdown is not recorded.', kind: 'unavailable' as const },
      auditRows: [],
    };
  }

  const peaks = reply.types.map(() => 0);
  let hasUnattributed = false;
  for (const run of reply.mix) {
    let sum = 0;
    for (let i = 0; i < reply.types.length; i++) {
      const val = run.values[i] ?? 0;
      if (val > peaks[i]!) peaks[i] = val;
      sum += val;
    }
    const totalRun = runAtHour(reply.runs, run.firstHour);
    const totalVal = totalRun?.[1] ?? null;
    if (totalVal !== null && totalVal > sum) hasUnattributed = true;
  }

  const orderedIndices = reply.types
    .map((_, idx) => idx)
    .filter(idx => peaks[idx]! > 0)
    .sort((a, b) => peaks[b]! - peaks[a]! || reply.types[a]!.key.localeCompare(reply.types[b]!.key));

  const bandSpecs: { readonly key: string; readonly name: string; readonly typeIndex: number | null }[] = [
    ...orderedIndices.map(idx => ({
      key: reply.types[idx]!.key,
      name: armyGroupLabel(reply.types[idx]!),
      typeIndex: idx,
    })),
    ...(hasUnattributed ? [{ key: UNATTRIBUTED_KEY, name: UNATTRIBUTED_LABEL, typeIndex: null }] : []),
  ];

  const colours = assignGroupColours(bandSpecs.map(spec => spec.key), options.surface);
  const hoverDetail = buildBattalionHoverDetail(readView, options.subject!, reply, colours);
  const headRun = runAtHour(reply.runs, floorHour);
  const headMix = mixAtHour(reply.mix, floorHour);
  const headMissing = missingAtHour(reply, floorHour);

  if (bandSpecs.length === 0) {
    if (headMissing !== null && headRun === null) {
      return {
        kind: 'army-size',
        measureTitle: 'Battalions by unit group',
        mode: 'stack' as const,
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        gaps: [...options.gaps, ...reply.missing],
        hoverDetail,
        warnings,
        empty: { message: headMissing, kind: 'unavailable' as const },
        auditRows: [],
      };
    }
    const totalAudit = headRun && headRun[1] !== null ? [auditRow('__total__', formatScalar(headRun[1], 'integer'))] : [];
    const partsAudit = headMissing ? [auditRow('__parts__', headMissing)] : [];
    return {
      kind: 'army-size',
      measureTitle: 'Battalions by unit group',
      mode: 'stack' as const,
      hour,
      extent: options.extent,
      yAxis: baseAxis,
      series: [],
      gaps: [...options.gaps, ...reply.missing],
      hoverDetail,
      warnings,
      empty: headMissing
        ? { message: headMissing, kind: 'unavailable' as const }
        : { message: 'No battalions recorded at this date.', kind: 'empty' as const },
      auditRows: [...totalAudit, ...partsAudit],
    };
  }

  const cutsSet = new Set<number>();
  for (const run of reply.mix) {
    cutsSet.add(run.firstHour);
    if (run.lastHour < floorHour) cutsSet.add(run.lastHour + 1);
  }
  for (const run of reply.runs) {
    cutsSet.add(run[0]);
    const last = run[2] ?? run[0];
    if (last < floorHour) cutsSet.add(last + 1);
  }
  for (const span of reply.missing) {
    cutsSet.add(span.firstHour);
    if (span.lastHour < floorHour) cutsSet.add(span.lastHour + 1);
  }
  const cuts = [...cutsSet].filter(h => h <= floorHour).sort((a, b) => a - b);

  const hours: number[] = [];
  const bandValues: (number | null)[][] = bandSpecs.map(() => []);
  const totalValues: (number | null)[] = [];
  const pushNull = (at: number) => {
    if (hours.length > 0 && hours[hours.length - 1] === at) return;
    hours.push(at);
    for (const vals of bandValues) vals.push(null);
    totalValues.push(null);
  };

  for (const at of cuts) {
    const mix = mixAtHour(reply.mix, at);
    const totalRun = runAtHour(reply.runs, at);
    const totalVal = totalRun?.[1] ?? null;
    if (!mix || totalVal === null) {
      pushNull(at);
      continue;
    }
    let knownSum = 0;
    for (const val of mix.values) knownSum += val ?? 0;
    const residual = Math.max(0, totalVal - knownSum);
    hours.push(at);
    let stackSum = 0;
    bandSpecs.forEach((spec, bandIdx) => {
      const val = spec.typeIndex === null ? residual : (mix.values[spec.typeIndex] ?? 0);
      bandValues[bandIdx]!.push(val);
      stackSum += val;
    });
    totalValues.push(stackSum);
  }

  const headTotalVal = headRun?.[1] ?? null;
  let headPartsSum = 0;
  if (headMix) {
    for (const val of headMix.values) headPartsSum += val ?? 0;
  }
  const headResidual = headMix && headTotalVal !== null ? Math.max(0, headTotalVal - headPartsSum) : null;

  const bands: ChartStackBand[] = bandSpecs.map((spec, index) => {
    const color = colours.get(spec.key) ?? colour(options.surface, index);
    const headVal = headMix
      ? (spec.typeIndex === null ? headResidual : (headMix.values[spec.typeIndex] ?? 0))
      : null;
    return {
      id: spec.key,
      key: spec.key,
      name: spec.name,
      color,
      values: bandValues[index]!,
      endLabel: headVal !== null && headVal > 0
        ? { colorBar: color, name: spec.name, fullName: spec.name, value: formatScalar(headVal, 'integer') }
        : null,
    };
  });

  const gaps: ChartGap[] = [
    ...options.gaps,
    ...reply.missing.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour, reason: span.reason })),
  ];

  const auditRows: FamilyAuditRow[] = [];
  if (headTotalVal !== null) {
    auditRows.push(auditRow('__total__', formatScalar(headTotalVal, 'integer')));
  } else if (headMissing !== null) {
    auditRows.push(auditRow('__total__', headMissing));
  }
  if (headMix) {
    bandSpecs.forEach(spec => {
      const val = spec.typeIndex === null ? headResidual : (headMix.values[spec.typeIndex] ?? 0);
      if (val !== null && val > 0) {
        auditRows.push(auditRow(spec.key, formatScalar(val, 'integer')));
      }
    });
  } else if (headMissing !== null && headTotalVal !== null) {
    auditRows.push(auditRow('__parts__', headMissing));
  }

  return {
    kind: 'army-size',
    measureTitle: 'Battalions by unit group',
    mode: 'stack' as const,
    hour,
    extent: options.extent,
    yAxis: baseAxis,
    series: [],
    stack: {
      hours,
      bands,
      step: true,
      total: { values: totalValues },
    },
    gaps: gaps.length > 0 ? gaps : undefined,
    hoverDetail,
    warnings,
    auditRows,
  };
}

function projectScalarMeasure(readView: BoundedReadView, options: ArmySizeBreakdownOptions, measure: ArmyMeasure) {
  const hour = readView.hour;
  const subject = options.subject!;
  const isStrength = measure === 'strength';
  const decimals = isStrength ? 5 : 0;
  const displayFormat = isStrength ? { decimals: 0, maxDecimals: 2 } : ('integer' as const);
  const title = measureName(measure);
  const yAxis = {
    format: 'compact' as const,
    decimals: isStrength ? 2 : 0,
    ...(options.log ? { log: true } : {}),
    ...(options.yMin !== null ? { min: options.yMin } : {}),
    ...(options.yMax !== null ? { max: options.yMax } : {}),
  };

  const bounded = readView.series('main')?.get(subject) ?? null;
  const head = headReading(readView, 'main', { tag: subject }, decimals);
  const who = readView.who('country', subject, hour);
  const tagVal = who.tag?.status === 'ready' ? who.tag.value : subject;
  const name = identityNameOr(readView, 'country', subject, tagVal, hour);
  const flag: ChartArtSlot = { art: flagChoiceArt(readView, who.flagChoice), tag: tagVal };
  const color = who.colour?.status === 'ready'
    ? readableColour(who.colour.value, options.surface)
    : colour(options.surface, 0);

  const exact = exactHoverReadings(readView, 'hover', decimals)?.[0];
  const hoverDetail: ChartHoverDetail | null = exact
    ? {
        hour: exact.hour,
        replacesSeries: true,
        items: [
          exact.value === null
            ? { label: title, missingReason: exact.reason ?? notRecordedLabel(), tone: 'muted' }
            : { label: title, swatch: color, value: formatScalar(exact.value, displayFormat), strong: true },
        ],
      }
    : null;

  const missing = missingSpansOf(bounded);
  const headValue = head && 'value' in head ? head.value : null;
  const auditRows: FamilyAuditRow[] = head === null
    ? []
    : 'value' in head
      ? [auditRow(subject, formatScalar(head.value, displayFormat))]
      : [auditRow(subject, head.missing)];

  return {
    kind: 'army-size',
    measureTitle: title,
    mode: 'lines' as const,
    hour,
    extent: options.extent,
    yAxis,
    series: bounded
      ? [{
          id: subject,
          key: subject,
          name,
          color,
          role: 'primary' as const,
          area: true,
          ...(bounded.runs ? { runs: bounded.runs } : { buckets: bounded.buckets }),
          ...(missing ? { missing } : {}),
          endLabel: headValue !== null
            ? { flag, name, fullName: name, value: formatScalar(headValue, displayFormat) }
            : null,
        }]
      : [],
    gaps: options.gaps.length > 0 ? options.gaps : undefined,
    hoverDetail,
    warnings: [...options.auxWarnings],
    auditRows,
  };
}

export const armySizeBreakdownProjection: TimeChartProjection<ArmySizeBreakdownOptions> = registerFamilyProjection({
  kind: 'army-size',
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'step',
  measureTitle: 'Army breakdown',
  nextChange(readView, options) {
    if (options.measure === 'battalions') {
      return readView.panel<ArmyBreakdownReply>('breakdown')?.nextChange ?? Infinity;
    }
    return Infinity;
  },
  project(readView: BoundedReadView, options: ArmySizeBreakdownOptions) {
    const hour = readView.hour;
    const baseAxis = {
      format: 'compact' as const,
      decimals: 0,
      ...(options.yMin !== null ? { min: options.yMin } : {}),
      ...(options.yMax !== null ? { max: options.yMax } : {}),
    };
    if (options.unavailableReason !== null) {
      return {
        kind: 'army-size',
        measureTitle: 'Army breakdown',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        warnings: [...options.auxWarnings],
        empty: { message: options.unavailableReason, kind: 'unavailable' },
        auditRows: [],
      };
    }
    if (!options.subject || !options.measure) {
      return {
        kind: 'army-size',
        measureTitle: 'Army breakdown',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: baseAxis,
        series: [],
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        warnings: [...options.auxWarnings],
        auditRows: [],
      };
    }
    if (options.measure === 'battalions') {
      return projectBattalionsStack(readView, options);
    }
    return projectScalarMeasure(readView, options, options.measure);
  },
}, 'breakdown');

export const armySizeBreakdownDeclaration: TimeChartWidgetDeclaration<ArmySizeBreakdownOptions, ArmyMetadataReply> = {
  family: 'time-chart',
  kind: 'army-size',
  label: 'Army breakdown',
  subject: 'army size',
  emptyCampaign: 'Load a campaign to see its army sizes.',
  icon: 'shield',
  countries: { mode: 'single', choose: 'Choose a country to see its army breakdown.' },
  aux: {
    product: 'army-size',
    label: 'the army types',
    params: track => ({ ...armyMetadataParams(track.branch.id) }),
    validate: validateArmyMetadata,
  },
  sources(ctx: FamilyPlanContext<ArmyMetadataReply>): SourcePlan {
    const subject = ctx.countrySubject;
    const resolved = resolveBreakdownMeasure(ctx.options.measure, ctx.aux);
    const countFor = armyCountFor(ctx.options.countFor);
    const owner = `army-size:breakdown:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${subject ?? ''}:${resolved.kind === 'ready' ? resolved.measure : resolved.kind}:${countFor}`;
    if (!subject || resolved.kind !== 'ready') {
      return { owner, entries: [], allowEmptyFollow: resolved.kind === 'unavailable' };
    }
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const country = countryFromKey(subject);
    if (resolved.measure === 'battalions') {
      return {
        owner,
        entries: [
          {
            name: 'breakdown',
            ...common,
            read: {
              shape: 'branch-to-hour',
              coverage: { mode: 'keyed', keys: [subject] },
              product: 'army-breakdown',
              followQuery: null,
              params: { branchId: ctx.branch.id, country, countFor },
              buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
              provenInterval: armyBreakdownProof,
              dynamicKeys: ['hour'],
              validate: validateArmyBreakdownReply,
            },
          },
          {
            name: 'hover',
            ...common,
            read: {
              shape: 'exact-hover',
              product: 'army-at',
              params: { branchId: ctx.branch.id, countries: [country], countFor },
              validate: validateForceAtReply,
            },
          },
        ],
      };
    }
    const family = { kind: 'army-totals', measure: `${countFor}${SERIES_MEASURES[resolved.measure]}` };
    const reads = [{ country }];
    return {
      owner,
      entries: [
        {
          name: 'main',
          ...common,
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'keyed', keys: [subject] },
            family,
            tags: [subject],
            headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family, reads } },
            params: { branchId: ctx.branch.id, family, reads },
            exactProduct: 'series-state',
          },
        },
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'series-state',
            params: { branchId: ctx.branch.id, family, reads },
          },
        },
      ],
    };
  },
  projection: armySizeBreakdownProjection,
  projectionOptions(ctx, { extent, surface }) {
    const resolved = resolveBreakdownMeasure(ctx.options.measure, ctx.aux);
    return {
      subject: ctx.countrySubject,
      measure: resolved.kind === 'ready' ? resolved.measure : null,
      unavailableReason: resolved.kind === 'unavailable' ? resolved.reason : null,
      countFor: armyCountFor(ctx.options.countFor),
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      yMin: numericOption(ctx.options.yMin),
      yMax: numericOption(ctx.options.yMax),
      log: ctx.options.log === true,
      auxWarnings: ctx.aux?.warnings ?? [],
    };
  },
};
