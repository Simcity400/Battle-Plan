import { unavailableArt } from '../../families/art/types.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentSet } from '../../dashboard/options.ts';
import { FIXED5 } from '../../dashboard/market.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import type { BoundedReadView, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { newIdentityBoundary } from '../../families/table/identityCell.ts';
import {
  LOG_TABLE_BASE,
  logCountryCell,
  logDateCell,
  logStatusCell,
  occurrenceSelection,
  pagedTailEntry,
  projectPagedSlots,
  urlArtHandle,
} from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, partialValue } from '../../families/value.ts';
import { formatFixed5Amount, formatScalar } from '../../families/valueFormat.ts';
import type { MarketContractEvent, MarketContractItem, MarketContractRow, MarketContractStatus, MarketEventsMore, MarketLedgerLine } from '../../generated/dashboard';
import { MARKET_LEDGER_COLUMNS } from './definition.ts';

export interface MarketLedgerProjectionOptions {
  readonly perspective: string | null;
  readonly selectionEmpty: string | null;
  readonly expanded?: readonly string[];
}

const GROUP = 'group:';

const HEAD_LIMIT = 256;

const CONTRACT_STATUS_TEXT: Record<MarketContractStatus, { readonly label: string; readonly mark: 'running' | 'waiting' | 'ended' | 'finished'; readonly muted?: boolean }> = {
  active: { label: 'Contract started', mark: 'running' },
  requested: { label: 'Request posted', mark: 'waiting' },
  ended: { label: 'Contract ended', mark: 'ended', muted: true },
  requestEnded: { label: 'Request ended', mark: 'ended', muted: true },
  notRecorded: { label: 'Not recorded', mark: 'ended', muted: true },
};

function formatUnits(raw: string): string {
  const n = BigInt(raw);
  return formatFixed5Amount(n < 0n ? -n : n);
}

function formatCic(raw: string, sign: -1 | 0 | 1): string {
  const abs = BigInt(raw);
  const mag = formatFixed5Amount(abs < 0n ? -abs : abs);
  if (sign < 0) return `\u2212${mag}`;
  if (sign > 0) return `+${mag}`;
  return mag;
}

function perspectiveSign(parties: { readonly seller: string | null; readonly buyer: string | null }, perspective: string | null): -1 | 0 | 1 {
  if (!perspective) return 0;
  if (parties.buyer === perspective && parties.seller !== perspective) return -1;
  if (parties.seller === perspective && parties.buyer !== perspective) return 1;
  return 0;
}

function units(amount: bigint): 'unit' | 'units' {
  return amount === FIXED5 ? 'unit' : 'units';
}

function contractParties(
  readView: BoundedReadView,
  contract: MarketContractRow,
  perspective: string | null,
  boundary: ReturnType<typeof newIdentityBoundary>,
): TableCell {
  if (perspective) {
    const isSeller = contract.seller === perspective;
    const isBuyer = contract.buyer === perspective;
    if (isSeller && !isBuyer) {
      return logCountryCell(readView, [{ tag: contract.buyer, missingLabel: contract.buyerNotRecorded ? 'Buyer not recorded' : 'Buyer name unavailable' }], contract.hour, boundary);
    }
    if (isBuyer && !isSeller) {
      return logCountryCell(readView, [{ tag: contract.seller, missingLabel: contract.sellerNotRecorded ? 'Seller not recorded' : 'Seller name unavailable' }], contract.hour, boundary);
    }
  }
  return logCountryCell(
    readView,
    [
      { tag: contract.seller, missingLabel: contract.sellerNotRecorded ? 'Seller not recorded' : 'Seller name unavailable' },
      { tag: contract.buyer, missingLabel: contract.buyerNotRecorded ? 'Buyer not recorded' : 'Buyer name unavailable' },
    ],
    contract.hour,
    boundary,
  );
}

function primaryItemSummary(items: readonly MarketContractItem[]): {
  readonly label: string | null;
  readonly subtext?: string;
  readonly missing?: string;
  readonly iconUrl: string | null;
} {
  if (!items.length) return { label: null, missing: 'Contract equipment was not recorded', iconUrl: null };
  const first = items[0]!;
  const label = first.name ?? null;
  const subtext = items.length > 1 ? `+${items.length - 1} more` : undefined;
  return {
    label,
    ...(subtext ? { subtext } : {}),
    missing: first.nameMissing ?? 'Equipment design not recorded',
    iconUrl: first.icon,
  };
}

