import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { barsLabelText, countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import { countryRowFromAction } from '../../families/host/installed.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import { formatDateOnly, formatScalar, type ValueFormat } from '../../families/valueFormat.ts';
import type {
  CountryRef,
  EquipmentProducedRaceReply,
  EquipmentProducedRaceRow,
  GroupSelection,
  ProducedCount,
  ProducedAtReply,
  ProducedMeasure,
  ProducedTypesReply,
  ProducedUnknown,
} from '../../generated/dashboard';

export interface EquipmentProducedRowMeta {
  readonly tag: string;
  readonly key: string;
  readonly counted: string;
}

export function detailCountry(meta: EquipmentProducedRowMeta): CountryRef {
  return countryFromKey(meta.counted);
}

export function detailRowFromAction(action: InstalledAction): EquipmentProducedRowMeta | null {
  const row = countryRowFromAction(action);
  if (!row) return null;
  const counted = (action.meta as { readonly counted?: unknown } | null | undefined)?.counted;
  return typeof counted === 'string' ? { ...row, counted } : null;
}

export function detailEmptyText(reply: Pick<ProducedAtReply, 'models' | 'icTotal'>, hour: number): string | null {
  if (reply.models.length) return null;
  const reason = reply.icTotal.reason;
  return reason !== null ? UNKNOWN_WORDS[reason] : `Nothing delivered at ${formatDateOnly(hour)}.`;
}

export interface EquipmentProducedRaceOptions {
  readonly count: ProducedCount;
  readonly measure: ProducedMeasure;
  readonly equipmentLabel: string;
  readonly measureLabel: string;
  readonly noDataMessage: string | null;
  readonly group: boolean;
  readonly top: number | null;
}

export const UNKNOWN_WORDS: Readonly<Record<ProducedUnknown, string>> = {
  notRecorded: 'Production was not recorded for this country then.',
  unitsInexact: 'A production line left a delivery out of its count.',
  spendingUnavailable: 'The factory IC a line applied was not recorded.',
  lostOutput: 'A production line went without its final count.',
  missedPublication: 'Production was not read at some hours.',
  typeNotRecorded: 'Designs whose type was not recorded hold output that could be of this type.',
  windowIncomplete: 'The window reaches before this country\'s recorded production.',
  observationRestarted: 'Production counting began again inside the window.',
  rocketTableUnavailable: 'Rocket site output was not recorded then.',
  rocketInputInexact: 'Rocket site output was not counted exactly.',
  rocketOwnerUnreadable: 'Rocket site output of an unknown country was recorded.',
  outOfRange: 'The amount is too large to show exactly.',
};

const MEASURE_LABELS: Readonly<Record<ProducedMeasure, string>> = {
  day: 'Per day',
  week: 'Per week',
  month: 'Per month',
  total: 'Since recording start',
};

const isObj = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

export function validateEquipmentProducedTypes(value: unknown): ProducedTypesReply {
  if (
    !isObj(value)
    || typeof value.branchId !== 'string'
    || !Array.isArray(value.types)
    || !(value.mostDelivered === null || typeof value.mostDelivered === 'string')
    || !Array.isArray(value.warnings)
  ) {
    throw new Error('The equipment types reply is malformed.');
  }
  return value as unknown as ProducedTypesReply;
}

export function validateEquipmentProducedRace(value: unknown): EquipmentProducedRaceReply {
  if (
    !isObj(value)
    || typeof value.branchId !== 'string'
    || !Number.isSafeInteger(value.hour)
    || !Number.isSafeInteger(value.firstHour)
    || !Number.isSafeInteger(value.lastHour)
    || (value.count !== 'units' && value.count !== 'ic')
    || !Number.isSafeInteger(value.decimals)
    || !Array.isArray(value.types)
    || !Array.isArray(value.rows)
    || !(value.readHour === null || Number.isSafeInteger(value.readHour))
    || !(value.observationHour === null || Number.isSafeInteger(value.observationHour))
    || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.warnings)
  ) {
    throw new Error('The equipment produced race reply is malformed.');
  }
  return value as unknown as EquipmentProducedRaceReply;
}

export function provenInterval(data: unknown): readonly [number, number] | null {
  const reply = data as Partial<EquipmentProducedRaceReply> | null | undefined;
  return reply && typeof reply.firstHour === 'number' && typeof reply.lastHour === 'number' && reply.firstHour <= reply.lastHour
    ? [reply.firstHour, reply.lastHour]
    : null;
}

export function measureOption(value: unknown): ProducedMeasure {
  return value === 'total' || value === 'month' || value === 'week' ? value : 'day';
}

export function countOption(value: unknown): ProducedCount {
  return value === 'ic' ? 'ic' : 'units';
}

export function windowUnavailable(measure: ProducedMeasure): string {
  if (measure === 'day') return 'A full day of production data is unavailable.';
  if (measure === 'week') return 'A full week of production data is unavailable.';
  if (measure === 'month') return 'A full month of production data is unavailable.';
  return 'No production data at this time.';
}

