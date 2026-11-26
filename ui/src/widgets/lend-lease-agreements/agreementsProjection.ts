import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { FIXED5, fixedText, FUEL_QUANTITY, MARKET, SHARE, validateMarketReply } from '../../dashboard/market.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { occurrenceSelection } from '../../families/table/logHelpers.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import { artNameCell, identityFlags, identityName, newIdentityBoundary } from '../../families/table/identityCell.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, valueFromLive, zeroValue, type Value } from '../../families/value.ts';
import type { LeaseTerms, MarketItem, MarketLendLeaseReply } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export type Direction = 'sent' | 'received';

export interface AgreementsProjectionOptions {
  readonly direction: Direction;
}

const PRIORITY: Readonly<Record<string, number>> = { equipment: 0, delivered: 1, country: 2, monthly: 3, last: 4, since: 5 };
export const AGREEMENT_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  numeric: column.key === 'monthly' || column.key === 'delivered' || column.key === 'last' || column.key === 'since',
  text: column.key === 'equipment' || column.key === 'country',
  ...(column.key === 'country' ? { pinned: true, sortable: false, widest: ['MMMMMMMMMM'] } : {}),
  ...(column.key === 'equipment' ? { cellKind: 'item' as const, pinned: true } : {}),
  ...(column.key === 'monthly' || column.key === 'delivered' ? { barEligible: true, format: 'integer' as const } : {}),
  ...(column.key === 'last' || column.key === 'since' ? { cellKind: 'date' as const, widest: ['30 Sep'] } : {}),
}));


const whole = (raw: bigint, denominator: bigint): Value => {
  const units = raw / denominator;
  return units === 0n ? zeroValue(0n) : knownValue(units);
};

const dateCell = (hour: number | null, none: Value): TableCell => (hour === null ? { value: none } : { value: knownValue(hour) });

export function lastDelivery(terms: Pick<LeaseTerms, 'lastDelivery' | 'delivered'>): Value {
  if (terms.lastDelivery > 0) return knownValue(terms.lastDelivery);
  return terms.delivered.every(item => BigInt(item.amountRaw) === 0n)
    ? notApplicable('No delivery yet')
    : notRecorded('The last delivery date was not recorded, though deliveries were');
}

function amounts(list: readonly MarketItem[]): Map<string, bigint> {
  const out = new Map<string, bigint>();
  for (const item of list) out.set(item.variant, (out.get(item.variant) ?? 0n) + BigInt(item.amountRaw));
  return out;
}

function scheduleWords(variant: string, percentage: ReadonlyMap<string, bigint>, once: ReadonlyMap<string, bigint>): string {
  const share = percentage.get(variant);
  const single = once.get(variant);
  if (share !== undefined) return `Production share ${fixedText(share, SHARE)}%`;
  if (single !== undefined) return `Once ${fixedText(single, FIXED5)}`;
  return 'No monthly schedule';
}

interface AgreementMeta {
  readonly terms: LeaseTerms;
  readonly variant: string | null;
}

interface Lookups {
  readonly variants: ReadonlyMap<string, MarketLendLeaseReply['variants'][number]>;
  readonly images: ReadonlyMap<string, string>;
}

