import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountryFlowReply, FlowEndpoint, FlowLink, FlowSide as ReplySide } from '../../generated/dashboard';
import { countryRowPresentation } from '../bars/model.ts';
import type { ArtResult, BoundedReadView } from '../contract.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { formatCompactScalar, formatScalar, type ValueFormat } from '../valueFormat.ts';
import type { FlowBand, FlowModel, FlowNode, FlowPartKey, FlowRibbon, FlowSide } from './model.ts';

export interface CountryFlowText {
  readonly leftHeader: string;
  readonly rightHeader: string;
  readonly unitNote?: string;
  readonly groupLeftHeader: string;
  readonly groupRightHeader: string;
  readonly notRecorded: string;
  readonly none: string;
  readonly others: (count: number) => string;
  readonly ribbonTitle: (from: string, to: string) => string;
}

export const UNKNOWN_COUNTRY = 'Unknown country';
const AMOUNT_FORMAT: ValueFormat = { decimals: 0, maxDecimals: 1 };

export function scaledAmount(amount: string, decimals: number): number {
  return decimals > 0 ? Number(BigInt(amount)) / 10 ** decimals : Number(BigInt(amount));
}

export interface FlowEndpointPresentation {
  readonly key: string;
  readonly label: string;
  readonly labelMissing: boolean;
  readonly subject?: string;
  readonly tag?: string;
  readonly flag?: ArtResult;
  readonly color?: string;
  readonly unknown?: string;
}

export function flowEndpointKey(endpoint: FlowEndpoint): string {
  return endpoint.slot === null ? 'unknown' : String(endpoint.slot);
}

export function presentFlowEndpoint(readView: BoundedReadView, endpoint: FlowEndpoint): FlowEndpointPresentation {
  const key = flowEndpointKey(endpoint);
  if (endpoint.slot === null) return { key, label: UNKNOWN_COUNTRY, labelMissing: true, unknown: endpoint.unknown ?? UNKNOWN_COUNTRY };
  const subject = countryKey(endpoint.slot);
  const shown = countryRowPresentation(readView, subject);
  const named = shown.label !== subject;
  return {
    key,
    label: named ? shown.label : endpoint.tag ?? UNKNOWN_COUNTRY,
    labelMissing: shown.labelMissing || !named,
    subject,
    ...(endpoint.tag ? { tag: endpoint.tag } : {}),
    flag: shown.media.art,
    ...(shown.color ? { color: shown.color } : {}),
  };
}

export function flowSideNotes(...sides: readonly ReplySide[]): readonly string[] {
  return [...new Set(sides.flatMap(side => (side.missing ? [side.missing] : [])))];
}

function blank(hour: number, extra: Partial<FlowModel>): FlowModel {
  return { hour, mode: 'centred', left: [], right: [], ribbons: [], parts: [], scaleMax: 0, ...extra };
}

