import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { SummedLink, SummedNode } from '../../charts/flow/sum';
import { usNumber } from '../../charts/numberFormat.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey } from '../../dashboard/countryFacts.ts';
import type { NameAt } from '../../dashboard/countryFacts';
import { transferAmount } from './territoryData.ts';
import type { TransferLink, TransferOccurrence } from './territoryData';

export interface TransferStateAt { hour: number; state: number }
export interface TransferTipRequest { countries: NameAt[]; states: TransferStateAt[] }
interface TooltipOptions {
  note?: string;
  nodeName: (node: SummedNode) => string;
  labelAt: (hour: number) => CountryLabel;
  stateNameAt: (hour: number, state: number) => string;
  ask: (request: TransferTipRequest) => void;
}

/** Ask for every country and state the hover lists, at each occurrence's own
 * hour. A campaign ribbon may name the same tag differently at several dates. */
export function transferTipRequest(states: readonly TransferOccurrence[]): TransferTipRequest {
  const countries = new Map<string, NameAt>();
  const namedStates = new Map<string, TransferStateAt>();
  for (const state of states) {
    const hour = state.dates.afterHour;
    for (const country of [state.fromCountry, state.toCountry]) countries.set(nameKey(country.hour, country.tag), { hour: country.hour, tag: country.tag });
    namedStates.set(JSON.stringify([hour, state.state]), { hour, state: state.state });
  }
  return { countries: [...countries.values()], states: [...namedStates.values()] };
}

function occurrenceRows(state: TransferOccurrence, options: TooltipOptions): TooltipRow[] {
  const hour = state.dates.afterHour;
  const giver = options.labelAt(state.fromCountry.hour)(state.fromCountry.tag);
  const taker = options.labelAt(state.toCountry.hour)(state.toCountry.tag);
  const date = state.derived === false ? formatDateTime(hour)
    : state.derived ? `By ${formatDateTime(hour)}`
      : `${formatDateTime(state.dates.beforeHour)} – ${formatDateTime(hour)}`;
  return [
    { label: options.stateNameAt(hour, state.state), value: date, divider: true, strong: true },
    { label: 'Giver', value: giver.name, countryFlag: giver, indent: true },
    { label: 'Taker', value: taker.name, countryFlag: taker, indent: true },
    { label: 'Victory points', value: state.victoryPointsPending || state.victoryPoints == null ? 'Not available at this time'
      : usNumber.format(BigInt(state.victoryPoints)), indent: true },
  ];
}

function latestOccurrence(parts: readonly TransferLink[]): TransferOccurrence | null {
  return parts.reduce<TransferOccurrence | null>((found, part) => {
    const state = part.states[part.states.length - 1];
    return !found || state.dates.afterHour > found.dates.afterHour ? state : found;
  }, null);
}

function transfersTooltip(title: string, links: readonly TransferLink[], options: TooltipOptions, scrollId: string, countryFlag?: TooltipContent['countryFlag']): TooltipContent {
  // Each link already has a date index. Show its newest occurrence first without
  // sorting the whole campaign or dropping any part of an other node.
  const states = links.flatMap(link => [...link.states].reverse());
  options.ask(transferTipRequest(states));
  const total: TooltipRow[] = [
    { label: 'Transfers', value: transferAmount(links, 'states'), strong: true },
    { label: 'Victory points', value: transferAmount(links, 'victoryPoints') },
  ];
  const head = { title, countryFlag, note: options.note };
  return { ...head, scrollable: true, scrollId, rows: [...total, ...states.flatMap(state => occurrenceRows(state, options))] };
}

export function transferRibbonTooltip(link: SummedLink<TransferLink>, source: SummedNode, target: SummedNode, options: TooltipOptions): TooltipContent {
  const latest = latestOccurrence(link.parts);
  const giver = source.tag != null && latest ? options.labelAt(latest.fromCountry.hour)(latest.fromCountry.tag) : null;
  const taker = target.tag != null && latest ? options.labelAt(latest.toCountry.hour)(latest.toCountry.tag) : null;
  return transfersTooltip(`${giver?.name ?? options.nodeName(source)} → ${taker?.name ?? options.nodeName(target)}`, link.parts, options, `link:${link.id}`, giver ?? undefined);
}

export function transferNodeTooltip(node: SummedNode, links: readonly SummedLink<TransferLink>[], options: TooltipOptions): TooltipContent {
  const parts = links.flatMap(link => link.parts);
  const latest = latestOccurrence(parts);
  const country = latest && (node.side === 'source' ? latest.fromCountry : latest.toCountry);
  const label = node.tag != null && country ? options.labelAt(country.hour)(country.tag) : null;
  return transfersTooltip(label?.name ?? options.nodeName(node), parts, options, `node:${node.id}`, label ?? undefined);
}
