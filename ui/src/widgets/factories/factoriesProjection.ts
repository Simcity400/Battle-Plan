import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { seriesStateProvenInterval, validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { contentSet } from '../../dashboard/options.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { countryRowPresentation, formatBarsValueField, type BarsDataRow, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourceEntry, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, inHourRanges, nextHoldersChange, nextMembershipChange, nextRangesChange, type HolderRun, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, notRecorded, valueFromRawBigInt, zeroValue, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { FactoryGroupsReply, SeriesState, SeriesStateReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { COUNT_MODES, FACTORY_GROUPS, chosenGroups, countMode, damagedMeasure, factoryMeasure, groupName, validateFactoryGroups, type CountMode } from './factoriesData.ts';

const PART_SURFACE = DEFAULT_CARD_SURFACE;

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
  readonly damagedSource?: string;
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

const DAMAGED_SOURCE = 'damaged';

function partSpecs(mode: CountMode, types: readonly string[], groups: FactoryGroupsReply | undefined): FactoryPartSpec[] {
  return chosenGroups(types).map(group => ({
    id: group.value,
    label: groupName(group.value, groups),
    source: partSourceKey(group.value),
    ...(mode === 'built' ? { damagedSource: damagedSourceKey(group.value) } : {}),
    color: colour(PART_SURFACE, FACTORY_GROUPS.indexOf(group)),
  }));
}

function activeTags(cfg: FactoriesExactBase, hour: number): readonly string[] {
  return cfg.membership
    ? cfg.tags.filter(tag => inHourRanges(cfg.membership![tag], hour))
    : cfg.tags;
}

function buildFactoriesExactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as FactoriesExactBase;
  const family = { kind: 'country-factories', measure: cfg.measure };
  if (cfg.top != null) return { branchId: cfg.branchId, family, hour, top: cfg.top };
  const tags = activeTags(cfg, hour);
  if (!tags.length) return null;
  return { branchId: cfg.branchId, family, reads: tags.map(tag => ({ country: countryFromKey(tag), hour })), top: tags.length };
}

function buildPartParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as FactoriesExactBase;
  const family = { kind: 'country-factories', measure: cfg.measure };
  const tags = activeTags(cfg, hour);
  if (!tags.length) return null;
  return { branchId: cfg.branchId, family, reads: tags.map(tag => ({ country: countryFromKey(tag), hour })) };
}

const PART_NOT_READ = 'Not recorded at this hour';

type PanelIndex = (source: string) => ReadonlyMap<string, SeriesState> | null;

function panelIndex(readView: BoundedReadView): PanelIndex {
  const held = new Map<string, ReadonlyMap<string, SeriesState> | null>();
  return source => {
    let index = held.get(source);
    if (index === undefined) {
      const panel = readView.panel(source);
      index = panel === undefined ? null : new Map(validateRankReply(panel).states.map(state => [countryKey(state.country), state]));
      held.set(source, index);
    }
    return index;
  };
}

function partValue(index: PanelIndex, source: string, key: string): Value<bigint | number> | null {
  const states = index(source);
  if (states === null) return null;
  const state = states.get(key);
  if (!state) return notRecorded(PART_NOT_READ);
  return valueFromRawBigInt(state.run?.valueRaw ?? null, { kind: 'notRecorded', reason: state.missing ?? PART_NOT_READ });
}

function reasonOf(value: Value<bigint | number>): string {
  return value.kind === 'notRecorded' || value.kind === 'notApplicable' ? value.reason : PART_NOT_READ;
}

function numericOf(value: Value<bigint | number>): number | null {
  if (value.kind === 'known' || value.kind === 'partial') return Number(value.value);
  if (value.kind === 'zero') return 0;
  return null;
}

function scalarOf(raw: bigint | number): Value<bigint | number> {
  return raw === 0 || raw === 0n ? zeroValue() : knownValue(raw);
}

function countText(amount: number): string {
  return formatScalar(amount, { decimals: 0 });
}

function segment(key: string, label: string, amount: number, total: number, color: string, hatched = false): BarsSegment {
  return { key, label, share: amount / total, value: scalarOf(amount), format: { decimals: 0 }, color, ...(hatched ? { hatched } : {}) };
}

