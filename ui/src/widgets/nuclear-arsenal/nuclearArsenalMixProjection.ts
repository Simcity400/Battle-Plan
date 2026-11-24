import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsLegendItem, type BarsModel, type BarsProjection, type BarsSegment } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { buildShareRowsModel, type ShareRowEntry } from '../../families/bars/shareRows.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, inHourRanges, nextHoldersChange, nextMembershipChange, nextRangesChange, type HolderRun, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { NuclearArsenalReply, NukeCountry, NukeType } from '../../generated/dashboard';
import { typeName, validateNuclearArsenal } from './nuclearArsenalData.ts';

const PART_SURFACE = DEFAULT_CARD_SURFACE;

export interface NuclearArsenalMixOptions {
  readonly top: number | null;
  readonly all: boolean;
  readonly chosenTags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
}

function sharePct(share: number): string {
  return `${Math.round(share * 100)}%`;
}

export const nuclearArsenalMixProjection: BarsProjection<NuclearArsenalMixOptions> = registerFamilyProjection({
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
  project(readView: BoundedReadView, options: NuclearArsenalMixOptions): BarsModel {
    const hour = readView.hour;
    const ranked = options.top != null || options.all;
    if (!ranked && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return buildShareRowsModel({ hour, rows: [], empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable' });
    }
    const refused = readView.refused('main');
    if (refused) {
      return buildShareRowsModel({ hour, rows: [], empty: refused, emptyKind: 'unavailable' });
    }
    const panel = readView.panel('main');
    if (panel === undefined) {
      return buildShareRowsModel({ hour, rows: [] });
    }
    const reply: NuclearArsenalReply = validateNuclearArsenal(panel);
    if (reply.missing !== null) {
      return buildShareRowsModel({ hour, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });
    }

    const typeByOrdinal = new Map<number, { readonly type: NukeType; readonly color: string; readonly label: string }>();
    reply.types.forEach((type, index) => {
      typeByOrdinal.set(type.ordinal, { type, color: colour(PART_SURFACE, index), label: typeName(type) });
    });

    let selected: NukeCountry[];
    if (ranked) {
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

    if (!ranked) {
      selected = [...selected].sort((a, b) => b.total - a.total || compareTags(a.tag, b.tag));
    }

    const usedOrdinals = new Set<number>();
    const rows: ShareRowEntry[] = [];
    for (const row of selected) {
      const pres = countryRowPresentation(readView, row.tag);
      const countText = formatScalar(row.total, { decimals: 0 });
      const tooltipItems: TooltipListItem[] = [{ label: 'Nuclear stockpile', value: countText }];
      if (row.total <= 0) {
        rows.push({
          key: row.tag,
          subject: row.tag,
          label: pres.label,
          labelMissing: pres.labelMissing,
          media: pres.media,
          segments: [],
          countText,
          tooltip: { title: pres.labelMissing ? 'Name not recorded' : pres.label, hour, items: tooltipItems },
        });
        continue;
      }

      const positive = row.counts.filter(c => c.value > 0);
      const partsSum = positive.reduce((sum, c) => sum + c.value, 0);
      if (partsSum !== row.total) {
        const reason = 'Weapon type breakdown does not reconcile with the recorded total at this hour';
        tooltipItems.push({ label: 'Weapon types', missingReason: reason });
        rows.push({
          key: row.tag,
          subject: row.tag,
          label: pres.label,
          labelMissing: pres.labelMissing,
          media: pres.media,
          segments: [],
          countText,
          missing: notRecorded(reason),
          tooltip: { title: pres.labelMissing ? 'Name not recorded' : pres.label, hour, missingReason: reason, items: tooltipItems },
        });
        continue;
      }

      const segments: BarsSegment[] = [];
      for (const count of positive) {
        const info = typeByOrdinal.get(count.ordinal);
        const label = info?.label ?? `Unnamed nuclear type ${count.ordinal}`;
        const color = info?.color ?? colour(PART_SURFACE, count.ordinal);
        const share = count.value / row.total;
        usedOrdinals.add(count.ordinal);
        segments.push({
          key: String(count.ordinal),
          label,
          share,
          value: knownValue(count.value),
          format: { decimals: 0 },
          color,
          inlineLabel: `${label} ${sharePct(share)}`,
        });
        tooltipItems.push({
          label,
          swatch: color,
          value: `${formatScalar(count.value, { decimals: 0 })} (${sharePct(share)})`,
        });
      }

      rows.push({
        key: row.tag,
        subject: row.tag,
        label: pres.label,
        labelMissing: pres.labelMissing,
        media: pres.media,
        segments,
        countText,
        tooltip: { title: pres.labelMissing ? 'Name not recorded' : pres.label, hour, items: tooltipItems },
      });
    }

    const legend: BarsLegendItem[] = [];
    for (const type of reply.types) {
      if (!usedOrdinals.has(type.ordinal)) continue;
      const info = typeByOrdinal.get(type.ordinal)!;
      legend.push({ key: String(type.ordinal), label: info.label, color: info.color });
    }

    return buildShareRowsModel({
      hour,
      rows,
      legend,
      warnings: reply.warnings,
      ...(!rows.length ? { empty: 'No nuclear stockpile recorded at this date.', emptyKind: 'empty' as const } : {}),
    });
  },
}, 'mix');

function nuclearArsenalMixSources(ctx: FamilyPlanContext): SourcePlan {
  const top = ctx.countries.top;
  const asked = top != null ? { branchId: ctx.branch.id, top } : { branchId: ctx.branch.id };
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : { mode: 'population', form: 'all' };
  return {
    owner: `nuclear-arsenal:mix:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${top ?? 'all'}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage,
        product: 'nuclear-arsenal',
        followQuery: { panel: 'nuclear-arsenal', ...asked },
        params: asked,
        validate: validateNuclearArsenal,
      },
    }],
    allowEmptyFollow: false,
  };
}

export const nuclearArsenalMixDeclaration: BarsWidgetDeclaration<NuclearArsenalMixOptions> = {
  family: 'bars',
  kind: 'nuclear-arsenal',
  label: 'Nuclear arsenal composition',
  subject: 'nuclear arsenals',
  emptyCampaign: 'Load a campaign to see its nuclear arsenal.',
  icon: 'target',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: nuclearArsenalMixSources,
  projection: nuclearArsenalMixProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      top: ctx.countries.top,
      all: ctx.countries.all,
      chosenTags: ranked ? [] : ctx.countries.tags,
      holders: ranked ? null : ctx.countries.holders,
      membership: ranked ? null : ctx.countries.membership,
      unobserved: ranked ? [] : ctx.countries.unobserved,
    };
  },
  unavailable: ctx => {
    if (!ctx.countries.all && ctx.countries.top == null && ctx.following && !ctx.countries.tags.length && ctx.countries.membership === null) {
      return 'No countries available for this selection.';
    }
    return null;
  },
  warnings: (_ctx, followRefused) => (followRefused ? [`Values are not followed here: ${followRefused}.`] : []),
  jumpToDate: true,
  keyByBranch: true,
};