export function projectCountryFlow(readView: BoundedReadView, text: CountryFlowText): FlowModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return blank(hour, { refused });
  const reply = readView.panel<CountryFlowReply>('main');
  if (!reply) return blank(hour, { empty: text.notRecorded, emptyKind: 'unavailable' });
  if (reply.missing) return blank(hour, { empty: text.notRecorded, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });

  const decimals = reply.decimals;
  const value = (amount: string | null) => (amount === null ? null : scaledAmount(amount, decimals));
  const figure = (amount: string | null) => (amount === null ? 'Not recorded' : formatCompactScalar(scaledAmount(amount, decimals), AMOUNT_FORMAT).text);
  const exact = (amount: string | null) => (amount === null ? 'Not recorded' : formatScalar(scaledAmount(amount, decimals), AMOUNT_FORMAT));
  const parts: FlowPartKey[] = reply.parts.map((part, index) => ({ key: part.key, label: part.name ?? part.key, ordinal: part.ordinal ?? index }));
  const items = (link: FlowLink): TooltipListItem[] => link.amounts.map(amount => {
    const part = parts[amount.part];
    return amount.amount === null
      ? { label: part?.label ?? '', missingReason: link.missing ?? text.notRecorded }
      : { label: part?.label ?? '', value: exact(amount.amount) };
  });
  const bands = (link: FlowLink): FlowBand[] => link.amounts.filter(amount => amount.amount !== null).map(amount => ({ part: amount.part, numeric: scaledAmount(amount.amount!, decimals) }));
  const endpoints = reply.endpoints.map(endpoint => presentFlowEndpoint(readView, endpoint));
  const keyOf = (index: number | null, side: FlowSide) => `${side}:${index === null ? 'others' : endpoints[index]!.key}`;
  const notes = (...sides: readonly ReplySide[]) => {
    const said = flowSideNotes(...sides);
    return said.length ? { notes: said } : {};
  };
  let notRecordedKey = false;

  const node = (index: number | null, side: FlowSide, link: FlowLink, others?: number): FlowNode => {
    if (link.total === null) notRecordedKey = true;
    const key = keyOf(index, side);
    const amount = link.total;
    if (index === null) {
      const label = text.others(others ?? 0);
      return {
        key,
        side,
        label,
        numeric: value(amount),
        figure: figure(amount),
        others: others ?? 0,
        spoken: `${label}: ${exact(amount)}`,
        audit: exact(amount),
        tooltip: { title: label, hour, items: items(link) },
      };
    }
    const shown = endpoints[index]!;
    const missingReason = link.missing ?? shown.unknown;
    return {
      key,
      side,
      label: shown.label,
      ...(shown.labelMissing ? { labelMissing: true } : {}),
      ...(shown.tag ? { tag: shown.tag } : {}),
      ...(shown.flag ? { flag: shown.flag } : {}),
      ...(shown.color ? { color: shown.color } : {}),
      ...(shown.unknown ? { unknown: shown.unknown } : {}),
      numeric: value(amount),
      figure: figure(amount),
      spoken: `${shown.label}: ${exact(amount)}`,
      audit: exact(amount),
      ...(shown.subject ? { subject: shown.subject } : {}),
      tooltip: { title: shown.label, hour, items: items(link), ...(missingReason ? { missingReason } : {}) },
    };
  };

  const ribbon = (link: FlowLink, from: FlowNode, to: FlowNode): FlowRibbon => ({
    key: `${from.key}>${to.key}`,
    from: from.key,
    to: to.key,
    bands: bands(link),
    numeric: value(link.total),
    audit: exact(link.total),
    tooltip: { title: text.ribbonTitle(from.label, to.label), hour, items: items(link), ...(link.missing ? { missingReason: link.missing } : {}) },
  });

  const unproven = (sides: readonly ReplySide[]) => blank(hour, {
    mode: reply.subject === null ? 'bipartite' : 'centred',
    empty: text.notRecorded,
    emptyDetail: flowSideNotes(...sides).join(' '),
    emptyKind: 'unavailable',
    warnings: reply.warnings,
  });

  if (reply.subject !== null) {
    const { incoming, outgoing } = reply;
    if (!incoming.links.length && !outgoing.links.length) {
      return incoming.missing || outgoing.missing ? unproven([incoming, outgoing]) : blank(hour, { empty: text.none, warnings: reply.warnings });
    }
    const subject = endpoints[reply.subject]!;
    const sideAudit = (label: string, side: ReplySide) => `${label} ${side.total === null ? `Not recorded: ${side.missing ?? text.notRecorded}` : exact(side.total)}`;
    const centreAudit = `${sideAudit(text.leftHeader, incoming)} · ${sideAudit(text.rightHeader, outgoing)}`;
    const centre: FlowNode = {
      key: keyOf(reply.subject, 'centre'),
      side: 'centre',
      label: subject.label,
      ...(subject.labelMissing ? { labelMissing: true } : {}),
      ...(subject.tag ? { tag: subject.tag } : {}),
      ...(subject.flag ? { flag: subject.flag } : {}),
      numeric: value(reply.largest),
      figure: '',
      spoken: `${subject.label}: ${centreAudit}`,
      audit: centreAudit,
      ...(subject.subject ? { subject: subject.subject } : {}),
    };
    const left = incoming.links.map(link => node(link.from, 'left', link, link.others ?? undefined));
    const right = outgoing.links.map(link => node(link.to, 'right', link, link.others ?? undefined));
    const ribbons = [
      ...incoming.links.map((link, at) => ribbon(link, left[at]!, centre)),
      ...outgoing.links.map((link, at) => ribbon(link, centre, right[at]!)),
    ];
    return {
      hour,
      mode: 'centred',
      left,
      right,
      centre,
      ribbons,
      parts,
      scaleMax: value(reply.largest) ?? 0,
      leftHeader: text.leftHeader,
      rightHeader: text.rightHeader,
      ...(text.unitNote ? { unitNote: text.unitNote } : {}),
      notRecordedKey,
      ...notes(incoming, outgoing),
      warnings: reply.warnings,
    };
  }

  const { senders, receivers } = reply;
  if (!reply.links.length) {
    return senders.missing || receivers.missing ? unproven([senders, receivers]) : blank(hour, { mode: 'bipartite', empty: text.none, warnings: reply.warnings });
  }
  const left = senders.links.map(link => node(link.from, 'left', link, link.others ?? undefined));
  const right = receivers.links.map(link => node(link.to, 'right', link, link.others ?? undefined));
  const byKey = new Map([...left, ...right].map(item => [item.key, item]));
  const ribbons = reply.links.flatMap(link => {
    const from = byKey.get(keyOf(link.from, 'left'));
    const to = byKey.get(keyOf(link.to, 'right'));
    return from && to ? [ribbon(link, from, to)] : [];
  });
  return {
    hour,
    mode: 'bipartite',
    left,
    right,
    ribbons,
    parts,
    scaleMax: value(reply.largest) ?? 0,
    leftHeader: text.groupLeftHeader,
    rightHeader: text.groupRightHeader,
    ...(text.unitNote ? { unitNote: text.unitNote } : {}),
    notRecordedKey,
    ...notes(senders, receivers),
    warnings: reply.warnings,
  };
}
