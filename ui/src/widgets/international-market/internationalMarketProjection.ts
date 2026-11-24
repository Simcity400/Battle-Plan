import { liveKey } from '../../charts/live.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { MARKET, validateMarketReply } from '../../dashboard/market.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  resolveArtUrlFromReadView,
  resolveCountryFromReadView,
  type TableCell,
  type TableColumnSpec,
  type TableModel,
  type TableProjection,
  type TableRow,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY, type TableBase } from '../../families/table/settings.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import { OPTIONS, SORTS } from './definition.ts';
import { cic, contractRows, levelText, offerRows, units, type MarketTab } from './marketData.ts';

export interface InternationalMarketProjectionOptions {
  readonly view: MarketTab;
  readonly filterAll: boolean;
  readonly filter: FamilyPlanContext<unknown>['countries']['filter'];
}

export function marketColumns(view: MarketTab): TableColumnSpec[] {
  if (view === 'inventory') {
    return [
      {
        key: 'seller',
        label: 'Seller',
        as: 'flag',
        offered: OFFERED.country,
        showNamesDefault: false,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
      },
      {
        key: 'equipment',
        label: 'Equipment',
        as: 'icon',
        cellKind: 'item',
        offered: OFFERED.text,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
        minWidth: 200,
      },
      {
        key: 'amount',
        label: 'On market',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        widest: ['99,999'],
      },
      {
        key: 'level',
        label: 'Price level',
        as: 'plain',
        barEligible: false,
        offered: OFFERED.text,
        priority: 2,
        text: true,
        minWidth: 104,
      },
    ];
  }
  if (view === 'requests') {
    return [
      {
        key: 'buyer',
        label: 'Buyer',
        as: 'flag',
        offered: OFFERED.country,
        showNamesDefault: false,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
      },
      {
        key: 'seller',
        label: 'Seller',
        as: 'flag',
        offered: OFFERED.country,
        showNamesDefault: false,
        pinned: true,
        priority: 0,
        text: true,
        hideable: false,
      },
      {
        key: 'equipment',
        label: 'Wants',
        as: 'icon',
        cellKind: 'item',
        offered: OFFERED.text,
        priority: 1,
        text: true,
        minWidth: 200,
      },
      {
        key: 'amount',
        label: 'Amount',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        widest: ['99,999'],
      },
      {
        key: 'level',
        label: 'Price offered',
        as: 'plain',
        barEligible: false,
        offered: OFFERED.text,
        priority: 2,
        text: true,
        minWidth: 110,
      },
    ];
  }
  return [
    {
      key: 'seller',
      label: 'Seller',
      as: 'flag',
      offered: OFFERED.country,
      showNamesDefault: false,
      pinned: true,
      priority: 0,
      text: true,
      hideable: false,
    },
    {
      key: 'buyer',
      label: 'Buyer',
      as: 'flag',
      offered: OFFERED.country,
      showNamesDefault: false,
      pinned: true,
      priority: 0,
      text: true,
      hideable: false,
    },
    {
      key: 'equipment',
      label: 'Equipment',
      as: 'icon',
      cellKind: 'item',
      offered: OFFERED.text,
      priority: 1,
      text: true,
      minWidth: 200,
    },
    {
      key: 'price',
      label: 'Price',
      unit: 'CIC',
      as: 'plain',
      barEligible: false,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      minWidth: 84,
      widest: ['99,999'],
    },
    {
      key: 'convoys',
      label: 'Convoys',
      as: 'plain',
      cellKind: 'outOf',
      barEligible: false,
      offered: OFFERED.number,
      priority: 3,
      numeric: true,
      minWidth: 86,
      widest: ['99 / 99'],
    },
    {
      key: 'collected',
      label: 'Collected',
      unit: 'CIC',
      as: 'faint',
      barEligible: true,
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      minWidth: 96,
      widest: ['99,999'],
    },
    {
      key: 'deliveries',
      label: 'Deliveries',
      as: 'faint',
      barEligible: true,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'integer',
      minWidth: 88,
      widest: ['999'],
    },
  ];
}

export function marketTableBase(view: MarketTab): TableBase {
  return {
    density: 'normal',
    bars: 'all',
    overflow: 'scroll',
    showRank: false,
    sort: SORTS[view],
  };
}

function makeMatcher(filterAll: boolean, filter: InternationalMarketProjectionOptions['filter']) {
  return (tag: string, atHour: number): boolean => {
    if (filterAll || !filter.passing) return true;
    return filter.passing[tag]?.some(([first, last]) => first <= atHour && atHour <= last) ?? false;
  };
}

function readLiveNum(live: ReturnType<BoundedReadView['live']>, key: string, col: string): number | null {
  if (!live) return null;
  const v = live.get(liveKey(key, col));
  return typeof v === 'number' && Number.isFinite(v) ? v : null;
}

