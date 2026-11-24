import { liveKey } from '../../charts/live.ts';
import { MARKET, validateMarketReply } from '../../dashboard/market.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artState, resolveIdentity, urlArtHandle, type ResolvedIdentity } from '../../families/table/identityCell.ts';
import {
  type TableCell,
  type TableModel,
  type TableProjection,
  type TableRow,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { knownValue, notRecorded, valueFromLive, zeroValue } from '../../families/value.ts';
import { MARKET_TABS, OPTIONS, marketColumns, marketSettingsKey, marketTableBase } from './definition.ts';

const UNKNOWN_COUNTRY: ResolvedIdentity = { value: notRecorded('Unknown country'), name: 'Unknown country' };
const countryAt = (readView: BoundedReadView, tag: string | null | undefined, hour: number): ResolvedIdentity =>
  tag ? resolveIdentity(readView, 'country', tag, { atHour: hour }) : UNKNOWN_COUNTRY;
import { cic, contractRows, levelText, offerRows, units, type MarketTab } from './marketData.ts';

export interface InternationalMarketProjectionOptions {
  readonly view: MarketTab;
  readonly filterAll: boolean;
  readonly filter: FamilyPlanContext<unknown>['countries']['filter'];
}

function makeMatcher(filterAll: boolean, filter: InternationalMarketProjectionOptions['filter']) {
  return (tag: string, atHour: number): boolean => {
    if (filterAll || !filter.passing) return true;
    return filter.passing[tag]?.some(([first, last]) => first <= atHour && atHour <= last) ?? false;
  };
}


export const internationalMarketProjection: TableProjection<InternationalMarketProjectionOptions> = {
  kind: 'international-market',
  family: 'table',
  project(readView: BoundedReadView, options: InternationalMarketProjectionOptions): TableModel {
    const hour = readView.hour;
    const columns = marketColumns(options.view);
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns, rows: [], hour, refused };
    }
    const rawPanel = readView.panel('main');
    if (rawPanel === undefined) {
      return { columns, rows: [], hour };
    }
    const reply = validateMarketReply(rawPanel);
    const at = { ...reply, hour };
    const section = options.view === 'inventory' ? reply.offers : reply.contracts;
    if (!section) {
      return {
        columns,
        rows: [],
        hour,
        empty: options.view === 'inventory' ? 'Market inventory was not recorded.' : 'Contract terms unavailable.',
        emptyKind: 'unavailable',
        warnings: reply.warnings,

      };
    }

    const matcher = makeMatcher(options.filterAll, options.filter);
    const live = readView.live('main') ?? null;

    if (options.view === 'inventory') {
      const offers = offerRows(at, matcher);
      if (!offers.length) {
        return {
          columns,
          rows: [],
          hour,
          empty: 'Nothing listed.',
          emptyKind: 'empty',
          warnings: reply.warnings,

        };
      }
      const rows: TableRow[] = offers.map(row => {
        const seller = countryAt(readView, row.seller, hour);
        const eqName = row.name ?? 'Equipment name unavailable';
        const art = artState(readView, urlArtHandle(row.icon));
        return {
          key: row.key,
          name: eqName,
          cells: {
            seller: { value: seller.value, iconSlot: true, icon: seller.icon, iconPending: seller.iconPending, country: row.seller ?? undefined, countryColor: seller.color },
            equipment: { value: knownValue(eqName), iconSlot: true, artShape: 'wide', artName: eqName, ...art },
            amount: { value: row.amount === 0n ? zeroValue(0n) : knownValue(row.amount) },
            level: { value: row.level === null ? notRecorded('Price level not recorded') : knownValue(levelText(row.level)) },
          },
          tooltipExtra: [
            { label: 'Seller', value: seller.value, divider: true },
            { label: 'On market', value: knownValue(row.amount), format: 'fixed5Amount' },
            { label: 'Price level', value: row.level === null ? notRecorded('Price level not recorded') : knownValue(levelText(row.level)) },
          ],
          tooltipAction: 'Click for market details',
          meta: { title: `${seller.name}: market`, tags: row.seller ? [row.seller] : [], selected: `offer:${row.offer.country}` },
        };
      });
      return { columns, rows, hour, warnings: reply.warnings };
    }

    const listed = contractRows(at, options.view, matcher);
    const warnings = listed.unattributed
      ? [...reply.warnings, `${listed.unattributed} ${options.view === 'contracts' ? 'contracts' : 'requests'} whose countries were not recorded at this date may also match`]
      : reply.warnings;

    if (!listed.rows.length) {
      return {
        columns,
        rows: [],
        hour,
        empty: listed.unattributed
          ? `Some ${options.view === 'contracts' ? 'contracts' : 'requests'}' countries were not recorded at this date.`
          : (options.view === 'contracts' ? 'No market contracts at this date.' : 'No open requests.'),
        emptyKind: listed.unattributed ? 'unavailable' : 'empty',
        warnings,

      };
    }

    const rows: TableRow[] = listed.rows.map(row => {
      const seller = countryAt(readView, row.seller, hour);
      const buyer = countryAt(readView, row.buyer, hour);
      const allEquipment = row.equipment.map(item => item.name ?? 'Equipment name unavailable').join(', ');
      const firstIcon = row.equipment[0]?.icon ?? null;
      const art = artState(readView, urlArtHandle(firstIcon));

      const extra: TooltipExtraRow[] = [
        { label: 'Seller', value: seller.value, divider: true },
        { label: 'Buyer', value: buyer.value },
        ...row.equipment.map((item, idx): TooltipExtraRow => ({
          label: item.name ?? 'Equipment name unavailable',
          value: knownValue(units(item.amount)),
          lookupKey: item.variant,
          ...(idx === 0 ? { divider: true } : {}),
        })),
        ...row.prices.map((item): TooltipExtraRow => ({
          label: `Agreed price · ${item.name ?? 'Equipment name unavailable'}`,
          value: knownValue(cic(item.amount)),
        })),
      ];

      if (options.view === 'requests') {
        const amt = row.equipment[0]?.amount ?? null;
        const lvl = row.levels[0]?.level ?? null;
        return {
          key: row.key,
          name: allEquipment || 'Equipment',
          cells: {
            buyer: { value: buyer.value, iconSlot: true, icon: buyer.icon, iconPending: buyer.iconPending, country: row.buyer ?? undefined, countryColor: buyer.color },
            seller: { value: seller.value, iconSlot: true, icon: seller.icon, iconPending: seller.iconPending, country: row.seller ?? undefined, countryColor: seller.color },
            equipment: { value: knownValue(allEquipment || 'Equipment name unavailable'), iconSlot: true, artShape: 'wide', artName: allEquipment, ...art },
            amount: { value: amt === null ? notRecorded('Amount not recorded') : amt === 0n ? zeroValue(0n) : knownValue(amt) },
            level: { value: lvl === null ? notRecorded('Price level not recorded') : knownValue(levelText(lvl)) },
          },
          tooltipTitle: `${seller.name} \u2192 ${buyer.name}`,
          tooltipExtra: extra,
          tooltipAction: 'Click for market details',
          meta: {
            title: `${seller.name} \u2192 ${buyer.name}`,
            tags: [row.seller, row.buyer].filter((t): t is string => t !== null),
            selected: `contract:true:${row.terms.id}`,
          },
        };
      }

      const priceRaw = row.prices[0]?.amount ?? null;
      const assigned = valueFromLive(live, liveKey(row.key, 'assigned'));
      const needed = valueFromLive(live, liveKey(row.key, 'convoys'));
      const collected = valueFromLive(live, liveKey(row.key, 'collected'));
      const deliveries = valueFromLive(live, liveKey(row.key, 'deliveries'));

      extra.push(
        { label: 'Convoys assigned', value: assigned, format: 'integer', divider: true },
        { label: 'Convoys needed', value: needed, format: 'integer' },
        { label: 'Payment collected', value: collected, format: 'fixed5Amount' },
        { label: 'Completed deliveries', value: deliveries, format: 'integer' },
      );

      const cells: Record<string, TableCell> = {
        seller: { value: seller.value, iconSlot: true, icon: seller.icon, iconPending: seller.iconPending, country: row.seller ?? undefined, countryColor: seller.color },
        buyer: { value: buyer.value, iconSlot: true, icon: buyer.icon, iconPending: buyer.iconPending, country: row.buyer ?? undefined, countryColor: buyer.color },
        equipment: { value: knownValue(allEquipment || 'Equipment name unavailable'), iconSlot: true, artShape: 'wide', artName: allEquipment, ...art },
        price: { value: priceRaw === null ? notRecorded('Price not recorded') : knownValue(priceRaw) },
        convoys: { value: assigned, format: 'integer', secondaryValue: needed },
        collected: { value: collected },
        deliveries: { value: deliveries },
      };

      return {
        key: row.key,
        name: allEquipment || `${seller.name} \u2192 ${buyer.name}`,
        cells,
        tooltipTitle: `${seller.name} \u2192 ${buyer.name}`,
        tooltipExtra: extra,
        tooltipAction: 'Click for market details',
        meta: {
          title: `${seller.name} \u2192 ${buyer.name}`,
          tags: [row.seller, row.buyer].filter((t): t is string => t !== null),
          selected: `contract:false:${row.terms.id}`,
        },
      };
    });

    return { columns, rows, hour, warnings };
  },
};

