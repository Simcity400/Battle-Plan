import { formatDateTime } from '../../dashboard/calendar.ts';
import {
  LEDGER_EMPTY,
  ledgerFigure,
  ledgerPartyName,
  ledgerTitle,
  type AmountState,
  type LedgerItem,
  type PartyState,
} from '../../dashboard/marketLedger.ts';
import type { FamilySources } from '../../families/contract.ts';
import { logLookAt, type LogLooksMap, type LogProjection } from '../../families/log/declaration.ts';
import type { LogColumnTrack, LogModel, LogRowSpec, LogTextPart, LogTooltipField } from '../../families/log/model.ts';
import { registerLogProjection } from '../../families/log/registry.ts';
import { LogRowProjectionCache } from '../../families/log/rowCache.ts';
import {
  cellAt,
  cellNumber,
  cellRawNumber,
  cellText,
  columnIndex,
  isLogTailReply,
  nonEmptyText,
  parseOrdinal,
} from '../../families/log/tailReply.ts';
import { knownValue, notApplicable, notRecorded, partialValue, valueFromRawBigInt, type Value } from '../../families/value.ts';
import type { LogCell } from '../../generated/dashboard';

export const MARKET_LEDGER_PROJECTION_KIND = 'market-ledger';
const rowCache = new LogRowProjectionCache();

export interface MarketLedgerProjectionOptions {
  readonly looks: LogLooksMap;
  readonly fromHour?: number;
  readonly emptyMessage?: string;
}

const COLUMNS: readonly LogColumnTrack[] = [
  { id: 'date', width: 68 },
  { id: 'art', width: 24 },
  { id: 'body', flex: 1, minWidth: 160 },
  { id: 'figure', width: 120, align: 'end', priority: 2 },
];

const KIND_COLOR: Readonly<Record<number, string>> = {
  3: '#4c9aff',
  4: '#f87171',
  5: '#f59e0b',
  6: '#f87171',
  10: '#60a5fa',
  11: '#34d399',
  12: '#f87171',
  20: '#4c9aff',
  21: '#a78bfa',
  34: '#4c9aff',
};

function partyStateOf(cell: LogCell | undefined): PartyState {
  if (cell?.kind === 'text') return 'tag';
  if (cell?.kind === 'notRecorded') return 'notRecorded';
  return 'unavailable';
}

function amountStateOf(cell: LogCell | undefined): AmountState {
  if (cell?.kind === 'number') return 'number';
  if (cell?.kind === 'notRecorded') return 'notRecorded';
  return 'missing';
}

function partyValue(
  looks: LogLooksMap,
  tag: string | null,
  state: PartyState,
  hour: number,
): { readonly value: Value<string>; readonly tagValue: Value<string>; readonly look: ReturnType<typeof logLookAt>; readonly spoken: string } {
  if (!tag) {
    const label = ledgerPartyName(null, state, t => t);
    return {
      value: notRecorded(label),
      tagValue: notRecorded(label),
      look: null,
      spoken: label,
    };
  }
  const look = logLookAt(looks, tag, hour);
  const name = nonEmptyText(look?.name);
  if (name) {
    return { value: knownValue(name), tagValue: knownValue(tag), look, spoken: name };
  }
  const reason = look?.status ?? 'Country display name not recorded';
  return {
    value: notRecorded(reason),
    tagValue: knownValue(tag),
    look,
    spoken: `${reason} (${tag})`,
  };
}

function amountAuditValue(item: LedgerItem): Value {
  if (item.kind === null) return notRecorded('Market event kind not recorded');
  if (item.kind === 34 || (item.amountRaw === null && item.reason !== null)) {
    return notRecorded(item.reason ?? 'Amount unavailable');
  }
  if (item.kind === 6 || item.kind === 10 || item.kind === 11 || item.kind === 12) {
    return notApplicable('No delivery amount for this event');
  }
  return valueFromRawBigInt(
    item.amountRaw,
    { kind: 'notRecorded', reason: item.amountState === 'notRecorded' ? 'Amount not recorded' : 'Amount unavailable' },
  );
}

