import type { CountryFlowReply, FlowLink } from '../../generated/dashboard';
import { countryRowPresentation } from '../bars/model.ts';
import type { BoundedReadView } from '../contract.ts';
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

const AMOUNT_FORMAT: ValueFormat = { decimals: 0, maxDecimals: 1 };
const OTHERS_IN = '\u0000others-in';
const OTHERS_OUT = '\u0000others-out';

export function scaledAmount(amount: string, decimals: number): number {
  return decimals > 0 ? Number(BigInt(amount)) / 10 ** decimals : Number(BigInt(amount));
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
  let notRecordedKey = false;

  const node = (index: number | null, side: FlowSide, link: FlowLink | null, total: string | null, others?: number): FlowNode => {
    const missing = link ? link.total === null : total === null;
    if (missing) notRecordedKey = true;
    const amount = link ? link.total : total;
    if (index === null) {
      const label = text.others(others ?? 0);
      return {
        key: side === 'left' ? OTHERS_IN : OTHERS_OUT,
        side,
        label,
        numeric: value(amount),
        figure: figure(amount),
        others: others ?? 0,
        spoken: `${label}: ${exact(amount)}`,
        audit: exact(amount),
        ...(link ? { tooltip: { title: label, hour, items: items(link) } } : {}),
      };
    }
    const tag = reply.countries[index]!;
    const shown = countryRowPresentation(readView, tag);
    return {
      key: tag,
      side,
      label: shown.label,
      ...(shown.labelMissing ? { labelMissing: true } : {}),
      tag,
      flag: shown.media.art,
      ...(shown.color ? { color: shown.color } : {}),
      numeric: value(amount),
      figure: side === 'centre' ? '' : figure(amount),
      spoken: `${shown.label}: ${exact(amount)}`,
      audit: exact(amount),
      subject: tag,
      ...(link ? { tooltip: { title: shown.label, hour, items: items(link), ...(link.missing ? { missingReason: link.missing } : {}) } } : {}),
    };
  };

  const ribbon = (link: FlowLink, from: string, to: string, fromLabel: string, toLabel: string): FlowRibbon => ({
    key: `${from}>${to}`,
    from,
    to,
    bands: bands(link),
    numeric: value(link.total),
    audit: exact(link.total),
    tooltip: { title: text.ribbonTitle(fromLabel, toLabel), hour, items: items(link), ...(link.missing ? { missingReason: link.missing } : {}) },
  });

  if (reply.subject !== null) {
    if (!reply.incoming.length && !reply.outgoing.length) return blank(hour, { empty: text.none, warnings: reply.warnings, notes: reply.partial ? [reply.partial] : undefined });
    const centre = node(reply.subject, 'centre', null, null);
    const left = reply.incoming.map(link => node(link.from, 'left', link, null, link.others ?? undefined));
    const right = reply.outgoing.map(link => node(link.to, 'right', link, null, link.others ?? undefined));
    const ribbons = [
      ...reply.incoming.map((link, at) => ribbon(link, left[at]!.key, centre.key, left[at]!.label, centre.label)),
      ...reply.outgoing.map((link, at) => ribbon(link, centre.key, right[at]!.key, centre.label, right[at]!.label)),
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
      ...(reply.partial ? { notes: [reply.partial] } : {}),
      warnings: reply.warnings,
    };
  }

  if (!reply.links.length) return blank(hour, { mode: 'bipartite', empty: text.none, warnings: reply.warnings });
  const left = reply.senders.map(link => node(link.from, 'left', link, null, link.others ?? undefined));
  const right = reply.receivers.map(link => node(link.to, 'right', link, null, link.others ?? undefined));
  const keyOf = (index: number | null, side: 'left' | 'right') => (index === null ? (side === 'left' ? OTHERS_IN : OTHERS_OUT) : reply.countries[index]!);
  const labelOf = (index: number | null, side: 'left' | 'right') => (side === 'left' ? left : right).find(item => item.key === keyOf(index, side))?.label ?? '';
  const ribbons = reply.links.map(link => ribbon(link, keyOf(link.from, 'left'), keyOf(link.to, 'right'), labelOf(link.from, 'left'), labelOf(link.to, 'right')));
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
    ...(reply.partial ? { notes: [reply.partial] } : {}),
    warnings: reply.warnings,
  };
}