for (const view of MARKET_TABS) registerTableProjection(internationalMarketProjection, view);

function marketSources(ctx: FamilyPlanContext<unknown>, view: MarketTab): SourcePlan {
  const only = view === 'inventory' ? 'offers' : 'contracts';
  const owner = `international-market:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${only}`;
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
        product: MARKET,
        followQuery: { panel: MARKET, branchId: ctx.branch.id, countries: [], only },
        params: { branchId: ctx.branch.id, countries: [], only },
        validate: validateMarketReply,
      },
    }],
  };
}

function marketDeclaration(view: MarketTab): TableWidgetDeclaration<InternationalMarketProjectionOptions> {
  return {
    family: 'table',
    kind: 'international-market',
    label: 'International market',
    subject: 'international market',
    emptyCampaign: 'Load a campaign to see the market.',
    icon: 'globe',
    optionFields: OPTIONS,
    countries: { mode: 'filter', labels: true, colours: true },
    sources: ctx => marketSources(ctx, view),
    projection: internationalMarketProjection,
    columns: marketColumns(view),
    settings: { kind: 'table', key: marketSettingsKey(view), base: marketTableBase(view) },
    projectionOptions: ctx => ({
      view,
      filterAll: ctx.countries.all,
      filter: ctx.countries.filter,
    }),
    jumpToDate: true,
    keyByBranch: true,
  };
}

export const MARKET_DECLARATIONS: Readonly<Record<MarketTab, TableWidgetDeclaration<InternationalMarketProjectionOptions>>> = {
  contracts: marketDeclaration('contracts'),
  requests: marketDeclaration('requests'),
  inventory: marketDeclaration('inventory'),
};
