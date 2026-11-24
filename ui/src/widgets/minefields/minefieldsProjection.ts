import { liveKey } from '../../charts/live.ts';
import { DEFAULT_CARD_SURFACE, reservedGrey } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsModel, BarsProjection, BarsSegment } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import { buildShareRowsModel, type ShareRowEntry } from '../../families/bars/shareRows.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { regionLabel } from '../../families/heat/scale.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import type { MinedRegion } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { minesText, validateMinefields, type RegionRow } from './minefieldsData.ts';

const FALLBACK_SURFACE = DEFAULT_CARD_SURFACE;

export type MinefieldsOrderOption = 'mines' | 'name';

export interface MinefieldsProjectionOptions {
  readonly order: MinefieldsOrderOption;
}

function readBigInt(live: ReturnType<BoundedReadView['live']>, key: string): bigint | null {
  if (!live) return null;
  const raw = live.raw?.(key);
  if (raw !== undefined) return raw;
  const value = live.get(key);
  if (typeof value === 'bigint') return value;
  if (typeof value === 'number') return BigInt(Math.trunc(value));
  return null;
}

function minesCountText(total: bigint, scale: number): string {
  return `${minesText(total, scale)} ${total === BigInt(scale) ? 'mine' : 'mines'}`;
}

function projectMineRegion(
  region: MinedRegion,
  names: ReadonlyMap<string, string>,
  scale: number,
  readView: BoundedReadView,
): ShareRowEntry {
  const id = String(region.region);
  const named = regionLabel(readView, region.region, names.get(id) ?? null);
  const name = named.label;
  const labelMissing = named.nameMissing !== undefined;
  const live = readView.live('main');
  const meta: RegionRow = {
    region: id,
    name,
    owners: region.owners.map(owner => ({ tag: owner.tag })),
    unknownOwners: region.unknownOwners,
  };

  if (region.owners.length === 0) {
    if (region.unknownOwners > 0) {
      const reason = `${region.unknownOwners} ${region.unknownOwners === 1 ? 'country' : 'countries'} had unrecorded minefields here.`;
      return {
        key: id,
        subject: id,
        label: name,
        labelMissing,
        segments: [],
        countText: 'Not recorded',
        missing: notRecorded(reason),
        tooltip: {
          title: name,
          hour: readView.hour,
          missingReason: reason,
        },
        meta,
      };
    }
    return {
      key: id,
      subject: id,
      label: name,
      labelMissing,
      segments: [],
      countText: '0 mines',
      tooltip: {
        title: name,
        hour: readView.hour,
        items: [{ label: 'Mines', value: '0' }],
      },
      meta,
    };
  }

  const totalKey = liveKey(id, 'total');
  const total = readBigInt(live, totalKey);
  if (total === null) {
    const reason = live?.missing(totalKey) ?? 'Not recorded at this hour';
    const items: TooltipListItem[] = region.owners.map(owner => {
      const identity = readView.who('country', owner.tag);
      const ownerName = identity.name.status === 'ready' ? identity.name.value : owner.tag;
      const ownerKey = liveKey(id, owner.tag);
      const amount = readBigInt(live, ownerKey);
      return amount === null
        ? { label: ownerName, missingReason: live?.missing(ownerKey) ?? 'Not recorded at this hour' }
        : { label: ownerName, value: minesCountText(amount, scale) };
    });
    if (region.unknownOwners > 0) {
      items.push({ label: 'Not recorded', value: `${region.unknownOwners} ${region.unknownOwners === 1 ? 'country' : 'countries'}` });
    }
    return {
      key: id,
      subject: id,
      label: name,
      labelMissing,
      segments: [],
      countText: 'Not recorded',
      missing: notRecorded(reason),
      tooltip: {
        title: name,
        hour: readView.hour,
        missingReason: reason,
        items,
      },
      meta,
    };
  }

  const segments: BarsSegment[] = [];
  const items: TooltipListItem[] = [{ label: 'Mines', value: minesCountText(total, scale) }];
  const unrecordedColour = reservedGrey(FALLBACK_SURFACE);
  for (const owner of region.owners) {
    const identity = readView.who('country', owner.tag);
    const ownerName = identity.name.status === 'ready' ? identity.name.value : owner.tag;
    const ownerColor = identity.colour?.status === 'ready' ? identity.colour.value : unrecordedColour;
    const ownerKey = liveKey(id, owner.tag);
    const amount = readBigInt(live, ownerKey);
    if (amount === null) {
      items.push({ label: ownerName, swatch: ownerColor, missingReason: live?.missing(ownerKey) ?? 'Not recorded at this hour' });
      continue;
    }
    const share = total > 0n && amount > 0n ? Number(amount * 10000n / total) / 10000 : 0;
    const pct = Math.round(share * 100);
    if (amount > 0n) {
      segments.push({
        key: owner.tag,
        label: ownerName,
        share,
        value: knownValue(amount),
        color: ownerColor,
        inlineLabel: `${ownerName} ${pct}%`,
      });
    } else {
      segments.push({
        key: owner.tag,
        label: ownerName,
        share: 0,
        value: zeroValue(),
        color: ownerColor,
      });
    }
    items.push({ label: ownerName, swatch: ownerColor, value: `${minesCountText(amount, scale)} (${pct}%)` });
  }

  if (region.unknownOwners > 0) {
    items.push({ label: 'Not recorded', value: `${region.unknownOwners} ${region.unknownOwners === 1 ? 'country' : 'countries'}` });
  }

  return {
    key: id,
    subject: id,
    label: name,
    labelMissing,
    segments,
    countText: minesCountText(total, scale),
    tooltip: {
      title: name,
      hour: readView.hour,
      items,
    },
    meta,
  };
}

