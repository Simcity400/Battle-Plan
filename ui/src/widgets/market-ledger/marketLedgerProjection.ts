import type { BoundFacetFilter } from '../../charts/log/logSource';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { FIXED5, fixedText } from '../../dashboard/market.ts';
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
  pagedTailEntry,
  projectPagedSlots,
  urlArtHandle,
} from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { DEFAULT_TAIL_PAGE_SIZE, type DatedLogRow } from '../../families/table/rowSource.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import type { MarketContractEvent, MarketContractItem, MarketContractRow, MarketContractStatus } from '../../generated/dashboard';
import { MARKET_LEDGER_COLUMNS } from './definition.ts';

export interface MarketLedgerProjectionOptions {
  readonly perspective: string | null;
  readonly showRequests: boolean;
  readonly showEnded: boolean;
  readonly selectionEmpty: string | null;
}

const HEAD_LIMIT = 256;

const CONTRACT_STATUS_TEXT: Record<MarketContractStatus, { readonly label: string; readonly mark: 'running' | 'waiting' | 'ended' | 'finished'; readonly muted?: boolean }> = {
  active: { label: 'Contract started', mark: 'running' },
  requested: { label: 'Request posted', mark: 'waiting' },
  ended: { label: 'Contract ended', mark: 'ended', muted: true },
  notRecorded: { label: 'Not recorded', mark: 'ended', muted: true },
};

function formatUnits(raw: string): string {
  const n = BigInt(raw);
  return fixedText(n);
}

function formatCic(raw: string, sign: -1 | 0 | 1): string {
  const abs = BigInt(raw);
  const mag = fixedText(abs < 0n ? -abs : abs);
  if (sign < 0) return `\u2212${mag}`;
  if (sign > 0) return `+${mag}`;
  return mag;
}