export function captionFor(count: ProducedCount, measure: ProducedMeasure, equipment: string, observationHour: number | null): string {
  const what = count === 'ic' ? `IC · ${equipment}` : equipment;
  if (measure === 'total') {
    return observationHour !== null ? `${what} · since ${formatDateOnly(observationHour)}` : `${what} · since recording start`;
  }
  return `${what} · ${MEASURE_LABELS[measure].toLowerCase()}`;
}

export interface ProducedRaceExactBase {
  readonly branchId: string;
  readonly top: number | null;
  readonly all: boolean;
  readonly group: GroupSelection | null;
  readonly tags: readonly string[];
  readonly count: ProducedCount;
  readonly measure: ProducedMeasure;
  readonly types: readonly string[];
  readonly refits: boolean;
  readonly fromHour?: number;
}

export function buildExactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as ProducedRaceExactBase;
  const common = {
    branchId: cfg.branchId,
    hour,
    count: cfg.count,
    measure: cfg.measure,
    types: [...cfg.types],
    refits: cfg.refits,
    ...(cfg.fromHour !== undefined ? { fromHour: cfg.fromHour } : {}),
  };
  if (cfg.top != null || cfg.all) {
    return { ...common, countries: [], ...(cfg.top != null ? { top: cfg.top } : {}) };
  }
  if (cfg.group) {
    return { ...common, countries: [], group: cfg.group };
  }
  if (!cfg.tags.length) return null;
  return { ...common, countries: cfg.tags.map(countryFromKey) };
}

export const equipmentProducedRaceProjection: BarsProjection<EquipmentProducedRaceOptions> = registerFamilyProjection({
  kind: 'equipment-produced',
  family: 'bars',
  project(readView: BoundedReadView, options: EquipmentProducedRaceOptions): BarsModel {
    const hour = readView.hour;
    const valueFormat: ValueFormat = options.count === 'ic' ? { decimals: 0, maxDecimals: 2 } : { decimals: 0 };
    const baseModel = {
      form: 'ordered' as const,
      hour,
      animateOrder: true,
      showAxis: true,
      scaleUnit: options.count === 'ic' ? 'IC' : options.equipmentLabel,
      scaleFormat: valueFormat,
      caption: captionFor(options.count, options.measure, options.equipmentLabel, null),
    };

    if (options.noDataMessage !== null) {
      return { ...baseModel, rows: [], empty: options.noDataMessage, emptyKind: 'unavailable' };
    }
    const refused = readView.refused('main');
    if (refused) {
      return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
    }
    const panel = readView.panel('main');
    if (panel === undefined) {
      return { ...baseModel, rows: [] };
    }
    const reply = validateEquipmentProducedRace(panel);
    if (reply.missing !== null) {
      return { ...baseModel, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }

    const scale = 10 ** reply.decimals;
    const measureLabel = options.measure === 'total' && reply.observationHour !== null ? `Since ${formatDateOnly(reply.observationHour)}` : options.measureLabel;
    const rows: BarsDataRow[] = [];
    for (const item of reply.rows as readonly EquipmentProducedRaceRow[]) {
      const key = countryKey(item.country);
      const pres = countryRowPresentation(readView, key);
      const subjectTag = item.tag;
      const meta: EquipmentProducedRowMeta = { tag: subjectTag, key, counted: countryKey(item.counted) };
      const value = item.value === null
        ? notRecorded(item.unknown ? UNKNOWN_WORDS[item.unknown] : 'Not recorded at this hour')
        : reply.decimals === 0
          ? (item.value === 0 ? zeroValue() : knownValue(item.value))
          : (item.value === 0 ? zeroValue() : knownValue(item.value / scale));
      const numeric = item.value === null ? null : (reply.decimals === 0 ? item.value : item.value / scale);
      const tooltipItems: TooltipListItem[] = [
        value.kind === 'notRecorded'
          ? { label: measureLabel, missingReason: value.reason }
          : { label: measureLabel, value: formatScalar(numeric ?? 0, valueFormat) },
      ];
      if (reply.readHour !== null) {
        tooltipItems.push({ label: 'Read on', value: formatDateOnly(reply.readHour) });
      }
      const note = reply.observationHour !== null && options.measure === 'total'
        ? `Counted since ${formatDateOnly(reply.observationHour)}.`
        : undefined;
      rows.push({
        kind: 'data',
        key,
        subject: subjectTag,
        label: pres.label,
        media: pres.media,
        ...(pres.color ? { color: pres.color } : {}),
        value: { value, format: valueFormat, label: measureLabel },
        numeric,
        tooltip: {
          title: barsLabelText(pres.label),
          hour,
          items: tooltipItems,
          ...(value.kind === 'notRecorded' ? { missingReason: value.reason } : {}),
          ...(note ? { note } : {}),
        },
        meta,
      });
    }

    const empty = !rows.length
      ? (options.group
        ? 'No country of this group at this date.'
        : reply.readHour !== null
          ? (options.measure === 'total' ? `No production by ${formatDateOnly(hour)}.` : `No production ${MEASURE_LABELS[options.measure].toLowerCase()}.`)
          : windowUnavailable(options.measure))
      : undefined;

    return {
      ...baseModel,
      caption: captionFor(options.count, options.measure, options.equipmentLabel, reply.observationHour),
      ...(reply.readHour !== null ? { captionRight: `Read ${formatDateOnly(reply.readHour)}` } : {}),
      rows,
      ...(empty ? { empty, emptyKind: (options.group || reply.readHour !== null) ? 'empty' as const : 'unavailable' as const } : {}),
      warnings: reply.warnings,
    };
  },
}, 'race');

export function resolvedEquipment(ctx: FamilyPlanContext<ProducedTypesReply>): {
  readonly count: ProducedCount;
  readonly measure: ProducedMeasure;
  readonly refits: boolean;
  readonly shownTypes: readonly string[];
  readonly equipmentLabel: string;
  readonly noDataMessage: string | null;
} {
  const count = countOption(ctx.options.count);
  const measure = measureOption(ctx.options.measure);
  const refits = ctx.options.refits === true;
  const included = contentSet(ctx.options.included).map(contentKey);
  const pickingDefault = count === 'units' && !included.length;
  const mostType = ctx.aux?.mostDelivered ?? null;
  const shownTypes = pickingDefault ? (mostType ? [mostType] : []) : included;
  const nameOf = (key: string) => {
    const found = ctx.aux?.types.find(item => item.key === key);
    return contentTitle(found?.name ?? null, key, 'Equipment type name unavailable');
  };
  const equipmentLabel = !shownTypes.length
    ? (count === 'ic' ? 'All equipment' : 'Equipment')
    : shownTypes.length === 1
      ? nameOf(shownTypes[0]!)
      : `${formatScalar(shownTypes.length, { decimals: 0 })} types`;
  let noDataMessage: string | null = null;
  if (ctx.aux) {
    if (!ctx.aux.types.length) noDataMessage = 'Production data is not in this recording.';
    else if (pickingDefault && !mostType) noDataMessage = 'No production data in this recording.';
  }
  return { count, measure, refits, shownTypes, equipmentLabel, noDataMessage };
}

function equipmentProducedRaceSources(ctx: FamilyPlanContext<ProducedTypesReply>): SourcePlan {
  const { count, measure, refits, shownTypes, noDataMessage } = resolvedEquipment(ctx);
  const top = ctx.countries.top;
  const all = ctx.countries.all;
  const group = top == null && !all ? groupSelection(ctx.widget.countries) : null;
  const waitingForDefault = count === 'units' && !shownTypes.length && !noDataMessage;
  const emptySelection = top == null && !all && !group && !ctx.countries.tags.length;
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : all || group
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const fromHour = measure === 'total' && ctx.widget.time.mode === 'range' && ctx.staticSpan ? ctx.staticSpan[0] : undefined;
  const base: ProducedRaceExactBase = {
    branchId: ctx.branch.id,
    top,
    all,
    group,
    tags: group ? [] : ctx.countries.tags,
    count,
    measure,
    types: shownTypes,
    refits,
    ...(fromHour !== undefined ? { fromHour } : {}),
  };
  const skip = waitingForDefault || noDataMessage !== null || emptySelection;
  return {
    owner: `equipment-produced:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${count}:${measure}:${refits ? 1 : 0}:${shownTypes.join(',')}:${fromHour ?? ''}`,
    entries: skip ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'equipment-produced-race',
        exactProduct: 'equipment-produced-race',
        followQuery: null,
        params: base as unknown as Readonly<Record<string, unknown>>,
        buildExactParams,
        dynamicKeys: ['hour'],
        provenInterval,
        validate: validateEquipmentProducedRace,
      },
    }],
    allowEmptyFollow: skip,
  };
}

