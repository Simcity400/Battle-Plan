import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountryFlowReply, FlowEndpoint, FlowLink, FlowSide as ReplySide } from '../../generated/dashboard';
import { countryRowPresentation } from '../bars/model.ts';
import { LOADING_NAME } from '../table/identityCell.ts';
import type { ArtResult, BoundedReadView } from '../contract.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { formatCompactScalar, formatScalar, type ValueFormat } from '../valueFormat.ts';
import type { FlowBand, FlowModel, FlowNode, FlowPartKey, FlowRibbon, FlowSide } from './model.ts';

export interface CountryFlowText {
  readonly leftHeader: string;
  readonly rightHeader: string;
  readonly unitNote?: string;
  readonly unit?: string | readonly [one: string, many: string];
  readonly groupLeftHeader: string;
  readonly groupRightHeader: string;
  readonly notRecorded: string;
  readonly none: string;
  readonly others: (count: number) => string;
  readonly ribbonTitle: (from: string, to: string) => string;
}

export const UNKNOWN_COUNTRY = 'Unknown country';
export const UNKNOWN_COUNTRIES = 'Unknown countries, combined';

export function scaledAmount(amount: string, decimals: number): number {
  return decimals > 0 ? Number(BigInt(amount)) / 10 ** decimals : Number(BigInt(amount));
}

interface FlowEndpointPresentation {
  readonly key: string;
  readonly label: string;
  readonly labelMissing: boolean;
  readonly labelLoading?: true;
  readonly subject?: string;
  readonly tag?: string;
  readonly flag?: ArtResult;
  readonly flagKey?: string;
  readonly unknown?: string;
}

export function flowEndpointKey(endpoint: FlowEndpoint): string {
  return endpoint.key;
}

export function unknownEndpointLabel(endpoint: FlowEndpoint): string {
  return endpoint.collective ? UNKNOWN_COUNTRIES : UNKNOWN_COUNTRY;
}

export function presentFlowEndpoint(readView: BoundedReadView, endpoint: FlowEndpoint): FlowEndpointPresentation {
  const key = flowEndpointKey(endpoint);
  if (endpoint.slot === null) {
    const label = unknownEndpointLabel(endpoint);
    return { key, label, labelMissing: true, unknown: endpoint.unknown ?? label };
  }
  const subject = countryKey(endpoint.slot);
  const shown = countryRowPresentation(readView, subject);
  const labelLoading = shown.label.kind === 'pending';
  const labelMissing = shown.label.kind !== 'known' && !labelLoading;
  return {
    key,
    label: shown.label.kind === 'known' ? shown.label.value : labelLoading ? LOADING_NAME : endpoint.tag ?? UNKNOWN_COUNTRY,
    labelMissing,
    ...(labelLoading ? { labelLoading: true as const } : {}),
    subject,
    ...(endpoint.tag ? { tag: endpoint.tag } : {}),
    ...(endpoint.unknown ? { unknown: endpoint.unknown } : {}),
    flag: shown.media.art,
    ...(shown.flagKey ? { flagKey: shown.flagKey } : {}),
  };
}

export function flowSideNotes(...sides: readonly ReplySide[]): readonly string[] {
  return [...new Set(sides.flatMap(side => [
    ...(side.missing ? [side.missing] : []),
    ...(side.unranked ? [`Top countries not applied, every partner shown: ${side.unranked}`] : []),
  ]))];
}

function blank(hour: number, extra: Partial<FlowModel>): FlowModel {
  return { hour, mode: 'centred', left: [], right: [], ribbons: [], parts: [], ...extra };
}