function buildPartSegments(
  index: PanelIndex,
  wireName: string,
  total: number,
  options: FactoriesProjectionOptions,
  baseColor: string | undefined,
): { readonly segments?: readonly BarsSegment[]; readonly tooltipItems: TooltipListItem[] } {
  if (!options.split && options.mode !== 'built') return { tooltipItems: [] };
  if (total <= 0) return { tooltipItems: [] };

  const tooltipItems: TooltipListItem[] = [];
  if (!options.split) {
    const damaged = options.damagedSource ? partValue(index, options.damagedSource, wireName) : null;
    if (damaged === null) return { tooltipItems };
    const amount = numericOf(damaged);
    if (amount === null) {
      tooltipItems.push({ label: 'Damaged', missingReason: reasonOf(damaged) });
      return { tooltipItems };
    }
    const hurt = Math.min(total, Math.max(0, amount));
    tooltipItems.push({ label: 'Damaged', value: countText(hurt) });
    if (hurt <= 0) return { tooltipItems };
    const fill = baseColor ?? colour(PART_SURFACE, 0);
    const segments: BarsSegment[] = [];
    if (total > hurt) segments.push(segment('healthy', 'Undamaged', total - hurt, total, fill));
    segments.push(segment('damaged', 'Damaged', hurt, total, fill, true));
    return { segments, tooltipItems };
  }

  const segments: BarsSegment[] = [];
  let known = 0;
  let unknownReason: string | null = null;
  for (const spec of options.parts) {
    const value = partValue(index, spec.source, wireName);
    if (value === null) return { tooltipItems };
    const amount = numericOf(value);
    if (amount === null) {
      unknownReason ??= reasonOf(value);
      tooltipItems.push({ label: spec.label, swatch: spec.color, missingReason: reasonOf(value) });
      continue;
    }
    const whole = Math.max(0, amount);
    known += whole;
    tooltipItems.push({ label: spec.label, swatch: spec.color, value: countText(whole) });
    if (whole <= 0) continue;
    const damagedValue = spec.damagedSource ? partValue(index, spec.damagedSource, wireName) : null;
    const damaged = damagedValue === null ? null : numericOf(damagedValue);
    if (damagedValue !== null && damaged === null) {
      tooltipItems.push({ label: `Damaged ${spec.label}`, swatch: spec.color, missingReason: reasonOf(damagedValue) });
    }
    const hurt = damaged === null ? 0 : Math.min(whole, Math.max(0, damaged));
    if (whole > hurt) segments.push(segment(spec.id, spec.label, whole - hurt, total, spec.color));
    if (hurt > 0) segments.push(segment(`damaged:${spec.id}`, `Damaged ${spec.label}`, hurt, total, spec.color, true));
  }
  if (unknownReason !== null && total > known) {
    segments.push({ key: 'unrecorded', label: 'Not recorded', share: (total - known) / total, value: notRecorded(unknownReason), color: PART_SURFACE, hatched: true });
  }
  return { ...(segments.length ? { segments } : {}), tooltipItems };
}

function makeRow(
  readView: BoundedReadView,
  index: PanelIndex,
  key: string,
  displayTag: string,
  wireName: string,
  value: Value<bigint | number>,
  options: FactoriesProjectionOptions,
): BarsDataRow {
  const pres = countryRowPresentation(readView, displayTag);
  const numeric = numericOf(value);
  const parts = numeric !== null ? buildPartSegments(index, wireName, numeric, options, pres.color) : { tooltipItems: [] };
  const totalItem: TooltipListItem = value.kind === 'notRecorded'
    ? { label: options.modeLabel, missingReason: value.reason }
    : value.kind === 'notApplicable'
      ? { label: options.modeLabel, value: '\u2014' }
      : { label: options.modeLabel, value: formatBarsValueField({ value, format: { decimals: 0 } }).text };
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

    if (options.top == null && options.membership && !Object.values(options.membership).some(ranges => inHourRanges(ranges, hour))) {
      return { ...baseModel, rows: [], empty: 'No country of this group at this date.', emptyKind: 'empty' };
    }

    const panel = readView.panel('main');
    if (panel !== undefined) {
      const reply: SeriesStateReply = validateRankReply(panel);
      const index = panelIndex(readView);
      const rows: BarsDataRow[] = [];
      for (const state of reply.states) {
        const key = countryKey(state.country);
        const displayTag = options.holders ? (holderTagAt(options.holders[key], hour) ?? key) : key;
        if (options.top == null && options.membership) {
          const inMember = inHourRanges(options.membership[key], hour) || inHourRanges(options.membership[displayTag], hour);
          if (!inMember) continue;
        }
        const value = valueFromRawBigInt(
          state.run?.valueRaw ?? null,
          { kind: 'notRecorded', reason: state.missing ?? 'Not recorded at this hour' },
        );
        rows.push(makeRow(readView, index, key, displayTag, key, value, options));
      }
      const ordered = [...rows.filter(row => row.numeric != null), ...rows.filter(row => row.numeric == null)];
      return {
        ...baseModel,
        rows: ordered,
        ...(!ordered.length ? { empty: 'No factories recorded at this date.', emptyKind: 'empty' as const } : {}),
      };
    }
    const refused = readView.refused('main');
    if (refused) return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
    return { ...baseModel, rows: [] };
  },
});

function factoriesSources(ctx: FamilyPlanContext<FactoryGroupsReply>): SourcePlan {
  const mode = countMode(ctx.options.count);
  const types = contentSet(ctx.options.types);
  const measureName = factoryMeasure(mode, types);
  const top = ctx.countries.top;
  const split = ctx.options.split === true;
  const owner = `factories:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${measureName}:${split ? 1 : 0}`;
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : { mode: 'keyed', keys: ctx.countries.tags };
  const tags = top != null ? ctx.countries.every : ctx.countries.tags;
  const membership = top != null ? null : ctx.countries.membership;
  const read = (name: string, base: FactoriesExactBase, build: typeof buildFactoriesExactParams): SourceEntry => ({
    name,
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
      params: base as unknown as Readonly<Record<string, unknown>>,
      buildExactParams: build,
      dynamicKeys: ['hour', 'reads'],
      ...(base.top == null ? { provenInterval: seriesStateProvenInterval } : {}),
      validate: validateRankReply,
    },
  });
  const main = read('main', { branchId: ctx.branch.id, measure: measureName, top, tags, membership }, buildFactoriesExactParams);
  const entries: SourceEntry[] = [main];
  const part = (name: string, measure: string) => read(name, { branchId: ctx.branch.id, measure, top: null, tags, membership }, buildPartParams);
  if (split) {
    for (const group of chosenGroups(types)) {
      entries.push(part(partSourceKey(group.value), `${mode}${group.bit}`));
      if (mode === 'built') entries.push(part(damagedSourceKey(group.value), `damaged${group.bit}`));
    }
  } else if (mode === 'built') {
    entries.push(part(DAMAGED_SOURCE, damagedMeasure(types)));
  }
  return { owner, entries };
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
      ...(mode === 'built' ? { damagedSource: DAMAGED_SOURCE } : {}),
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
