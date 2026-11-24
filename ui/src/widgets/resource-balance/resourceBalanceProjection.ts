import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { defaultCountryKey, selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import { unavailableArt } from '../../families/art/types.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import type { BarsDataRow, BarsModel, BarsProjection, BarsRowMedia } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, type Value } from '../../families/value.ts';
import type { ValueFormat } from '../../families/valueFormat.ts';
import type { ResourceInfo, ResourceNet } from '../../generated/dashboard';
import { RESOURCE_BALANCE, resourceName, validateResourceBalance } from './resourceReply.ts';

export interface ResourceBalanceOptions {
  readonly country: string | null;
}

const NO_ICON = 'The current installation has no icon for this resource';

function media(readView: BoundedReadView, info: ResourceInfo): BarsRowMedia {
  if (info.icon === null) return { kind: 'art', art: unavailableArt(NO_ICON) };
  return { kind: 'art', art: readView.art({ kind: 'url', url: info.icon }) };
}

function figure(raw: string | null, reason: string): Value<bigint> {
  return raw === null ? notRecorded(reason) : knownValue(BigInt(raw));
}

function row(readView: BoundedReadView, info: ResourceInfo, ordinal: number, net: ResourceNet, decimals: number): BarsDataRow {
  const reason = net.missing ?? 'Not recorded at this hour';
  const amount: ValueFormat = { decimals, maxDecimals: 2 };
  const value = figure(net.net, reason);
  const numeric = net.net === null ? null : Number(BigInt(net.net)) / 10 ** decimals;
  return {
    kind: 'data',
    key: String(net.resource),
    subject: String(net.resource),
    label: resourceName(info),
    ...(info.name === null ? { labelMissing: true } : {}),
    media: media(readView, info),
    color: colour(DEFAULT_CARD_SURFACE, ordinal),
    value: {
      value,
      format: { ...amount, signed: true },
      label: 'Net a day',
      ...(numeric !== null && numeric < 0 ? { tone: 'shortage' as const } : {}),
    },
    numeric,
    pair: { topLabel: 'In', topValue: figure(net.supply, reason), bottomLabel: 'Out', bottomValue: figure(net.demand, reason), format: amount },
  };
}

function summary(short: number, unrecorded: number): string {
  const counted = short > 0 ? `${short} short` : unrecorded > 0 ? null : 'No shortages';
  const missing = unrecorded > 0 ? `${unrecorded} not recorded` : null;
  return [counted, missing].filter(Boolean).join(' · ');
}

function project(readView: BoundedReadView): BarsModel {
  const base = { form: 'net' as const, hour: readView.hour };
  const refused = readView.refused('main');
  if (refused) return { ...base, rows: [], empty: refused, emptyKind: 'unavailable' };
  const panel = readView.panel('main');
  if (panel === undefined) return { ...base, rows: [] };
  const reply = validateResourceBalance(panel);
  if (reply.rows.length === 0 && reply.poolsMissing !== null) {
    return { ...base, rows: [], empty: reply.poolsMissing, emptyKind: 'unavailable', warnings: reply.warnings };
  }
  const ordinals = new Map(reply.resources.map((info, index) => [info.id, [info, index] as const]));
  const rows = reply.rows.flatMap(net => {
    const held = ordinals.get(net.resource);
    return held ? [row(readView, held[0], held[1], net, reply.decimals)] : [];
  });
  return {
    ...base,
    rows,
    summaryText: summary(reply.short, reply.unrecorded),
    ...(rows.length ? {} : { empty: 'This recording names no resources.', emptyKind: 'empty' as const }),
    warnings: reply.warnings,
  };
}

export const resourceBalanceProjection: BarsProjection<ResourceBalanceOptions> = registerFamilyProjection({
  kind: 'resource-balance', family: 'bars', nextChange: () => Infinity, project,
}, 'bars');

const countryOf = (ctx: FamilyPlanContext) => selectedCountryKeys(ctx.widget.countries, ctx.track)[0] ?? defaultCountryKey(ctx.track) ?? null;

function sources(ctx: FamilyPlanContext): SourcePlan {
  const country = countryOf(ctx);
  const hideEmpty = ctx.options.hideEmpty === true;
  const owner = `resource-balance:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${JSON.stringify([country, hideEmpty])}`;
  if (!country) return { owner, entries: [], unavailable: 'Choose a country in the widget’s header.' };
  const params = { branchId: ctx.branch.id, country: countryFromKey(country), ...(hideEmpty ? { hideEmpty } : {}) };
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
        product: RESOURCE_BALANCE,
        followQuery: { panel: RESOURCE_BALANCE, ...params },
        params,
        validate: validateResourceBalance,
      },
    }],
  };
}

export const resourceBalanceDeclaration: BarsWidgetDeclaration<ResourceBalanceOptions> = {
  family: 'bars',
  kind: 'resource-balance',
  label: 'Resource balance',
  subject: 'resources',
  emptyCampaign: 'Load a campaign to see resources.',
  icon: 'layers',
  countries: { mode: 'none' },
  sources,
  projection: resourceBalanceProjection,
  projectionOptions: ctx => ({ country: countryOf(ctx) }),
  jumpToDate: true,
  keyByBranch: true,
};