function agreementRows(
  readView: BoundedReadView,
  { variants, images }: Lookups,
  terms: LeaseTerms,
  live: LiveValues<number | bigint | string> | null,
  boundary: ReturnType<typeof newIdentityBoundary>,
): TableRow[] {
  const monthly = amounts(terms.monthly);
  const percentage = amounts(terms.percentage);
  const once = amounts(terms.once);
  const delivered = amounts(terms.delivered);
  const sunk = amounts(terms.sunk);
  const flags = identityFlags(readView, 'country', [terms.giver, terms.receiver].filter((tag): tag is string => tag !== null), { boundary });
  const sideName = (tag: string | null) => (tag === null ? 'Country not recorded' : identityName(readView, 'country', tag));
  const pair = `${sideName(terms.giver)} → ${sideName(terms.receiver)}`;
  const country: TableCell = { value: knownValue(pair), flags };
  const last: TableCell = { value: lastDelivery(terms) };
  const since = dateCell(terms.activeSince, notRecorded('Start date not recorded'));
  const unnamed: TooltipExtraRow[] = [
    ...(terms.giverMissing !== null ? [{ label: 'Giver', value: notRecorded(terms.giverMissing) }] : []),
    ...(terms.receiverMissing !== null ? [{ label: 'Receiver', value: notRecorded(terms.receiverMissing) }] : []),
  ];
  const extra = (lost: Value, schedule: string | null): readonly TooltipExtraRow[] => [
    ...unnamed,
    ...(schedule ? [{ label: 'Schedule', value: knownValue(schedule), divider: true }] : []),
    { label: 'Lost on the way', value: lost, format: 'integer' },
  ];
  const rows: TableRow[] = [];
  const keys = [...new Set([...monthly.keys(), ...percentage.keys(), ...once.keys(), ...delivered.keys(), ...sunk.keys()])];
  for (const variant of keys) {
    const named = variants.get(variant);
    const image = named?.icon ? images.get(named.icon) ?? null : null;
    const month = monthly.get(variant);
    const lost = whole(sunk.get(variant) ?? 0n, FIXED5);
    const schedule = month === undefined ? scheduleWords(variant, percentage, once) : null;
    const meta: AgreementMeta = { terms, variant };
    rows.push({
      key: `${terms.key}:${variant}`,
      name: named?.name ?? variant,
      cells: {
        country,
        equipment: artNameCell(readView, named?.name ?? null, urlArtHandle(image), named?.nameUnknown ?? 'The recording does not name this design at this hour', 'equipment'),
        monthly: month === undefined ? { value: notApplicable(schedule!) } : { value: whole(month, FIXED5), format: 'integer' },
        delivered: { value: whole(delivered.get(variant) ?? 0n, FIXED5), format: 'integer' },
        last,
        since,
      },
      tooltipTitle: pair,
      tooltipExtra: () => extra(lost, schedule),
      meta,
    });
  }
  const daily = BigInt(terms.fuelDailyRaw);
  const share = BigInt(terms.fuelPercentageRaw);
  if (daily !== 0n || share !== 0n) {
    const schedule = [daily !== 0n ? `Daily ${fixedText(daily, FUEL_QUANTITY)}` : '', share !== 0n ? `Production share ${fixedText(share, SHARE)}%` : ''].filter(Boolean).join(' · ');
    const sent = valueFromLive(live, liveKey(terms.key, 'fuelSent'));
    const lost = valueFromLive(live, liveKey(terms.key, 'fuelLost'));
    const scaled = (value: Value): Value => value.kind === 'known' ? whole(BigInt(Math.trunc(Number(value.value))), FUEL_QUANTITY) : value;
    const meta: AgreementMeta = { terms, variant: null };
    rows.push({
      key: `${terms.key}:fuel`,
      name: 'Fuel',
      cells: {
        country,
        equipment: { value: knownValue('Fuel') },
        monthly: { value: notApplicable(schedule) },
        delivered: { value: scaled(sent), format: 'integer' },
        last,
        since,
      },
      tooltipTitle: pair,
      tooltipExtra: () => extra(scaled(lost), schedule),
      meta,
    });
  }
  return rows;
}

export const agreementsProjection: TableProjection<AgreementsProjectionOptions> = registerFamilyProjection({
  kind: 'lend-lease-agreements',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: AGREEMENT_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: AGREEMENT_COLUMNS, rows: [], hour };
    const reply = validateMarketReply(raw);
    if (reply.refused !== null) {
      return { columns: AGREEMENT_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: reply.refused, emptyKind: 'unavailable', warnings: reply.warnings };
    }
    if (reply.agreements === null) {
      return { columns: AGREEMENT_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: 'Lend-lease terms are not in this recording at this date', emptyKind: 'unavailable', warnings: reply.warnings };
    }
    const live = readView.live('main') ?? null;
    const boundary = newIdentityBoundary();
    const rows: TableRow[] = [];
    const lookups: Lookups = { variants: new Map(reply.variants.map(variant => [variant.id, variant])), images: new Map(reply.icons.map(icon => [icon.key, icon.image])) };
    for (const terms of reply.agreements) rows.push(...agreementRows(readView, lookups, terms, live, boundary));
    const side = options.direction === 'sent' ? 'givers' : 'receivers';
    const unattributed = reply.unattributed;
    const warnings = reply.warnings;
    return {
      columns: AGREEMENT_COLUMNS,
      rows,
      hour,
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(rows.length === 0
        ? unattributed
          ? { empty: 'Not recorded', emptyDetail: `Some agreements' ${side} are not in this recording at this date`, emptyKind: 'unavailable' as const }
          : { empty: options.direction === 'sent' ? 'No lend-lease sent' : 'No lend-lease received', emptyKind: 'empty' as const }
        : {}),
      warnings,
    };
  },
});

export const agreementsDeclaration: TableWidgetDeclaration<AgreementsProjectionOptions> = {
  family: 'table',
  kind: 'lend-lease-agreements',
  label: 'Lend-lease agreements',
  subject: 'lend-lease agreements',
  emptyCampaign: 'Load a campaign to see lend-lease.',
  icon: 'link',
  countries: { mode: 'filter' },
  sources: ctx => {
    const selection = occurrenceSelection(ctx);
    const params = {
      branchId: ctx.branch.id,
      countries: selection?.countries ?? [],
      ...(selection?.group ? { group: selection.group } : {}),
      only: 'agreements',
      leaseSide: ctx.options.direction === 'received' ? 'received' : 'sent',
    };
    return {
      owner: `lend-lease-agreements:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'population', form: 'all' },
          product: MARKET,
          followQuery: { panel: MARKET, ...params },
          params,
          validate: validateMarketReply,
        },
      }],
    };
  },
  projection: agreementsProjection,
  columns: AGREEMENT_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: ctx => ({ direction: ctx.options.direction === 'received' ? 'received' : 'sent' }),
};
