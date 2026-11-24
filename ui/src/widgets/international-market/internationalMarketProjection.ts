import { liveKey } from '../../charts/live.ts';
import { MARKET, validateMarketReply } from '../../dashboard/market.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { occurrenceSelection } from '../../families/table/logHelpers.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import { cellArt, resolveIdentity, type ResolvedIdentity } from '../../families/table/identityCell.ts';
import {
  type TableCell,
  type TableModel,
  type TableProjection,
  type TableRow,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notApplicable, notRecorded, valueFromLive, zeroValue } from '../../families/value.ts';
import { MARKET_TABS, marketColumns, marketTableBase } from './definition.ts';

const UNKNOWN_COUNTRY: ResolvedIdentity = { value: notRecorded('Unknown country'), name: 'Unknown country' };
const countryAt = (readView: BoundedReadView, tag: string | null | undefined, hour: number): ResolvedIdentity =>
  tag ? resolveIdentity(readView, 'country', tag, { atHour: hour }) : UNKNOWN_COUNTRY;
import { contractRows, levelText, offerRows, PRICE_MISSING, quoteText, type MarketRowMeta, type MarketTab } from './marketData.ts';

export interface InternationalMarketProjectionOptions {
  readonly view: MarketTab;
}

const withArt = (who: ResolvedIdentity) => ({ ...(who.art ? { art: who.art } : {}), ...(who.artKey ? { artKey: who.artKey } : {}) });


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
    if (reply.refused !== null) {
      return { columns, rows: [], hour, empty: 'Not recorded', emptyDetail: reply.refused, emptyKind: 'unavailable', warnings: reply.warnings };
    }
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

    const live = readView.live('main') ?? null;
    const unattributed = reply.unattributed;

    if (options.view === 'inventory') {
      const offers = offerRows(at);
      if (!offers.length) {
        return {
          columns,
          rows: [],
          hour,
          empty: unattributed ? 'Some sellers were not recorded at this date.' : 'Nothing listed.',
          emptyKind: unattributed ? 'unavailable' : 'empty',
          warnings: reply.warnings,

        };
      }
      const rows: TableRow[] = offers.map(row => {
        const seller = countryAt(readView, row.seller, hour);
        const eqName = row.name ?? 'Equipment name unavailable';
        const art = cellArt(readView, urlArtHandle(row.icon), 'The product drew no icon for this equipment');
        return {
          key: row.key,
          name: eqName,
          cells: {
            seller: { value: seller.value, iconSlot: true, ...withArt(seller), country: row.seller ?? undefined, countryColor: seller.color },
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
          meta: { title: `${seller.name}: market`, section: 'offers', row: String(row.offer.country), variant: row.variant } satisfies MarketRowMeta,
        };
      });
      return { columns, rows, hour, warnings: reply.warnings };
    }

    const listed = contractRows(at, options.view);
    const warnings = reply.warnings;

    if (!listed.length) {
      return {
        columns,
        rows: [],
        hour,
        empty: unattributed
          ? `Some ${options.view === 'contracts' ? 'contracts' : 'requests'}' countries were not recorded at this date.`
          : (options.view === 'contracts' ? 'No market contracts at this date.' : 'No open requests.'),
        emptyKind: unattributed ? 'unavailable' : 'empty',
        warnings,

      };
    }

    const rows: TableRow[] = listed.map(row => {
      const seller = countryAt(readView, row.seller, hour);
      const buyer = countryAt(readView, row.buyer, hour);
      const allEquipment = row.equipment.map(item => item.name ?? 'Equipment name unavailable').join(', ');
      const firstIcon = row.equipment[0]?.icon ?? null;
      const art = cellArt(readView, urlArtHandle(firstIcon), 'The product drew no icon for this equipment');

      const priceOf = (variant: string) => row.prices.find(item => item.variant === variant)?.amount ?? null;
      const levelOf = (variant: string) => row.levels.find(item => item.variant === variant)?.level ?? null;
      const single = row.equipment.length === 1 ? row.equipment[0]! : null;
      const several = row.equipment.length === 0
        ? notApplicable('No equipment in the recorded terms')
        : notApplicable('Several equipment: each is listed with its own amount, price and level');
      const extra: TooltipExtraRow[] = [
        { label: 'Seller', value: seller.value, divider: true },
        { label: 'Buyer', value: buyer.value },
        ...row.equipment.map((item, idx): TooltipExtraRow => ({
          label: item.name ?? 'Equipment name unavailable',
          value: knownValue(quoteText(item.amount, priceOf(item.variant) ?? undefined, levelOf(item.variant) ?? undefined, options.view === 'contracts')),
          lookupKey: item.variant,
          ...(idx === 0 ? { divider: true } : {}),
        })),
      ];

      if (options.view === 'requests') {
        const amt = single?.amount ?? null;
        const lvl = single ? levelOf(single.variant) : null;
        return {
          key: row.key,
          name: allEquipment || 'Equipment',
          cells: {
            buyer: { value: buyer.value, iconSlot: true, ...withArt(buyer), country: row.buyer ?? undefined, countryColor: buyer.color },
            seller: { value: seller.value, iconSlot: true, ...withArt(seller), country: row.seller ?? undefined, countryColor: seller.color },
            equipment: { value: knownValue(allEquipment || 'Equipment name unavailable'), iconSlot: true, artShape: 'wide', artName: allEquipment, ...art },
            amount: { value: single === null ? several : amt === null ? notRecorded('Amount not recorded') : amt === 0n ? zeroValue(0n) : knownValue(amt) },
            level: { value: single === null ? several : lvl === null ? notRecorded('Price level not recorded') : knownValue(levelText(lvl)) },
          },
          tooltipTitle: `${seller.name} \u2192 ${buyer.name}`,
          tooltipExtra: extra,
          tooltipAction: 'Click for market details',
          meta: { title: `${seller.name} \u2192 ${buyer.name}`, section: 'contracts', row: row.terms.key, variant: null } satisfies MarketRowMeta,
        };
      }

      const priceRaw = single ? priceOf(single.variant) : null;
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
        seller: { value: seller.value, iconSlot: true, ...withArt(seller), country: row.seller ?? undefined, countryColor: seller.color },
        buyer: { value: buyer.value, iconSlot: true, ...withArt(buyer), country: row.buyer ?? undefined, countryColor: buyer.color },
        equipment: { value: knownValue(allEquipment || 'Equipment name unavailable'), iconSlot: true, artShape: 'wide', artName: allEquipment, ...art },
        price: { value: single === null ? several : priceRaw === null ? notRecorded(PRICE_MISSING) : knownValue(priceRaw) },
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
        meta: { title: `${seller.name} \u2192 ${buyer.name}`, section: 'contracts', row: row.terms.key, variant: null } satisfies MarketRowMeta,
      };
    });

    return { columns, rows, hour, warnings };
  },
};

for (const view of MARKET_TABS) registerFamilyProjection(internationalMarketProjection, view);

function marketSources(ctx: FamilyPlanContext<unknown>, view: MarketTab): SourcePlan {
  const only = view === 'inventory' ? 'offers' : view;
  const selection = occurrenceSelection(ctx);
  const params = { branchId: ctx.branch.id, countries: selection?.countries ?? [], ...(selection?.group ? { group: selection.group } : {}), only };
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
        followQuery: { panel: MARKET, ...params },
        params,
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
    countries: { mode: 'filter' },
    sources: ctx => marketSources(ctx, view),
    projection: internationalMarketProjection,
    columns: marketColumns(view),
    settings: { kind: 'table', base: marketTableBase(view) },
    projectionOptions: () => ({ view }),
    jumpToDate: true,
    keyByBranch: true,
  };
}

export const MARKET_DECLARATIONS: Readonly<Record<MarketTab, TableWidgetDeclaration<InternationalMarketProjectionOptions>>> = {
  contracts: marketDeclaration('contracts'),
  requests: marketDeclaration('requests'),
  inventory: marketDeclaration('inventory'),
};