function deliveredCell(items: readonly MarketContractItem[], missing: string | null): TableCell {
  let reason = missing;
  let deliveredSum = 0n;
  let contractedSum = 0n;
  let allContracted = true;
  for (const item of items) {
    reason ??= item.deliveryUnavailable;
    if (item.deliveredRaw !== null) deliveredSum += BigInt(item.deliveredRaw);
    if (item.contractedRaw === null) allContracted = false;
    else contractedSum += BigInt(item.contractedRaw);
  }
  if (reason !== null) {
    if (deliveredSum === 0n) return { value: notRecorded(reason) };
    return { value: partialValue(formatFixed5Amount(deliveredSum), reason, { atLeast: true }), unitSuffix: units(deliveredSum) };
  }
  if (!items.length) return { value: notApplicable('No deliveries yet') };
  const deliveredText = formatFixed5Amount(deliveredSum);
  if (allContracted && contractedSum > 0n) {
    return {
      value: knownValue(`${deliveredText} / ${formatFixed5Amount(contractedSum)}`),
      unitSuffix: units(contractedSum),
    };
  }
  if (deliveredSum === 0n) return { value: notApplicable('No deliveries yet') };
  return {
    value: knownValue(deliveredText),
    unitSuffix: units(deliveredSum),
  };
}

function paidCell(contract: MarketContractRow, sign: -1 | 0 | 1): TableCell {
  if (contract.paidMissing !== null) {
    return contract.paidRaw === null
      ? { value: notRecorded(contract.paidMissing) }
      : { value: partialValue(formatCic(contract.paidRaw, sign), contract.paidMissing, { atLeast: true }), unitSuffix: 'CIC' };
  }
  if (contract.paidRaw === null) return { value: notApplicable('No payments yet'), blank: true };
  return {
    value: knownValue(formatCic(contract.paidRaw, sign)),
    unitSuffix: 'CIC',
  };
}

function buildMoreRow(more: MarketEventsMore): TableRow {
  const blank: TableCell = { value: notApplicable('Not applicable to this contract event'), blank: true };
  const hidden = Number(more.hidden);
  const name = hidden > 0 ? `Show all ${formatScalar(Number(more.total))} events \u00b7 ${formatScalar(hidden)} more` : 'Show fewer events';
  return {
    key: `all:${GROUP}${more.contract}`,
    name,
    rowHour: more.hour,
    muted: true,
    pageParent: `${GROUP}${more.contract}`,
    cells: { date: blank, kind: blank, country: blank, item: { value: knownValue(name) }, delivered: blank, paid: blank, status: blank, action: blank },
  };
}

function buildChildEventRow(event: MarketContractEvent, perspective: string | null): TableRow {
  const sign = perspectiveSign(event, perspective);
  const parent = { pageParent: `${GROUP}${event.contract}` };
  const baseDate = logDateCell(event.hour);
  const blank: TableCell = { value: notApplicable('Not applicable to this contract event'), blank: true };
  if (event.kind === 'delivery') {
    const label = event.equipment ?? 'Delivery';
    const qtyCell: TableCell = event.amountRaw !== null
      ? { value: knownValue(formatUnits(event.amountRaw)), unitSuffix: BigInt(event.amountRaw) === FIXED5 ? 'unit' : 'units' }
      : { value: notRecorded('Delivery amount was not recorded') };
    return {
      key: `ev:${event.key}`,
      name: label,
      rowHour: event.hour,
      ...parent,
      cells: {
        date: baseDate,
        kind: blank,
        country: blank,
        item: { value: knownValue(label), ...(event.equipment ? { inlineSubtext: 'Delivery' } : {}) },
        delivered: qtyCell,
        paid: blank,
        status: logStatusCell(sign < 0 ? 'Bought' : sign > 0 ? 'Sold' : 'Delivered', 'finished'),
        action: blank,
      },
    };
  }
  if (event.kind === 'deliveryUnavailable') {
    const label = event.equipment ?? 'Delivery';
    return {
      key: `ev:${event.key}`,
      name: label,
      rowHour: event.hour,
      ...parent,
      muted: true,
      cells: {
        date: baseDate,
        kind: blank,
        country: blank,
        item: { value: knownValue(label), inlineSubtext: 'Delivery' },
        delivered: { value: notRecorded(event.reason ?? 'Delivery amount unavailable') },
        paid: blank,
        status: logStatusCell('Unavailable', 'ended', 'muted'),
        action: blank,
      },
    };
  }
  if (event.kind === 'payment') {
    const cicCell: TableCell = event.amountRaw !== null
      ? { value: knownValue(formatCic(event.amountRaw, sign)), unitSuffix: 'CIC' }
      : { value: notRecorded('Payment amount was not recorded') };
    return {
      key: `ev:${event.key}`,
      name: 'Payment',
      rowHour: event.hour,
      ...parent,
      cells: {
        date: baseDate,
        kind: blank,
        country: blank,
        item: { value: knownValue('Payment') },
        delivered: blank,
        paid: cicCell,
        status: logStatusCell(sign < 0 ? 'Paid' : sign > 0 ? 'Received' : 'Payment', 'finished'),
        action: blank,
      },
    };
  }
  const lifecycleLabel = event.kind === 'contractStarted'
    ? 'Contract started'
    : event.kind === 'requestPosted'
      ? 'Request posted'
      : 'Contract ended';
  const mark = event.kind === 'contractStarted' ? 'running' : event.kind === 'requestPosted' ? 'waiting' : 'ended';
  return {
    key: `ev:${event.key}`,
    name: lifecycleLabel,
    rowHour: event.hour,
    ...parent,
    muted: event.kind === 'contractEnded',
    cells: {
      date: baseDate,
      kind: blank,
      country: blank,
      item: { value: knownValue(lifecycleLabel) },
      delivered: blank,
      paid: blank,
      status: logStatusCell(lifecycleLabel, mark, event.kind === 'contractEnded' ? 'muted' : undefined),
      action: blank,
    },
  };
}

