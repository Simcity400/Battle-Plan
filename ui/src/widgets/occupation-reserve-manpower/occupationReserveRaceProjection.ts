import { dayLabel } from '../../charts/draw.ts';
import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { forceAtSources, GROUP_EMPTY } from '../../families/bars/forceAtRace.ts';
import { seriesRaceSelection, seriesRaceUnavailable, seriesRaceWarnings, type SeriesRaceBaseOptions } from '../../families/bars/seriesRace.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, zeroValue } from '../../families/value.ts';
import type { Country, OccupationReserveReply, ReserveHolder } from '../../generated/dashboard';
import { persons, validateOccupationReserve } from './occupationReserveData.ts';

const FALLBACK_SOURCE_COLOUR = colour(DEFAULT_CARD_SURFACE, 0);

export type OccupationReserveRaceOptions = SeriesRaceBaseOptions;

export interface OccupationReserveRowMeta {
  readonly tag: string;
  readonly id: number;
  readonly key: string;
}

function sourcePresentation(readView: BoundedReadView, source: Country): { readonly key: string; readonly label: string; readonly color: string } {
  if (source.tag === null) {
    return { key: `unresolved:${source.id}`, label: 'Source country unavailable', color: FALLBACK_SOURCE_COLOUR };
  }
  const pres = countryRowPresentation(readView, source.tag);
  return {
    key: source.tag,
    label: pres.labelMissing ? 'Source country unavailable' : pres.label,
    color: pres.color ?? FALLBACK_SOURCE_COLOUR,
  };
}

function makeHolderRow(readView: BoundedReadView, holder: ReserveHolder, readHour: number | null): BarsDataRow | null {
  const tag = holder.country.tag;
  if (tag === null) return null;
  const key = holder.slot === null ? tag : countryKey(holder.slot);
  const identity = readView.who('country', key);
  const shown = countryRowPresentation(readView, key);
  const pres = identity.name.status === 'ready' ? shown : { ...shown, label: tag };
  const total = holder.total;
  const value = total === 0 ? zeroValue() : knownValue(total);

  const bySource = new Map<string, { readonly label: string; readonly color: string; amount: number }>();
  const positiveItems: TooltipListItem[] = [];
  const negativeItems: TooltipListItem[] = [];
  for (const entry of holder.entries) {
    const src = sourcePresentation(readView, entry.source);
    if (entry.amount > 0) {
      const existing = bySource.get(src.key);
      if (existing) existing.amount += entry.amount;
      else bySource.set(src.key, { label: src.label, color: src.color, amount: entry.amount });
      positiveItems.push({ label: src.label, swatch: src.color, value: persons(entry.amount) });
    } else if (entry.amount === 0) {
      positiveItems.push({ label: src.label, swatch: src.color, value: '0' });
    } else {
      negativeItems.push({ label: src.label, value: persons(entry.amount) });
    }
  }

  let segments: BarsSegment[] | undefined;
  if (total > 0 && bySource.size > 0) {
    const ordered = [...bySource.entries()].sort((a, b) => b[1].amount - a[1].amount || a[0].localeCompare(b[0]));
    segments = ordered.map(([srcKey, item]) => ({
      key: srcKey,
      label: item.label,
      share: item.amount / total,
      value: knownValue(item.amount),
      format: { decimals: 0 },
      color: item.color,
    }));
  }

  const tooltipItems: TooltipListItem[] = [
    { label: 'Reserve manpower held', value: persons(total) },
    ...positiveItems,
    ...(negativeItems.length ? [{ label: 'Negative entries', value: '' }, ...negativeItems] : []),
    ...(readHour !== null ? [{ label: 'As of', value: dayLabel(readHour) }] : []),
  ];

  const meta: OccupationReserveRowMeta = { tag, id: holder.country.id, key };
  return {
    kind: 'data',
    key,
    subject: key,
    label: pres.label,
    labelMissing: pres.labelMissing,
    media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    value: { value, format: { decimals: 0 }, label: 'Reserve manpower' },
    numeric: total,
    ...(segments ? { segments } : {}),
    tooltip: {
      title: pres.labelMissing ? 'Name not recorded' : pres.label,
      hour: readView.hour,
      items: tooltipItems,
    },
    meta,
  };
}

export const occupationReserveRaceProjection: BarsProjection<OccupationReserveRaceOptions> = registerFamilyProjection({
  kind: 'occupation-reserve-manpower',
  family: 'bars',
  nextChange: () => Infinity,
  project(readView: BoundedReadView, options: OccupationReserveRaceOptions): BarsModel {
    const hour = readView.hour;
    const baseModel = {
      form: 'ordered' as const,
      hour,
      animateOrder: true,
      showAxis: true,
      scaleUnit: 'Manpower',
      scaleFormat: { decimals: 0 },
    };

    const refused = readView.refused('main');
    if (refused) {
      return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
    }

    const panel = readView.panel('main');
    if (panel === undefined) {
      return { ...baseModel, rows: [] };
    }
    const reply: OccupationReserveReply = validateOccupationReserve(panel);
    if (reply.missing !== null) {
      return { ...baseModel, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }

    const rows: BarsDataRow[] = [];
    for (const holder of reply.holders) {
      const row = makeHolderRow(readView, holder, reply.readHour);
      if (row) rows.push(row);
    }

    const warnings = [
      ...reply.warnings,
      ...(reply.unnamed > 0 ? [`${reply.unnamed} holders the recording names no tag for are not shown.`] : []),
    ];

    return {
      ...baseModel,
      rows,
      warnings,
      ...(!rows.length ? { empty: options.group ? GROUP_EMPTY : 'No reserve manpower is held at this date.', emptyKind: 'empty' as const } : {}),
    };
  },
}, 'main');

function occupationReserveSources(ctx: FamilyPlanContext): SourcePlan {
  const owner = `occupation-reserve:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`;
  return forceAtSources(ctx, owner, 'occupation-reserve-manpower', {}, validateOccupationReserve);
}

export const occupationReserveRaceDeclaration: BarsWidgetDeclaration<OccupationReserveRaceOptions> = {
  family: 'bars',
  kind: 'occupation-reserve-manpower',
  label: 'Occupation reserve manpower',
  subject: 'occupation manpower',
  emptyCampaign: 'Load a campaign to see occupation manpower.',
  icon: 'flag',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: occupationReserveSources,
  projection: occupationReserveRaceProjection,
  projectionOptions: seriesRaceSelection,
  unavailable: seriesRaceUnavailable,
  warnings: (_ctx, followRefused) => seriesRaceWarnings(followRefused),
  jumpToDate: true,
  keyByBranch: true,
};