export const marketLedgerProjection: LogProjection<MarketLedgerProjectionOptions> = {
  family: 'log',
  kind: MARKET_LEDGER_PROJECTION_KIND,
  layout: 'row',
  project(sources: FamilySources, hour: number, options: MarketLedgerProjectionOptions): LogModel {
    const rawTail = sources.panels.get('tail');
    const offsetRaw = sources.panels.get('__offset');
    const offset = typeof offsetRaw === 'number' && Number.isFinite(offsetRaw) ? offsetRaw : 0;
    const emptyMessage = options.emptyMessage ?? LEDGER_EMPTY;

    if (!isLogTailReply(rawTail)) {
      return {
        hour,
        ...(options.fromHour !== undefined ? { fromHour: options.fromHour } : {}),
        total: 0,
        baseOrdinal: 0,
        firstOrdinal: 0,
        offset: 0,
        layout: 'row',
        twoLine: true,
        columns: COLUMNS,
        rows: [],
        gaps: [],
        empty: { kind: 'empty', message: emptyMessage },
      };
    }

    const index = columnIndex(rawTail.columns);
    const total = parseOrdinal(rawTail.total);
    const baseOrdinal = parseOrdinal(rawTail.baseOrdinal);
    const firstOrdinal = parseOrdinal(rawTail.firstOrdinal);

    const rows: LogRowSpec[] = new Array(rawTail.rows.length);
    for (let i = 0; i < rawTail.rows.length; i++) {
      const wire = rawTail.rows[i]!;
      rows[i] = rowCache.getOrProject(options.looks, wire, () => {
        const sellerCell = cellAt(wire, index, 'seller');
        const buyerCell = cellAt(wire, index, 'buyer');
        const amountCell = cellAt(wire, index, 'amountRaw');
        const auxCell = cellAt(wire, index, 'auxiliaryRaw');
        const item: LedgerItem = {
          id: nonEmptyText(cellText(cellAt(wire, index, 'id'))) ?? wire.key,
          kind: cellNumber(cellAt(wire, index, 'kind')),
          mode: cellNumber(cellAt(wire, index, 'mode')),
          contract: nonEmptyText(cellText(cellAt(wire, index, 'contract'))),
          seller: nonEmptyText(cellText(sellerCell)),
          sellerState: partyStateOf(sellerCell),
          buyer: nonEmptyText(cellText(buyerCell)),
          buyerState: partyStateOf(buyerCell),
          variant: nonEmptyText(cellText(cellAt(wire, index, 'variant'))),
          equipment: nonEmptyText(cellText(cellAt(wire, index, 'equipment'))),
          equipmentMissing: nonEmptyText(cellText(cellAt(wire, index, 'equipmentMissing'))),
          amountRaw: cellRawNumber(amountCell),
          amountState: amountStateOf(amountCell),
          auxiliaryRaw: cellRawNumber(auxCell),
          auxiliaryState: amountStateOf(auxCell),
          totalRaw: cellRawNumber(cellAt(wire, index, 'totalRaw')),
          reason: nonEmptyText(cellText(cellAt(wire, index, 'reason'))),
        };

        const seller = partyValue(options.looks, item.seller, item.sellerState, wire.hour);
        const buyer = partyValue(options.looks, item.buyer, item.buyerState, wire.hour);
        const nameOf = (tag: string | null, state: PartyState = 'notRecorded') => {
          if (tag === item.buyer && state === item.buyerState) return buyer.spoken;
          if (tag === item.seller && state === item.sellerState) return seller.spoken;
          return partyValue(options.looks, tag, state, wire.hour).spoken;
        };
        const titleText = ledgerTitle(item, nameOf);
        const titleVal: Value<string> = item.kind === null
          ? notRecorded('Market event kind not recorded')
          : item.kind === 34 && item.mode === null
            ? partialValue('Delivery amount unavailable', 'Delivery mode not recorded')
            : knownValue(titleText);
        const kindVal: Value = item.kind !== null
          ? knownValue(item.kind)
          : notRecorded('Market event kind not recorded');
        const modeVal: Value = item.kind === 34
          ? (item.mode !== null ? knownValue(item.mode) : notRecorded('Delivery mode not recorded'))
          : (item.mode !== null ? knownValue(item.mode) : notApplicable('Mode applies to unavailable deliveries'));
        const contractVal: Value<string> = item.contract !== null
          ? knownValue(item.contract)
          : notRecorded('Contract not recorded');
        const equipmentVal: Value<string> = item.variant === null
          ? notRecorded(item.equipmentMissing ?? 'Equipment design not recorded')
          : item.variant === '0'
            ? notApplicable('No equipment variant for this event')
            : item.equipment !== null
              ? knownValue(item.equipment)
              : notRecorded(item.equipmentMissing ?? 'Equipment name unavailable');

        const figure = ledgerFigure(item);
        const figureVal: Value<string> = item.kind === null
          ? notRecorded('Market event kind not recorded')
          : item.kind === 34 || (item.amountRaw === null && item.reason !== null)
            ? notRecorded(item.reason ?? 'Amount unavailable')
            : item.amountRaw === null && (item.kind === 3 || item.kind === 4 || item.kind === 5 || item.kind === 20 || item.kind === 21)
              ? notRecorded(item.amountState === 'notRecorded' ? 'Amount not recorded' : 'Amount unavailable')
              : figure !== null
                ? knownValue(figure.text)
                : item.kind === 6 || item.kind === 10 || item.kind === 11 || item.kind === 12
                  ? notApplicable('No delivery amount for this event')
                  : notRecorded('Amount not recorded');

        const sublineParts: LogTextPart[] = [
          { text: seller.value, flag: item.seller ? seller.look : undefined, tone: 'muted' },
          { text: knownValue(' → '), tone: 'muted' },
          { text: buyer.value, tone: 'muted' },
        ];
        if (item.variant !== '0') {
          sublineParts.push(
            { text: knownValue(' · '), tone: 'muted' },
            { text: equipmentVal, tone: 'muted' },
          );
        }

        const spoken = [
          formatDateTime(wire.hour),
          titleText,
          `${seller.spoken} → ${buyer.spoken}`,
          item.variant !== '0' ? (item.equipment ?? item.equipmentMissing ?? 'Equipment design not recorded') : null,
          figure?.text ?? null,
        ].filter(Boolean).join(' · ');

        const amountAudit = amountAuditValue(item);
        const auxiliaryAudit: Value = item.kind === 5
          ? valueFromRawBigInt(
            item.auxiliaryRaw,
            { kind: 'notRecorded', reason: item.auxiliaryState === 'notRecorded' ? 'Lost fuel not recorded' : 'Lost fuel unavailable' },
          )
          : notApplicable('No auxiliary amount for this event');

        return {
          key: wire.key,
          hour: wire.hour,
          ordinal: parseOrdinal(wire.ordinal),
          art: item.kind !== null && KIND_COLOR[item.kind]
            ? { color: KIND_COLOR[item.kind] }
            : { missingReason: 'Market event kind not recorded' },
          slots: [
            { kind: 'text', text: titleVal, weight: 'bold' },
            { kind: 'value', value: figureVal, align: 'end', tone: figure?.negative ? 'negative' : 'default' },
          ],
          subline: { kind: 'rich', parts: sublineParts },
          rawMagnitudes: {
            kind: kindVal,
            mode: modeVal,
            contract: contractVal,
            sellerTag: seller.tagValue,
            buyerTag: buyer.tagValue,
            amount: amountAudit,
            auxiliary: auxiliaryAudit,
          },
          spoken,
          meta: item,
          tooltip: () => {
            const tipRows: LogTooltipField[] = [
              { label: 'Seller', value: seller.value },
              ...(item.seller && seller.value.kind !== 'known' ? [{ label: 'Seller tag', value: knownValue(item.seller), tone: 'muted' as const }] : []),
              { label: 'Buyer', value: buyer.value },
              ...(item.buyer && buyer.value.kind !== 'known' ? [{ label: 'Buyer tag', value: knownValue(item.buyer), tone: 'muted' as const }] : []),
            ];
            if (item.variant !== '0') {
              tipRows.push({ label: 'Equipment', value: equipmentVal });
            }
            if (figureVal.kind !== 'notApplicable') {
              tipRows.push({ label: 'Amount', value: figureVal, tone: figure?.negative ? 'negative' : undefined });
            }
            tipRows.push({ label: 'Contract', value: contractVal, tone: 'muted' });
            return {
              title: titleText,
              rows: tipRows,
              note: item.reason ?? undefined,
            };
          },
        };
      });
    }

    return {
      hour,
      ...(options.fromHour !== undefined ? { fromHour: options.fromHour } : {}),
      total,
      baseOrdinal,
      firstOrdinal,
      offset,
      layout: 'row',
      twoLine: true,
      columns: COLUMNS,
      rows,
      gaps: rawTail.missing,
      empty: { kind: 'empty', message: emptyMessage },
      warnings: rawTail.warnings,
    };
  },
};

registerLogProjection(marketLedgerProjection);