function buildContractRow(
  readView: BoundedReadView,
  contract: MarketContractRow,
  perspective: string | null,
  boundary: ReturnType<typeof newIdentityBoundary>,
  expanded: readonly string[],
): TableRow {
  const sign = perspectiveSign(contract, perspective);
  const summary = primaryItemSummary(contract.items);
  const artHandle = urlArtHandle(summary.iconUrl);
  const art = artHandle ? readView.art(artHandle) : unavailableArt(summary.missing ?? 'The product drew no icon for this item');

  const statusInfo = CONTRACT_STATUS_TEXT[contract.status];
  const title = summary.label ?? `Contract ${contract.key.slice(-6)}`;

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Contract', value: knownValue(contract.key) },
      { label: 'Seller', value: contract.seller ? knownValue(contract.seller) : notRecorded(contract.sellerNotRecorded ? 'Seller not recorded' : 'Seller name unavailable') },
      { label: 'Buyer', value: contract.buyer ? knownValue(contract.buyer) : notRecorded(contract.buyerNotRecorded ? 'Buyer not recorded' : 'Buyer name unavailable') },
    ];
    if (!contract.exact) out.push({ label: 'First seen', value: knownValue(formatDateTime(contract.hour)) });
    if (contract.startedHour !== null) out.push({ label: 'Started', value: knownValue(formatDateTime(contract.startedHour)) });
    else if (!contract.request) out.push({ label: 'Started', value: notRecorded('The contract began before its first recorded event') });
    if (contract.endedHour !== null) out.push({ label: 'Ended', value: knownValue(formatDateTime(contract.endedHour)) });
    if (contract.requestedHour !== null) out.push({ label: 'Requested', value: knownValue(formatDateTime(contract.requestedHour)) });
    for (const quote of contract.quotes) {
      out.push({ label: `Price (${quote.name ?? `Design ${quote.variant}`})`, value: knownValue(`${formatFixed5Amount(BigInt(quote.priceRaw))} CIC`) });
    }
    if (contract.termsMissing !== null) out.push({ label: 'Terms', value: notRecorded(contract.termsMissing) });
    for (const item of contract.items) {
      const itemLabel = item.name ?? `Design ${item.variant}`;
      const progress = item.deliveryUnavailable
        ? item.deliveredRaw !== null ? `At least ${formatUnits(item.deliveredRaw)} units (${item.deliveryUnavailable})` : `Not recorded (${item.deliveryUnavailable})`
        : item.contractedRaw !== null
          ? `${formatUnits(item.deliveredRaw ?? '0')} / ${formatUnits(item.contractedRaw)} units`
          : `${formatUnits(item.deliveredRaw ?? '0')} units`;
      out.push({ label: itemLabel, value: knownValue(progress) });
    }
    return out;
  };

  const events = Number(contract.eventsTotal);
  return {
    key: `${GROUP}${contract.key}`,
    name: title,
    rowHour: contract.hour,
    muted: statusInfo.muted === true,
    cells: {
      date: logDateCell(contract.hour),
      kind: {
        value: summary.label ? knownValue(summary.label) : notRecorded(summary.missing ?? 'Equipment icon not recorded'),
        art,
        iconSlot: true,
        artShape: 'equipment',
        artName: title,
      },
      country: contractParties(readView, contract, perspective, boundary),
      item: summary.label !== null
        ? { value: knownValue(summary.label), ...(summary.subtext ? { inlineSubtext: summary.subtext } : {}) }
        : { value: notRecorded(summary.missing ?? 'Equipment design not recorded') },
      delivered: deliveredCell(contract.items, contract.deliveryMissing),
      paid: paidCell(contract, sign),
      status: logStatusCell(statusInfo.label, statusInfo.mark, statusInfo.muted ? 'muted' : undefined),
      action: { value: notApplicable('Expand contract'), blank: true },
    },
    ...(events > 0 ? { pageGroup: { members: events, open: expanded.includes(`${GROUP}${contract.key}`) } } : {}),
    tooltipTitle: title,
    tooltipExtra,
  };
}

