import { formatDateTime } from '../../dashboard/calendar.ts';
import { LEDGER_EMPTY, ledgerFigure, ledgerTitle, type LedgerItem } from '../../dashboard/marketLedger.ts';
import type { FamilySources } from '../../families/contract.ts';
import { logLookAt, type LogLooksMap, type LogProjection } from '../../families/log/declaration.ts';
import type { LogColumnTrack, LogModel, LogRowSpec, LogTextPart, LogTooltipField } from '../../families/log/model.ts';
import { registerLogProjection } from '../../families/log/registry.ts';
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
import { knownValue, notApplicable, notRecorded, valueFromRawBigInt, type Value } from '../../families/value.ts';

export const MARKET_LEDGER_PROJECTION_KIND = 'market-ledger';

export interface MarketLedgerProjectionOptions {
  readonly looks: LogLooksMap;
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

function partyValue(looks: LogLooksMap, tag: string | null, hour: number): { readonly value: Value<string>; readonly look: ReturnType<typeof logLookAt>; readonly spoken: string } {
  if (!tag) return { value: notRecorded('Country not recorded'), look: null, spoken: 'Country not recorded' };
  const look = logLookAt(looks, tag, hour);
  const name = nonEmptyText(look?.name);
  if (name) return { value: knownValue(name), look, spoken: name };
  if (look?.status) return { value: notRecorded(look.status), look, spoken: look.status };
  return { value: knownValue(tag), look, spoken: tag };
}

function amountAuditValue(item: LedgerItem): Value {
  if (item.kind === null) return notRecorded('Event kind not recorded');
  if (item.kind === 34 || (item.amountRaw === null && item.reason !== null)) {
    return notRecorded(item.reason ?? 'Amount unavailable');
  }
  if (item.kind === 6 || item.kind === 10 || item.kind === 11 || item.kind === 12) {
    return notApplicable('No delivery amount for this event');
  }
  return valueFromRawBigInt(item.amountRaw, { kind: 'notRecorded', reason: 'Amount not recorded' });
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
      const item: LedgerItem = {
        id: nonEmptyText(cellText(cellAt(wire, index, 'id'))) ?? wire.key,
        kind: cellNumber(cellAt(wire, index, 'kind')),
        mode: cellNumber(cellAt(wire, index, 'mode')),
        contract: nonEmptyText(cellText(cellAt(wire, index, 'contract'))),
        seller: nonEmptyText(cellText(cellAt(wire, index, 'seller'))),
        buyer: nonEmptyText(cellText(cellAt(wire, index, 'buyer'))),
        variant: nonEmptyText(cellText(cellAt(wire, index, 'variant'))),
        equipment: nonEmptyText(cellText(cellAt(wire, index, 'equipment'))),
        equipmentMissing: nonEmptyText(cellText(cellAt(wire, index, 'equipmentMissing'))),
        amountRaw: cellRawNumber(cellAt(wire, index, 'amountRaw')),
        auxiliaryRaw: cellRawNumber(cellAt(wire, index, 'auxiliaryRaw')),
        totalRaw: cellRawNumber(cellAt(wire, index, 'totalRaw')),
        reason: nonEmptyText(cellText(cellAt(wire, index, 'reason'))),
      };

      const seller = partyValue(options.looks, item.seller, wire.hour);
      const buyer = partyValue(options.looks, item.buyer, wire.hour);
      const nameOf = (tag: string | null) => {
        if (tag === item.buyer) return buyer.spoken;
        if (tag === item.seller) return seller.spoken;
        return partyValue(options.looks, tag, wire.hour).spoken;
      };
      const titleText = ledgerTitle(item, nameOf);
      const titleVal: Value<string> = item.kind !== null
        ? knownValue(titleText)
        : notRecorded('Event kind not recorded');
      const kindVal: Value = item.kind !== null
        ? knownValue(item.kind)
        : notRecorded('Event kind not recorded');
      const contractVal: Value<string> = item.contract !== null
        ? knownValue(item.contract)
        : notRecorded('Contract not recorded');
      const equipmentVal: Value<string> = item.variant === null
        ? notRecorded('Equipment variant not recorded')
        : item.variant === '0'
          ? notApplicable('No equipment variant for this event')
          : item.equipment !== null
            ? knownValue(item.equipment)
            : notRecorded(item.equipmentMissing ?? 'Equipment name unavailable');

      const figure = ledgerFigure(item);
      const figureVal: Value<string> = item.kind === null
        ? notRecorded('Event kind not recorded')
        : item.kind === 34 || (item.amountRaw === null && item.reason !== null)
          ? notRecorded(item.reason ?? 'Amount unavailable')
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
        item.variant !== null && item.variant !== '0' ? (item.equipment ?? item.equipmentMissing ?? 'Equipment name unavailable') : null,
        figure?.text ?? null,
      ].filter(Boolean).join(' · ');

      const amountAudit = amountAuditValue(item);

      rows[i] = {
        key: wire.key,
        hour: wire.hour,
        ordinal: parseOrdinal(wire.ordinal),
        art: item.kind !== null && KIND_COLOR[item.kind]
          ? { color: KIND_COLOR[item.kind] }
          : { missingReason: 'Event kind not recorded' },
        slots: [
          { kind: 'text', text: titleVal, weight: 'bold' },
          { kind: 'value', value: figureVal, align: 'end', tone: figure?.negative ? 'negative' : 'default' },
        ],
        subline: { kind: 'rich', parts: sublineParts },
        auditCells: {
          kind: kindVal,
          contract: contractVal,
          seller: seller.value,
          buyer: buyer.value,
          equipment: equipmentVal,
          amount: amountAudit,
        },
        spoken,
        meta: item,
        tooltip: () => {
          const tipRows: LogTooltipField[] = [
            { label: 'Seller', value: seller.value },
            { label: 'Buyer', value: buyer.value },
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
            dateLine: formatDateTime(wire.hour),
            rows: tipRows,
            note: item.reason ?? undefined,
          };
        },
      };
    }

    return {
      hour,
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