export const equipmentProducedRaceDeclaration: BarsWidgetDeclaration<EquipmentProducedRaceOptions, ProducedTypesReply> = {
  family: 'bars',
  kind: 'equipment-produced',
  label: 'Equipment produced',
  subject: 'equipment produced',
  emptyCampaign: 'Load a campaign to see the equipment it produced.',
  icon: 'layers',
  countries: {
    mode: 'multi',
    allowSlots: true,
    everyOnTop: true,
  },
  aux: {
    product: 'equipment-produced-types',
    label: 'the equipment types',
    params: (track, options) => ({ branchId: track.branch.id, refits: options.refits === true }),
    validate: validateEquipmentProducedTypes,
  },
  sources: equipmentProducedRaceSources,
  projection: equipmentProducedRaceProjection,
  projectionOptions: ctx => {
    const { count, measure, equipmentLabel, noDataMessage } = resolvedEquipment(ctx);
    const top = ctx.countries.top;
    const all = ctx.countries.all;
    return {
      count,
      measure,
      equipmentLabel,
      measureLabel: MEASURE_LABELS[measure],
      noDataMessage,
      group: top == null && !all && groupSelection(ctx.widget.countries) !== null,
      top,
    };
  },
  unavailable: ctx => ctx.countries.reason,
  warnings: (ctx, followRefused) => [
    ...(ctx.aux?.warnings ?? []),
    ...(followRefused ? [followRefused] : []),
  ],
  jumpToDate: true,
  keyByBranch: true,
};