export const marketLedgerProjection: TableProjection<MarketLedgerProjectionOptions> = registerFamilyProjection({
  kind: 'market-ledger',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    if (options.selectionEmpty) {
      return {
        columns: MARKET_LEDGER_COLUMNS,
        rows: [],
        hour,
        hideHeader: true,
        unsorted: true,
        dateGrouped: true,
        empty: options.selectionEmpty,
        emptyKind: 'unavailable',
      };
    }
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: MARKET_LEDGER_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused };
    }
    const source = readView.rows<MarketLedgerLine & DatedLogRow>('main');
    if (!source) {
      return { columns: MARKET_LEDGER_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true };
    }
    if (source.refused !== null) {
      return { columns: MARKET_LEDGER_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused: source.refused };
    }
    const boundary = newIdentityBoundary();
    const { slots, rows, windowOffset, totalRows, gapBefore } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      line => line.line === 'contract'
        ? buildContractRow(readView, line, options.perspective, boundary, options.expanded ?? [])
        : line.line === 'event' ? buildChildEventRow(line, options.perspective) : buildMoreRow(line),
    );
    const rawHead = source.rawHead as { readonly warnings?: readonly string[] } | undefined;
    return {
      columns: MARKET_LEDGER_COLUMNS,
      rows,
      slots,
      hour,
      hideHeader: true,
      unsorted: true,
      dateGrouped: true,
      windowOffset,
      totalRows,
      ...(gapBefore?.length ? { gapBefore } : {}),
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      warnings: rawHead?.warnings,
      ...(slots.length === 0 ? { empty: 'No market contracts match these settings.' } : {}),
    };
  },
});

export const marketLedgerDeclaration: TableWidgetDeclaration<MarketLedgerProjectionOptions> = {
  family: 'table',
  kind: 'market-ledger',
  label: 'Market ledger',
  subject: 'market ledger',
  emptyCampaign: 'Load a campaign to see market activity.',
  icon: 'history',
  countries: { mode: 'multi' },
  columns: MARKET_LEDGER_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx): SourcePlan {
    const all = ctx.countries.all;
    const tags = all ? null : ctx.countries.tags;
    if (!all && (!tags || tags.length === 0)) {
      return { owner: `${ctx.track.source.id}:${ctx.branch.id}:market-ledger:empty`, entries: [], allowEmptyFollow: true };
    }
    const win = !ctx.following ? resolveWindow(ctx.widget, ctx.track, [ctx.track], ctx.branch.lastHour) : null;
    const rawFrom = ctx.widget.time.mode === 'range' && win ? Math.max(Math.floor(win.firstHour), ctx.branch.firstHour) : null;
    const boundFrom = rawFrom !== null ? ctx.gateSpan(rawFrom, ctx.branch.lastHour)?.firstHour ?? null : null;
    const statuses: MarketContractStatus[] = ['active', 'notRecorded'];
    if (ctx.options.showRequests !== false) statuses.push('requested');
    if (ctx.options.showEnded !== false) statuses.push('ended', 'requestEnded');
    const selection = occurrenceSelection(ctx);
    const opened = contentSet(ctx.options.expanded).filter(key => key.startsWith(GROUP));
    const expanded = opened.map(key => key.slice(GROUP.length));
    const full = contentSet(ctx.options.full).filter(key => opened.includes(key)).map(key => key.slice(GROUP.length));
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:market-ledger:${statuses.join(',')}:${selection ? JSON.stringify(selection) : 'all'}:${expanded.join(',')}:${full.join(',')}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'market-ledger-tail',
            branchId: ctx.branch.id,
            from: boundFrom,
            limit: HEAD_LIMIT,
            ask: { ...(statuses.length === 5 ? {} : { statuses }), ...selection, ...(expanded.length ? { expanded } : {}), ...(full.length ? { full } : {}) },
          },
          ctx.following,
          DEFAULT_TAIL_PAGE_SIZE,
        ),
      ],
    };
  },
  projection: marketLedgerProjection,
  projectionOptions(ctx): MarketLedgerProjectionOptions {
    const all = ctx.countries.all;
    const tags = all ? null : ctx.countries.tags;
    const ref = tags && tags.length === 1 ? countryFromKey(tags[0]!) : null;
    const perspective = typeof ref === 'string' ? ref : null;
    const selectionEmpty = !all && (!tags || tags.length === 0)
      ? (ctx.widget.countries.kind === 'group' || ctx.widget.countries.kind === 'factions'
          ? 'This group has no members in this campaign.'
          : 'Choose a country in the widget settings.')
      : null;
    return {
      perspective,
      selectionEmpty,
      expanded: contentSet(ctx.options.expanded),
    };
  },
};