export const internationalMarketProjection: TableProjection<InternationalMarketProjectionOptions> = registerTableProjection({
  kind: 'international-market',
  family: 'table',
  project(readView: BoundedReadView, options: InternationalMarketProjectionOptions): TableModel {
    const hour = readView.hour;
    const columns = marketColumns(options.view);
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns, rows: [], hour, refused, rowHeightMode: 'media' };
    }
    const rawPanel = readView.panel('main');
    if (rawPanel === undefined) {
      return { columns, rows: [], hour, rowHeightMode: 'media' };
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
        rowHeightMode: 'media',
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
          rowHeightMode: 'media',
        };
      }
      const rows: TableRow[] = offers.map(row => {
        const seller = resolveCountryFromReadView(readView, row.seller, hour);
        const eqName = row.name ?? 'Equipment name unavailable';
        const art = resolveArtUrlFromReadView(readView, row.icon);
        const amountNum = Number(row.amount) / 100000;
        return {
          key: row.key,
          name: eqName,
          cells: {
            seller: { value: seller.value, iconSlot: true, icon: seller.icon, iconPending: seller.iconPending, country: row.seller ?? undefined, countryColor: seller.color },
            equipment: { value: knownValue(eqName), iconSlot: true, artShape: 'wide', artName: eqName, ...art },
            amount: {
              value: row.amount === 0n ? zeroValue(0) : knownValue(formatAmount(row.amount, 5)),
              sortValue: knownValue(amountNum),
            },
            level: { value: row.level === null ? notRecorded('Price level not recorded') : knownValue(levelText(row.level)) },
          },
          tooltipExtra: [
            { label: 'Seller', value: seller.value, divider: true },
            { label: 'On market', value: knownValue(units(row.amount)) },
            { label: 'Price level', value: knownValue(levelText(row.level)) },
          ],
          tooltipAction: 'Click for market details',
          meta: { title: `${seller.name}: market`, tags: row.seller ? [row.seller] : [], selected: `offer:${row.offer.country}` },
        };
      });
      return { columns, rows, hour, warnings: reply.warnings, rowHeightMode: 'media' };
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
        rowHeightMode: 'media',
      };
    }

    const rows: TableRow[] = listed.rows.map(row => {
      const seller = resolveCountryFromReadView(readView, row.seller, hour);
      const buyer = resolveCountryFromReadView(readView, row.buyer, hour);
      const allEquipment = row.equipment.map(item => item.name ?? 'Equipment name unavailable').join(', ');
      const firstIcon = row.equipment[0]?.icon ?? null;
      const art = resolveArtUrlFromReadView(readView, firstIcon);

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
            amount: {
              value: amt === null ? notRecorded('Amount not recorded') : amt === 0n ? zeroValue(0) : knownValue(formatAmount(amt, 5)),
              sortValue: amt === null ? notRecorded('Not recorded') : knownValue(Number(amt) / 100000),
            },
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
      const assigned = readLiveNum(live, row.key, 'assigned');
      const needed = readLiveNum(live, row.key, 'convoys');
      const collected = readLiveNum(live, row.key, 'collected');
      const deliveries = readLiveNum(live, row.key, 'deliveries');

      extra.push(
        { label: 'Convoys assigned', value: assigned !== null && needed !== null ? knownValue(`${assigned} / ${needed}`) : notRecorded('Convoys not recorded'), divider: true },
        { label: 'Payment collected', value: collected !== null ? knownValue(cic(BigInt(Math.trunc(collected)))) : notRecorded('Collected not recorded') },
        { label: 'Completed deliveries', value: deliveries !== null ? knownValue(Math.round(deliveries)) : notRecorded('Deliveries not recorded'), format: 'integer' },
      );

      const cells: Record<string, TableCell> = {
        seller: { value: seller.value, iconSlot: true, icon: seller.icon, iconPending: seller.iconPending, country: row.seller ?? undefined, countryColor: seller.color },
        buyer: { value: buyer.value, iconSlot: true, icon: buyer.icon, iconPending: buyer.iconPending, country: row.buyer ?? undefined, countryColor: buyer.color },
        equipment: { value: knownValue(allEquipment || 'Equipment name unavailable'), iconSlot: true, artShape: 'wide', artName: allEquipment, ...art },
        price: {
          value: priceRaw === null ? notRecorded('Price not recorded') : knownValue(formatAmount(priceRaw, 5)),
          sortValue: priceRaw === null ? notRecorded('Not recorded') : knownValue(Number(priceRaw) / 100000),
        },
        convoys: {
          value: assigned === null ? notRecorded('Convoys not recorded') : assigned === 0 ? zeroValue(0) : knownValue(Math.round(assigned)),
          secondaryValue: needed === null ? notRecorded('Convoys not recorded') : knownValue(Math.round(needed)),
          sortValue: assigned === null ? notRecorded('Not recorded') : knownValue(assigned),
        },
        collected: {
          value: collected === null
            ? notRecorded('Payment collected not recorded')
            : Math.trunc(collected) === 0
              ? zeroValue(0)
              : knownValue(formatAmount(BigInt(Math.trunc(collected)), 5)),
          sortValue: collected === null ? notRecorded('Not recorded') : knownValue(collected / 100000),
        },
        deliveries: {
          value: deliveries === null ? notRecorded('Deliveries not recorded') : Math.round(deliveries) === 0 ? zeroValue(0) : knownValue(Math.round(deliveries)),
        },
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

    return { columns, rows, hour, warnings, rowHeightMode: 'media' };
  },
});

function marketViewOf(options: Readonly<Record<string, unknown>>): MarketTab {
  return options.view === 'requests' ? 'requests' : options.view === 'inventory' ? 'inventory' : 'contracts';
}

function marketSources(ctx: FamilyPlanContext<unknown>): SourcePlan {
  const view = marketViewOf(ctx.options);
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

export const internationalMarketDeclaration: TableWidgetDeclaration<InternationalMarketProjectionOptions> = {
  family: 'table',
  kind: 'international-market',
  label: 'International market',
  subject: 'international market',
  emptyCampaign: 'Load a campaign to see the market.',
  icon: 'globe',
  optionFields: OPTIONS,
  countries: { mode: 'filter', labels: true, colours: true },
  sources: marketSources,
  projection: internationalMarketProjection,
  columns: ctx => marketColumns(marketViewOf(ctx.options)),
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: marketTableBase('contracts') },
  projectionOptions: ctx => ({
    view: marketViewOf(ctx.options),
    filterAll: ctx.countries.all,
    filter: ctx.countries.filter,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
