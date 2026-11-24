import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { DOMAINS, UNRESOLVED_MARK } from '../../dashboard/equipment.ts';
import type { AmountRun, EquipmentStockpileReply, StockVariant, TypeLogistics } from './types';
import type { StockpileAtReply } from '../../generated/dashboard';

const RAW = /^-?\d{1,20}$/;
const text = (value: unknown) => value === null || typeof value === 'string';

export function validateStockpileAt(data: unknown): StockpileAtReply {
  const reply = data as StockpileAtReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.tag === 'string' && typeof reply.campaignKey === 'string'
    && Number.isInteger(reply.decimals) && Array.isArray(reply.types)
    && reply.types.every(type => typeof type?.key === 'string' && DOMAINS.includes(type.domain) && text(type.name) && text(type.needNote))
    && Array.isArray(reply.variants) && reply.variants.every(variant => RAW.test(variant?.id) && typeof variant.token === 'string'
      && (variant.archetype === null || reply.types.some(type => type.key === variant.archetype)) && typeof variant.known === 'boolean'
      && text(variant.name) && text(variant.design) && (variant.origin === undefined || text(variant.origin)) && typeof variant.held === 'boolean' && Array.isArray(variant.lines)
      && variant.lines.every(line => RAW.test(line)) && text(variant.outputMissing))
    && new Set(reply.variants.map(variant => variant.id)).size === reply.variants.length
    && Array.isArray(reply.logistics) && reply.logistics.every(item => typeof item?.key === 'string'
      && (item.key.startsWith(UNRESOLVED_MARK) || reply.types.some(type => type.key === item.key)))
    && ['player', 'ai', 'unknown'].includes(reply.balance) && text(reply.stockMissing) && text(reply.linesMissing)
    && text(reply.balanceMissing) && RAW.test(reply.needBoundRaw) && Array.isArray(reply.warnings);
  if (!ok) throw new Error('The equipment stockpile reply is malformed.');
  return reply;
}

function cell(values: LiveValues<number>, row: string, column: string): bigint | null {
  const value = values.get(liveKey(row, column));
  return value === undefined || !Number.isFinite(value) ? null : BigInt(Math.round(value));
}

export function lineFigures(lines: readonly string[], values: LiveValues<number>): { daily: bigint | null; ic: bigint | null; low: bigint | null; high: bigint | null } {
  let daily: bigint | null = 0n, ic: bigint | null = 0n, low: bigint | null = null, high: bigint | null = null, known = true;
  for (const line of lines) {
    const speed = cell(values, `l${line}`, 'speed'), cost = cell(values, `l${line}`, 'cost');
    if (speed == null || cost == null) return { daily: null, ic: null, low: null, high: null };
    ic = ic! + speed;
    daily = daily != null && cost > 0n ? daily + speed * 100000n / cost : null;
    if (cost <= 0n) known = false;
    else {
      low = low == null || cost < low ? cost : low;
      high = high == null || cost > high ? cost : high;
    }
  }
  return known ? { daily, ic, low, high } : { daily, ic, low: null, high: null };
}

export function atReply(reply: StockpileAtReply, values: LiveValues<number>, hour: number, branchId: string): EquipmentStockpileReply {
  const span = [{ firstHour: hour, lastHour: hour }];
  const run = (raw: bigint | null): AmountRun[] => [{ firstHour: hour, lastHour: hour, amountRaw: raw == null ? null : String(raw) }];
  const stockKnown = reply.stockMissing == null, linesKnown = reply.linesMissing == null, player = reply.balance === 'player';
  const variants: StockVariant[] = reply.variants.map(variant => {
    const made = linesKnown && (variant.lines.length > 0 || variant.outputMissing != null);
    const figures = variant.outputMissing != null ? { daily: null, ic: null, low: null, high: null } : lineFigures(variant.lines, values);
    const line = (raw: bigint | null) => made ? run(raw) : [];
    return {
      id: variant.id, token: variant.token, archetype: variant.archetype,
      names: [{ firstHour: Number.MIN_SAFE_INTEGER, name: variant.name, known: variant.known, design: variant.design }],
      stock: stockKnown && variant.held ? run(cell(values, `v${variant.id}`, 'stock')) : [],
      daily: line(figures.daily), dailyIc: line(figures.ic), unitCostLow: line(figures.low), unitCostHigh: line(figures.high),
    };
  });
  const logistics: TypeLogistics[] = player ? reply.logistics.map(item => ({ key: item.key,
    balance: run(cell(values, `t${item.key}`, 'balance')), stock: run(cell(values, `t${item.key}`, 'stock')),
    available: run(cell(values, `t${item.key}`, 'available')) })) : [];
  return {
    branchId, tag: reply.tag, campaignKey: reply.campaignKey, decimals: reply.decimals, types: reply.types, variants,
    reads: stockKnown ? [hour] : [], stockHours: stockKnown ? span : [], lineHours: linesKnown ? span : [], lineReads: linesKnown ? [hour] : [],
    logistics, logisticsReads: player ? [hour] : [], balanceHours: player ? span : [], aiHours: reply.balance === 'ai' ? span : [],
    needBoundRaw: reply.needBoundRaw,
    warnings: [...reply.warnings, ...[reply.stockMissing, reply.linesMissing, reply.balanceMissing].filter((why): why is string => why != null)],
  };
}