function perspectiveSign(contract: MarketContractRow, perspective: string | null): -1 | 0 | 1 {
  if (!perspective) return 0;
  if (contract.buyer === perspective && contract.seller !== perspective) return -1;
  if (contract.seller === perspective && contract.buyer !== perspective) return 1;
  return 0;
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

function deliveredCell(items: readonly MarketContractItem[]): TableCell {
  if (!items.length) return { value: notApplicable('No deliveries yet') };
  let deliveredSum = 0n;
  let contractedSum = 0n;
  let allContracted = true;
  for (const item of items) {
    if (item.deliveryUnavailable !== null || item.deliveredRaw === null) {
      return { value: notRecorded(item.deliveryUnavailable ?? 'Delivery amount was not recorded') };
    }
    deliveredSum += BigInt(item.deliveredRaw);
    if (item.contractedRaw === null) allContracted = false;
    else contractedSum += BigInt(item.contractedRaw);
  }
  const deliveredText = fixedText(deliveredSum);
  if (allContracted && contractedSum > 0n) {
    return {
      value: knownValue(`${deliveredText} / ${fixedText(contractedSum)}`),
      unitSuffix: contractedSum === FIXED5 ? 'unit' : 'units',
    };
  }
  if (deliveredSum === 0n) return { value: notApplicable('No deliveries yet') };
  return {
    value: knownValue(deliveredText),
    unitSuffix: deliveredSum === FIXED5 ? 'unit' : 'units',
  };
}

function paidCell(contract: MarketContractRow, sign: -1 | 0 | 1): TableCell {
  if (contract.paidNotRecorded) return { value: notRecorded('Payment amount was not recorded') };
  if (contract.paidRaw === null) return { value: notApplicable('No payments yet') };
  return {
    value: knownValue(formatCic(contract.paidRaw, sign)),
    unitSuffix: 'CIC',
  };
}

function buildChildEventRow(event: MarketContractEvent, sign: -1 | 0 | 1): TableRow {
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
      cells: {
        date: baseDate,
        kind: blank,
        country: blank,
        item: { value: knownValue(label), ...(event.equipment ? { inlineSubtext: 'Delivery' } : {}) },
        delivered: qtyCell,
        paid: blank,
        status: logStatusCell(sign < 0 ? 'Bought' : sign > 0 ? 'Sold' : 'Delivered', 'finished'),
      },
    };
  }
  if (event.kind === 'deliveryUnavailable') {
    const label = event.equipment ?? 'Delivery';
    return {
      key: `ev:${event.key}`,
      name: label,
      rowHour: event.hour,
      muted: true,
      cells: {
        date: baseDate,
        kind: blank,
        country: blank,
        item: { value: knownValue(label), inlineSubtext: 'Delivery' },
        delivered: { value: notRecorded(event.reason ?? 'Delivery amount unavailable') },
        paid: blank,
        status: logStatusCell('Unavailable', 'ended', 'muted'),
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
      cells: {
        date: baseDate,
        kind: blank,
        country: blank,
        item: { value: knownValue('Payment') },
        delivered: blank,
        paid: cicCell,
        status: logStatusCell(sign < 0 ? 'Paid' : sign > 0 ? 'Received' : 'Payment', 'finished'),
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
    muted: event.kind === 'contractEnded',
    cells: {
      date: baseDate,
      kind: blank,
      country: blank,
      item: { value: knownValue(lifecycleLabel) },
      delivered: blank,
      paid: blank,
      status: logStatusCell(lifecycleLabel, mark, event.kind === 'contractEnded' ? 'muted' : undefined),
    },
  };
}

function buildContractRow(
  readView: BoundedReadView,
  contract: MarketContractRow,
  perspective: string | null,
  boundary: ReturnType<typeof newIdentityBoundary>,
): TableRow {
  const sign = perspectiveSign(contract, perspective);
  const summary = primaryItemSummary(contract.items);
  const artHandle = urlArtHandle(summary.iconUrl);
  const artResult = artHandle ? readView.art(artHandle) : null;
  const icon = artResult?.status === 'ready' ? artResult.url : undefined;
  const iconPending = artResult?.status === 'pending' ? true : undefined;

  const statusInfo = CONTRACT_STATUS_TEXT[contract.status];
  const children = contract.events.map(ev => buildChildEventRow(ev, sign));
  const title = summary.label ?? `Contract ${contract.key.slice(-6)}`;

  const tooltipExtra = (): TooltipExtraRow[] => {
    const out: TooltipExtraRow[] = [
      { label: 'Contract', value: knownValue(contract.key) },
      { label: 'Seller', value: contract.seller ? knownValue(contract.seller) : notRecorded(contract.sellerNotRecorded ? 'Seller not recorded' : 'Seller name unavailable') },
      { label: 'Buyer', value: contract.buyer ? knownValue(contract.buyer) : notRecorded(contract.buyerNotRecorded ? 'Buyer not recorded' : 'Buyer name unavailable') },
    ];
    if (contract.startedHour !== null) out.push({ label: 'Started', value: knownValue(formatDateTime(contract.startedHour)) });
    if (contract.endedHour !== null) out.push({ label: 'Ended', value: knownValue(formatDateTime(contract.endedHour)) });
    if (contract.agreedPriceRaw !== null) out.push({ label: 'Agreed price', value: knownValue(`${fixedText(BigInt(contract.agreedPriceRaw))} CIC`) });
    for (const item of contract.items) {
      const itemLabel = item.name ?? `Design ${item.variant}`;
      const progress = item.deliveryUnavailable
        ? `Not recorded (${item.deliveryUnavailable})`
        : item.contractedRaw !== null
          ? `${formatUnits(item.deliveredRaw ?? '0')} / ${formatUnits(item.contractedRaw)} units`
          : `${formatUnits(item.deliveredRaw ?? '0')} units`;
      out.push({ label: itemLabel, value: knownValue(progress) });
    }
    return out;
  };

  return {
    key: `contract:${contract.key}`,
    name: title,
    rowHour: contract.hour,
    muted: statusInfo.muted === true,
    cells: {
      date: logDateCell(contract.hour),
      kind: {
        value: summary.label ? knownValue(summary.label) : notRecorded(summary.missing ?? 'Equipment icon not recorded'),
        icon,
        iconPending,
        iconSlot: true,
        artShape: 'equipment',
        artName: title,
      },
      country: contractParties(readView, contract, perspective, boundary),
      item: summary.label !== null
        ? { value: knownValue(summary.label), ...(summary.subtext ? { inlineSubtext: summary.subtext } : {}) }
        : { value: notRecorded(summary.missing ?? 'Equipment design not recorded') },
      delivered: deliveredCell(contract.items),
      paid: paidCell(contract, sign),
      status: logStatusCell(statusInfo.label, statusInfo.mark, statusInfo.muted ? 'muted' : undefined),
    },
    ...(children.length > 0 ? { children } : {}),
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
    const source = readView.rows<MarketContractRow & DatedLogRow>('main');
    if (!source) {
      return { columns: MARKET_LEDGER_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true };
    }
    if (source.refused !== null) {
      return { columns: MARKET_LEDGER_COLUMNS, rows: [], hour, hideHeader: true, unsorted: true, dateGrouped: true, refused: source.refused };
    }
    const boundary = newIdentityBoundary();
    const { slots, rows, windowOffset, totalRows } = projectPagedSlots(
      source,
      hour,
      readView.rowWindow,
      contract => {
        if (!options.showRequests && contract.status === 'requested') return null;
        if (!options.showEnded && contract.status === 'ended') return null;
        return buildContractRow(readView, contract, options.perspective, boundary);
      },
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
  countries: { mode: 'multi', labels: true },
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
    const facets: BoundFacetFilter[] = [];
    if (tags) {
      facets.push({ name: 'country|receiver', values: [], countries: tags.map(countryFromKey) });
    }
    return {
      owner: `${ctx.track.source.id}:${ctx.branch.id}:market-ledger:${tags ? tags.join(',') : 'all'}`,
      entries: [
        pagedTailEntry(
          'main',
          ctx.track.source,
          ctx.branch.id,
          ctx.track.axisOffset,
          {
            product: 'market-ledger-tail',
            branchId: ctx.branch.id,
            facets,
            from: boundFrom,
            limit: HEAD_LIMIT,
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
    const perspective = tags && tags.length === 1 ? tags[0]! : null;
    const selectionEmpty = !all && (!tags || tags.length === 0)
      ? (ctx.widget.countries.kind === 'group' || ctx.widget.countries.kind === 'factions'
          ? 'This group has no members in this campaign.'
          : 'Choose a country in the widget settings.')
      : null;
    return {
      perspective,
      showRequests: ctx.options.showRequests !== false,
      showEnded: ctx.options.showEnded !== false,
      selectionEmpty,
    };
  },
};
