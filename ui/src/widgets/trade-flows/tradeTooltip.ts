import { dayLabel } from '../../charts/draw.ts';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { NameAt } from '../../dashboard/countryNamesAt';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { measureAmount, measureNote, measureRows, resourceName } from './tradeData.ts';
import type { Flow, Measure, SideLink, SideNode, TradeCountryFact } from './tradeData';

export const TOOLTIP_LIMIT = 8;

interface TooltipOptions {
  measure: Measure;
  from: number;
  hour: number;
  decimals: number;
  note?: string;
  viewport: number;
  nodeName: (node: SideNode) => string;
  labelAt: (hour: number) => CountryLabel;
  countryAt: (identity: string, hour: number) => TradeCountryFact | null;
  ask: (wanted: readonly NameAt[]) => void;
}

function tooltipContext(options: TooltipOptions) {
  const { measure, from, hour, decimals, nodeName, labelAt } = options;
  const fact = (identity: string) => options.countryAt(identity, hour);
  const nativeLabel = (identity: string): ReturnType<CountryLabel> | undefined => {
    const at = fact(identity);
    return at ? labelAt(at.hour)(at.tag) : undefined;
  };
  const named = (identity: string) => nativeLabel(identity) ?? { name: 'Country name unavailable' };
  return {
    named, nativeLabel,
    ask: (identities: readonly (string | null)[]) => {
      const wanted = new Map<string, NameAt>();
      for (const identity of identities) {
        const at = identity == null ? null : fact(identity);
        if (at) wanted.set(JSON.stringify([at.hour, at.tag]), { tag: at.tag, hour: at.hour });
      }
      options.ask([...wanted.values()].sort((a, b) => a.hour - b.hour || compareTags(a.tag, b.tag)));
    },
    name: (node: SideNode) => node.identity ? named(node.identity).name : nodeName(node),
    amount: (flows: readonly Flow[]) => formatAmount(measureAmount(flows, measure, from, hour), decimals),
    subtitle: `${measureNote(measure)} · ${measure === 'current' ? dayLabel(hour) : `${dayLabel(from)} – ${dayLabel(hour)}`}`,
  };
}

export function ribbonTooltip(link: SideLink, source: SideNode, target: SideNode, options: TooltipOptions): TooltipContent {
  const { measure, from, hour, decimals, note, viewport } = options;
  const { named, ask, name, amount, subtitle } = tooltipContext(options);
  const parts = [...link.flows].sort((a, b) => Number(measureAmount([b], measure, from, hour) - measureAmount([a], measure, from, hour)));
  const soleAggregated = parts.length === 1 && (source.tag == null || target.tag == null);
  const listTrades = parts.length > 1;
  ask([source.identity, target.identity, ...(listTrades || soleAggregated ? parts.slice(0, TOOLTIP_LIMIT).flatMap(flow => [flow.sellerIdentity, flow.buyerIdentity]) : [])]);
  const title = soleAggregated ? `${named(parts[0].sellerIdentity).name} → ${named(parts[0].buyerIdentity).name}` : `${name(source)} → ${name(target)}`;
  const titleCountries = soleAggregated ? [named(parts[0].sellerIdentity), named(parts[0].buyerIdentity)]
    : [source.identity ? named(source.identity) : null, target.identity ? named(target.identity) : null];
  const head = { title, ...countryNameDetails(...titleCountries), icon: link.resource.icon ?? undefined, subtitle: `${resourceName(link.resource)} · ${subtitle}`, note };
  const rows = measureRows(link.flows, measure, from, hour, decimals);
  function* forms(): Generator<TooltipContent> {
    if (!listTrades) { yield { ...head, rows }; return; }
    for (let count = Math.min(TOOLTIP_LIMIT, parts.length); count >= 1; count--) {
      const listed: TooltipRow[] = parts.slice(0, count).map((flow, index) => ({ ...countryNameDetails(named(flow.sellerIdentity), named(flow.buyerIdentity)), label: `${named(flow.sellerIdentity).name} → ${named(flow.buyerIdentity).name}`,
          value: amount([flow]), indent: true, short: true, divider: index === 0 }));
      const more = parts.length - count;
      yield { ...head, rows: [...rows, ...listed, ...(more ? [{ label: `${usNumber.format(more)} more`, tone: 'muted' as const, indent: true }] : [])] };
    }
    yield { ...head, rows };
  }
  return fitTooltip(forms(), viewport, { keep: 0 });
}

interface NodeTooltipOptions extends TooltipOptions {
  resourceOrder: ReadonlyMap<number, number> | undefined;
  resourceColour: (id: number) => string | undefined;
}

export function nodeTooltip(node: SideNode, links: readonly SideLink[], options: NodeTooltipOptions): TooltipContent {
  const { note, viewport, resourceOrder, resourceColour } = options;
  const { named, nativeLabel, ask, name, amount, subtitle } = tooltipContext(options);
  const own = links.filter(link => link[node.side] === node.id);
  const sells = node.side === 'source';
  const byResource = new Map<number, Flow[]>();
  for (const link of own) {
    const list = byResource.get(link.resource.id) ?? [];
    for (const flow of link.flows) list.push(flow);
    byResource.set(link.resource.id, list);
  }
  const gameOrder = (id: number) => resourceOrder?.get(id) ?? 0;
  const resourceRows: TooltipRow[] = [...byResource].sort((a, b) => gameOrder(a[0]) - gameOrder(b[0])).map(([id, list]) => {
    const resource = list[0].resource;
    return { label: resourceName(resource), swatch: resourceColour(id), icon: resource.icon ?? undefined, value: amount(list), short: true };
  });
  const everything = own.flatMap(link => link.flows);
  const partners = new Set(everything.map(flow => sells ? flow.buyerIdentity : flow.sellerIdentity));
  const members = node.identity ? [] : node.members;
  ask(node.identity ? [node.identity] : members.slice(0, TOOLTIP_LIMIT));
  const head = { title: name(node), countryFlag: node.identity ? nativeLabel(node.identity) : undefined,
    subtitle: `${sells ? 'Sells' : 'Buys'} · ${subtitle}`, note };
  const total: TooltipRow = { label: `${sells ? 'Sold to' : 'Bought from'} ${usNumber.format(partners.size)} ${partners.size === 1 ? 'country' : 'countries'}`,
    value: amount(everything), strong: true, divider: true };
  function* forms(): Generator<TooltipContent> {
    for (let count = Math.min(TOOLTIP_LIMIT, members.length); count >= 0; count--) {
      const listed: TooltipRow[] = members.slice(0, count).map((tag, index) => ({ label: named(tag).name, countryFlag: nativeLabel(tag), divider: index === 0, short: true,
        value: amount(everything.filter(flow => (sells ? flow.sellerIdentity : flow.buyerIdentity) === tag)) }));
      const more = members.length - count;
      yield { ...head, rows: [...resourceRows, total, ...listed, ...(members.length && more ? [{ label: `${usNumber.format(more)} more`, tone: 'muted' as const }] : [])] };
    }
  }
  return fitTooltip(forms(), viewport, { keep: 1 });
}
