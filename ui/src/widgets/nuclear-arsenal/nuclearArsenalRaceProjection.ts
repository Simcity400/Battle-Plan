import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, inHourRanges, nextHoldersChange, nextMembershipChange, nextRangesChange, type HolderRun, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, zeroValue } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { NukeCountry, NukeType } from '../../generated/dashboard';
import { typeName, validateNuclearArsenal } from './nuclearArsenalData.ts';

const PART_SURFACE = DEFAULT_CARD_SURFACE;

export interface NuclearArsenalRaceOptions {
  readonly all: boolean;
  readonly chosenTags: readonly string[];
  readonly holders: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved: HourRanges;
  readonly top: number | null;
  readonly split: boolean;
}

function typeInfoMap(types: readonly NukeType[]): Map<number, { readonly label: string; readonly color: string }> {
  const map = new Map<number, { readonly label: string; readonly color: string }>();
  types.forEach((type, index) => {
    map.set(type.ordinal, { label: typeName(type), color: colour(PART_SURFACE, index) });
  });
  return map;
}

function makeRow(
  readView: BoundedReadView,
  row: NukeCountry,
  types: ReadonlyMap<number, { readonly label: string; readonly color: string }>,
  split: boolean,
): BarsDataRow {
  const pres = countryRowPresentation(readView, row.tag);
  const value = row.total === 0 ? zeroValue() : knownValue(row.total);
  const tooltipItems: TooltipListItem[] = [
    { label: 'Nuclear stockpile', value: formatScalar(row.total, { decimals: 0, maxDecimals: 0 }) },
  ];
  let segments: BarsSegment[] | undefined;
  if (split && row.total > 0 && row.counts.length > 0) {
    const positive = row.counts.filter(count => count.value > 0);
    const sum = positive.reduce((acc, count) => acc + count.value, 0);
    if (sum > 0) {
      segments = positive.map((count, idx) => {
        const info = types.get(count.ordinal) ?? { label: `Unnamed nuclear type ${count.ordinal}`, color: colour(PART_SURFACE, types.size + idx) };
        const text = count.ready > 0
          ? `${formatScalar(count.value, 'integer')} (${formatScalar(count.ready, 'integer')} ready)`
          : formatScalar(count.value, 'integer');
        tooltipItems.push({ label: info.label, swatch: info.color, value: text });
        return {
          key: String(count.ordinal),
          label: info.label,
          share: count.value / sum,
          value: knownValue(count.value),
          format: { decimals: 0 },
          color: info.color,
        };
      });
    }
  } else if (row.counts.length > 0) {
    row.counts.forEach((count, idx) => {
      if (count.value <= 0) return;
      const info = types.get(count.ordinal) ?? { label: `Unnamed nuclear type ${count.ordinal}`, color: colour(PART_SURFACE, types.size + idx) };
      tooltipItems.push({ label: info.label, value: formatScalar(count.value, 'integer') });
    });
  }
  return {
    kind: 'data',
    key: row.tag,
    subject: row.tag,
    label: pres.label,
    labelMissing: pres.labelMissing,
    media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    value: { value, format: { decimals: 0 }, label: 'Nuclear stockpile' },
    numeric: row.total,
    ...(segments ? { segments } : {}),
    tooltip: {
      title: pres.labelMissing ? 'Name not recorded' : pres.label,
      hour: readView.hour,
      items: tooltipItems,
    },
  };
}

function compareExplicitRows(a: BarsDataRow, b: BarsDataRow): number {
  const aMissing = a.numeric === null || a.numeric === undefined;
  const bMissing = b.numeric === null || b.numeric === undefined;
  if (aMissing !== bMissing) return aMissing ? 1 : -1;
  if (!aMissing && !bMissing && a.numeric !== b.numeric) return b.numeric! - a.numeric!;
  return compareTags(a.subject ?? a.key, b.subject ?? b.key);
}

export const nuclearArsenalRaceProjection: BarsProjection<NuclearArsenalRaceOptions> = registerFamilyProjection({
  kind: 'nuclear-arsenal',
  family: 'bars',
  nextChange(readView, options) {
    const hour = readView.hour;
    return Math.min(
      nextHoldersChange(options.holders, hour),
      nextRangesChange(options.unobserved, hour),
      nextMembershipChange(options.membership, hour),
    );
  },
  project(readView: BoundedReadView, options: NuclearArsenalRaceOptions): BarsModel {
    const hour = readView.hour;
    const baseModel = {
      form: 'ordered' as const,
      hour,
      animateOrder: true,
      showAxis: true,
      scaleUnit: 'Nuclear bombs',
      scaleFormat: { decimals: 0 },
    };

    if (options.top == null && options.unobserved.some(([first, last]) => first <= hour && hour <= last)) {
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

    const reply = validateNuclearArsenal(panel);
    if (reply.missing !== null) {
      return { ...baseModel, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }

    const types = typeInfoMap(reply.types);
    let selected: NukeCountry[];
    if (options.top != null || options.all) {
      selected = reply.countries;
    } else if (options.membership) {
      selected = reply.countries.filter(row => inHourRanges(options.membership![row.tag], hour));
    } else if (options.holders) {
      const activeTags = new Set<string>();
      for (const runs of Object.values(options.holders)) {
        const tag = holderTagAt(runs, hour);
        if (tag !== null) activeTags.add(tag);
      }
      selected = reply.countries.filter(row => activeTags.has(row.tag));
    } else {
      const chosen = new Set(options.chosenTags);
      selected = reply.countries.filter(row => chosen.has(row.tag));
    }

    const rows = selected.map(row => makeRow(readView, row, types, options.split));
    if (options.top == null && !options.all) rows.sort(compareExplicitRows);

    return {
      ...baseModel,
      rows,
      warnings: reply.warnings,
      ...(!rows.length ? { empty: 'No nuclear stockpile recorded at this date.', emptyKind: 'empty' as const } : {}),
    };
  },
}, 'race');

function nuclearArsenalRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const top = ctx.countries.top;
  const asked = top != null ? { branchId: ctx.branch.id, top } : { branchId: ctx.branch.id };
  return {
    owner: `nuclear-arsenal:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${top ?? 'all'}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: top != null ? { mode: 'population', form: 'top', limit: top } : { mode: 'population', form: 'all' },
        product: 'nuclear-arsenal',
        followQuery: { panel: 'nuclear-arsenal', ...asked },
        params: asked,
        validate: validateNuclearArsenal,
      },
    }],
  };
}

export const nuclearArsenalRaceDeclaration: BarsWidgetDeclaration<NuclearArsenalRaceOptions> = {
  family: 'bars',
  kind: 'nuclear-arsenal',
  label: 'Nuclear arsenal',
  subject: 'nuclear stockpiles',
  emptyCampaign: 'Load a campaign to see nuclear stockpiles.',
  icon: 'target',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    allTagsForLabels: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: nuclearArsenalRaceSources,
  projection: nuclearArsenalRaceProjection,
  projectionOptions: ctx => ({
    all: ctx.countries.all,
    chosenTags: ctx.countries.tags,
    holders: ctx.countries.top == null ? ctx.countries.holders : null,
    membership: ctx.countries.top == null ? ctx.countries.membership : null,
    unobserved: ctx.countries.top == null ? ctx.countries.unobserved : [],
    top: ctx.countries.top,
    split: ctx.options.split !== false,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
