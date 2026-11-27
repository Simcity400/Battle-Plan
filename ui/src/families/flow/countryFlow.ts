import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountryFlowReply, FlowEndpoint, FlowLink, FlowSide as ReplySide } from '../../generated/dashboard';
import { barsLabelSpoken, barsLabelText, countryRowPresentation } from '../bars/model.ts';
import { knownValue, notRecorded, type Value } from '../value.ts';
import type { ArtResult, BoundedReadView } from '../contract.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { formatCompactScalar, formatScalar, type ValueFormat } from '../valueFormat.ts';
import type { FlowBand, FlowFold, FlowModel, FlowNode, FlowPartKey, FlowProjectionOptions, FlowRibbon, FlowSide } from './model.ts';

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
  readonly label: Value<string>;
  readonly placeholder?: string;
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
    return { key, label: notRecorded(endpoint.unknown ?? label), placeholder: label, unknown: endpoint.unknown ?? label };
  }
  const subject = countryKey(endpoint.slot);
  const shown = countryRowPresentation(readView, subject);
  return {
    key,
    label: shown.label.kind === 'notRecorded' && endpoint.unknown ? notRecorded(endpoint.unknown) : shown.label,
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
    ...(side.unranked ? [`Top countries not applied${side.links.some(link => link.others !== null) ? '' : ', every partner shown'}: ${side.unranked}`] : []),
  ]))];
}

function blank(hour: number, extra: Partial<FlowModel>): FlowModel {
  return { hour, mode: 'centred', left: [], right: [], ribbons: [], parts: [], ...extra };
}

export function foldedFlowMembers(reply: CountryFlowReply, fold: Pick<FlowFold, 'side' | 'count' | 'total'>) {
  const left = fold.side === 'left';
  const side = reply.subject === null ? (left ? reply.senders : reply.receivers) : (left ? reply.incoming : reply.outgoing);
  const ranked = side.links.flatMap(link => {
    const index = left ? link.from : link.to;
    const endpoint = index === null ? undefined : reply.endpoints[index];
    return endpoint && !endpoint.collective && link.total !== null ? [{ endpoint, total: link.total }] : [];
  });
  const members = ranked.slice(-fold.count);
  const sum = members.reduce((total, member) => total + BigInt(member.total), 0n);
  return members.length === fold.count && sum === BigInt(fold.total) ? members : null;
}

export function projectCountryFlow(readView: BoundedReadView, text: CountryFlowText, { foldRead }: FlowProjectionOptions = {}): FlowModel {
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
  const parts: FlowPartKey[] = reply.parts.map((part, index) => ({ key: part.key, label: part.name?.trim() ? knownValue(part.name) : notRecorded(part.nameMissing ?? 'Resource name not recorded'), ordinal: part.ordinal ?? index }));
  const partNote = (part: FlowPartKey | undefined): TooltipListItem[] => (part?.label.kind === 'notRecorded' ? [{ label: part.label.reason, tone: 'muted', indent: true }] : []);
  const partSaid = (part: FlowPartKey | undefined) => `${barsLabelText(part?.label, 'Name', part?.key)}${part?.label.kind === 'notRecorded' ? ` (${part.label.reason})` : ''}`;
  const items = (link: FlowLink): TooltipListItem[] => {
    const listed = link.amounts.flatMap((amount): TooltipListItem[] => {
      const part = parts[amount.part];
      const shown: TooltipListItem = amount.amount === null
        ? { label: barsLabelText(part?.label, 'Name', part?.key), missingReason: link.missing ?? text.notRecorded }
        : { label: barsLabelText(part?.label, 'Name', part?.key), value: exact(amount.amount) };
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
  const identity = (end: Pick<FlowNode, 'label' | 'placeholder' | 'unknown'>, label = 'Name'): TooltipListItem[] =>
    [
      ...(end.label.kind === 'pending' ? [{ label, pendingReason: end.label.reason }] : []),
      ...(end.label.kind === 'notRecorded' ? [{ label: end.placeholder ?? label, missingReason: end.label.reason }] : []),
      ...(end.unknown && end.label.kind !== 'notRecorded' ? [{ label: barsLabelText(end.label), missingReason: end.unknown }] : []),
    ];
  const identityAudit = (...ends: readonly Pick<FlowNode, 'label' | 'placeholder' | 'unknown'>[]) =>
    ends.flatMap(end => (end.unknown ? [` · ${barsLabelText(end.label, 'Name', end.placeholder)}: ${end.unknown}`] : [])).join('');
  const named = (end: Pick<FlowNode, 'label' | 'placeholder' | 'tag'>) => `${end.placeholder ?? barsLabelSpoken(end.label)}${end.tag ? ` [${end.tag}]` : ''}`;
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
      const fold = foldRead && side !== 'centre' && others && amount !== null
        ? { read: foldRead, side, count: others, total: amount, title: `${label} · ${exact(amount)}` }
        : undefined;
      return {
        key,
        side,
        label: knownValue(label),
        numeric: value(amount),
        figure: amount === null ? figure(amount) : formatScalar(BigInt(amount), plainFormat),
        others: others ?? 0,
        ...(fold ? { fold } : {}),
        spoken: `${label}: ${detail(link)}`,
        audit: `${label} · ${detail(link)}`,
        tooltip: { title: label, hour, items: items(link), ...(fold ? { note: 'Open to see these partners.' } : {}) },
      };
    }
    const shown = endpoints[index]!;
    const missingReason = link.missing ?? shown.unknown;
    return {
      key,
      side,
      label: shown.label,
      ...(shown.placeholder ? { placeholder: shown.placeholder } : {}),
      ...(shown.tag ? { tag: shown.tag } : {}),
      ...(shown.flag ? { flag: shown.flag } : {}),
      ...(shown.unknown ? { unknown: shown.unknown } : {}),
      numeric: value(amount),
      figure: figure(amount),
      spoken: `${shown.placeholder ?? barsLabelSpoken(shown.label)}: ${detail(link)}${missingReason && link.missing === null && (shown.placeholder || shown.label.kind !== 'notRecorded' || shown.label.reason !== missingReason) ? ` (${missingReason})` : ''}`,
      audit: `${named(shown)} · ${detail(link)}${identityAudit(shown)}`,
      ...(shown.subject ? { subject: shown.subject } : {}),
      tooltip: { title: barsLabelText(shown.label, 'Name', shown.placeholder ?? shown.tag), hour, items: [...items(link), ...identity(shown).filter(item => !missingReason || item.missingReason !== missingReason)], ...(missingReason ? { missingReason } : {}) },
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
    tooltip: { title: text.ribbonTitle(barsLabelText(from.label, 'Name', from.placeholder ?? from.tag), barsLabelText(to.label, 'Name', to.placeholder ?? to.tag)), hour, items: [...items(link), ...identity(from, 'From name'), ...identity(to, 'To name')], ...(link.missing ? { missingReason: link.missing } : {}) },
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
      ...(subject.placeholder ? { placeholder: subject.placeholder } : {}),
      ...(subject.tag ? { tag: subject.tag } : {}),
      ...(subject.flag ? { flag: subject.flag } : {}),
      ...(subject.unknown ? { unknown: subject.unknown } : {}),
      numeric: value(reply.largest),
      figure: '',
      spoken: `${barsLabelSpoken(subject.label)}: ${centreAudit}`,
      audit: `${named(subject)} · ${centreAudit}`,
      ...(subject.label.kind === 'known' ? {} : { tooltip: { title: barsLabelText(subject.label, 'Name', subject.placeholder ?? subject.tag), hour, items: identity(subject) } }),
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