export function projectCountryFlow(readView: BoundedReadView, text: CountryFlowText): FlowModel {
  const hour = readView.hour;
  const refused = readView.refused('main');
  if (refused) return blank(hour, { refused });
  const reply = readView.panel<CountryFlowReply>('main');
  if (!reply) return blank(hour, { empty: text.notRecorded, emptyKind: 'unavailable' });
  if (reply.missing) return blank(hour, { empty: text.notRecorded, emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings });

  const decimals = reply.decimals;
  const shownFormat: ValueFormat = { decimals, maxDecimals: 1 };
  const plainFormat: ValueFormat = { decimals };
  const one = 10n ** BigInt(decimals);
  const unitOf = (raw: bigint) => (typeof text.unit === 'string' ? text.unit : text.unit?.[raw === one ? 0 : 1]);
  const value = (amount: string | null) => (amount === null ? null : scaledAmount(amount, decimals));
  const figure = (amount: string | null) => (amount === null ? 'Not recorded' : formatCompactScalar(BigInt(amount), shownFormat).text);
  const exact = (amount: string | null) => {
    if (amount === null) return 'Not recorded';
    const raw = BigInt(amount);
    const unit = unitOf(raw);
    return formatScalar(raw, unit ? { decimals, unit } : plainFormat);
  };
  const parts: FlowPartKey[] = reply.parts.map((part, index) => ({ key: part.key, label: part.name ?? part.key, ordinal: part.ordinal ?? index, ...(part.nameMissing ? { missing: part.nameMissing } : {}) }));
  const partNote = (part: FlowPartKey | undefined): TooltipListItem[] => (part?.missing ? [{ label: part.missing, tone: 'muted', indent: true }] : []);
  const partSaid = (part: FlowPartKey | undefined) => `${part?.label ?? ''}${part?.missing ? ` (${part.missing})` : ''}`;
  const items = (link: FlowLink): TooltipListItem[] => {
    const listed = link.amounts.flatMap((amount): TooltipListItem[] => {
      const part = parts[amount.part];
      const shown: TooltipListItem = amount.amount === null
        ? { label: part?.label ?? '', missingReason: link.missing ?? text.notRecorded }
        : { label: part?.label ?? '', value: exact(amount.amount) };
      return [shown, ...partNote(part)];
    });
    const total = link.total;
    const loses = total !== null && formatCompactScalar(BigInt(total), shownFormat).text !== formatScalar(BigInt(total), plainFormat);
    if (link.amounts.length < 2 && !loses) return listed;
    const sum: TooltipListItem = total === null
      ? { label: 'Total', strong: true, missingReason: link.missing ?? text.notRecorded }
      : { label: 'Total', strong: true, value: exact(total) };
    return [sum, ...listed];
  };
  const detail = (link: FlowLink) => {
    const said = link.amounts.map(amount => `${partSaid(parts[amount.part])}: ${amount.amount === null ? `Not recorded (${link.missing ?? text.notRecorded})` : exact(amount.amount)}`);
    return `${exact(link.total)} · ${said.join('; ')}`;
  };
  const identity = (...ends: readonly Pick<FlowNode, 'label' | 'unknown'>[]): TooltipListItem[] =>
    ends.flatMap(end => (end.unknown ? [{ label: end.label, missingReason: end.unknown }] : []));
  const identityAudit = (...ends: readonly Pick<FlowNode, 'label' | 'unknown'>[]) =>
    ends.flatMap(end => (end.unknown ? [` · ${end.label}: ${end.unknown}`] : [])).join('');
  const named = (end: Pick<FlowNode, 'label' | 'tag' | 'labelMissing'>) => `${end.label}${end.tag ? ` [${end.tag}]` : ''}${end.labelMissing ? ' (name missing)' : ''}`;
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
        spoken: `${label}: ${detail(link)}`,
        audit: `${label} · ${detail(link)}`,
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
      ...(shown.labelLoading ? { labelLoading: true } : {}),
      ...(shown.tag ? { tag: shown.tag } : {}),
      ...(shown.flag ? { flag: shown.flag } : {}),
      ...(shown.unknown ? { unknown: shown.unknown } : {}),
      numeric: value(amount),
      figure: figure(amount),
      spoken: `${shown.label}: ${detail(link)}${missingReason && link.missing === null ? ` (${missingReason})` : ''}`,
      audit: `${named(shown)} · ${detail(link)}${identityAudit(shown)}`,
      ...(shown.subject ? { subject: shown.subject } : {}),
      tooltip: { title: shown.label, hour, items: items(link), ...(missingReason ? { missingReason } : {}) },
    };
  };

  const ribbon = (link: FlowLink, from: FlowNode, to: FlowNode): FlowRibbon => ({
    key: `${from.key}>${to.key}`,
    from: from.key,
    to: to.key,
    bands: bands(link),
    ...(link.total === null ? { missing: true } : {}),
    ...(from.others !== undefined || to.others !== undefined ? { others: true } : {}),
    numeric: value(link.total),
    audit: `${detail(link)}${identityAudit(from, to)}`,
    tooltip: { title: text.ribbonTitle(from.label, to.label), hour, items: [...items(link), ...identity(from, to)], ...(link.missing ? { missingReason: link.missing } : {}) },
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
      ...(subject.labelLoading ? { labelLoading: true } : {}),
      ...(subject.tag ? { tag: subject.tag } : {}),
      ...(subject.flag ? { flag: subject.flag } : {}),
      ...(subject.unknown ? { unknown: subject.unknown } : {}),
      numeric: value(reply.largest),
      figure: '',
      spoken: `${subject.label}: ${centreAudit}`,
      audit: `${named(subject)} · ${centreAudit}`,
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
  const receiverOrder = new Map(right.map((item, index) => [item.key, index]));
  const ribbons = reply.links
    .flatMap(link => {
      const from = byKey.get(keyOf(link.from, 'left'));
      const to = byKey.get(keyOf(link.to, 'right'));
      return from && to ? [ribbon(link, from, to)] : [];
    })
    .sort((a, b) => receiverOrder.get(a.to)! - receiverOrder.get(b.to)!);
  return {
    hour,
    mode: 'bipartite',
    left,
    right,
    ribbons,
    parts,
    leftHeader: text.groupLeftHeader,
    rightHeader: text.groupRightHeader,
    ...(text.unitNote ? { unitNote: text.unitNote } : {}),
    notRecordedKey,
    ...notes(senders, receivers),
    warnings: reply.warnings,
  };
}