export const minefieldsProjection: BarsProjection<MinefieldsProjectionOptions> = registerBarsProjection({
  kind: 'minefields',
  family: 'bars',
  project(readView: BoundedReadView): BarsModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused) {
      return buildShareRowsModel({ hour, rows: [], empty: refused, emptyKind: 'unavailable' });
    }
    const panel = readView.panel('main');
    if (panel === undefined) {
      return buildShareRowsModel({ hour, rows: [] });
    }
    const reply = validateMinefields(panel);
    const names = new Map(reply.choices.map(choice => [choice.id, choice.name]));
    const rows = reply.regions.map(region => projectMineRegion(region, names, reply.scale, readView));
    const empty = !rows.length
      ? (reply.unknown.length > 0 ? "Some countries' minefields were not recorded at this date." : 'No minefields at this date.')
      : undefined;
    const emptyKind = reply.unknown.length > 0 ? 'unavailable' : 'empty';
    return buildShareRowsModel({
      hour,
      rows,
      ...(empty !== undefined ? { empty, emptyKind } : {}),
      warnings: reply.warnings,
    });
  },
});

function minefieldsSources(ctx: FamilyPlanContext): SourcePlan {
  const tags = selectedCountryKeys(ctx.widget.countries, ctx.track);
  const chosen = contentSet(ctx.options.regions).map(contentKey).map(Number).filter(Number.isInteger);
  const order: MinefieldsOrderOption = ctx.options.order === 'name' ? 'name' : 'mines';
  const params = {
    branchId: ctx.branch.id,
    ...(tags.length ? { countries: tags.map(countryFromKey) } : {}),
    ...(chosen.length ? { regions: chosen } : {}),
    order,
  };
  const owner = `minefields:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${order}:${JSON.stringify(tags)}:${JSON.stringify(chosen)}`;
  return {
    owner,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'minefields',
        followQuery: { panel: 'minefields', ...params },
        params,
        validate: validateMinefields,
      },
    }],
  };
}

export const minefieldsDeclaration: BarsWidgetDeclaration<MinefieldsProjectionOptions> = {
  family: 'bars',
  kind: 'minefields',
  label: 'Minefields by sea region',
  subject: 'minefields',
  emptyCampaign: 'Load a campaign to see minefields.',
  icon: 'anchor',
  optionFields: OPTIONS,
  countries: { mode: 'none' },
  sources: minefieldsSources,
  projection: minefieldsProjection,
  projectionOptions: ctx => ({
    order: ctx.options.order === 'name' ? 'name' : 'mines',
  }),
  jumpToDate: true,
  keyByBranch: true,
};
