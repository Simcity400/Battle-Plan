import { contentTitle } from '../../dashboard/contentTitles.ts';
import { runAt } from '../../charts/series.ts';
import type { Run } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatAmount, formatRate, formatSigned } from '../../charts/valueAxis.ts';
import { gbNumber, gbTwoSignificant } from '../../charts/numberFormat.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import type { ChoiceSource } from '../../dashboard/chosen';
import { setToken } from '../../dashboard/options.ts';
import { DOMAINS, UNRESOLVED_ARM, UNRESOLVED_MARK, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import type { AmountRun, Domain, EquipmentStockpileReply, HourSpan, StockpileCatalogueReply, StockVariant, StockVariantName } from './types';
import { chartSummaryFrom } from '../../charts/chartSummary.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import type { PreparedStockpileReply } from './stockpileCatalogue';
import type { StockType } from '../../generated/dashboard';


const RAW = /^-?\d{1,20}$/;
const PEAK = /^\d{1,39}$/;
const HOUR = (value: unknown) => Number.isInteger(value);

const ordered = <T extends { firstHour: number; lastHour: number }>(runs: readonly T[]) =>
  runs.every((run, index) => run.firstHour <= run.lastHour && (index === 0 || runs[index - 1].lastHour < run.firstHour));
const amounts = (runs: readonly AmountRun[]) => Array.isArray(runs) && ordered(runs)
  && runs.every(run => HOUR(run.firstHour) && HOUR(run.lastHour) && (run.amountRaw === null || RAW.test(run.amountRaw)));
const ascending = (hours: unknown) => Array.isArray(hours) && hours.every((hour, index) => HOUR(hour) && (index === 0 || hours[index - 1] < hour));
const typesValid = (types: readonly StockType[]) => Array.isArray(types) && types.every(type => typeof type.key === 'string' && type.key.length > 0
  && (type.name === null || typeof type.name === 'string') && DOMAINS.includes(type.domain)
  && (type.needNote === null || typeof type.needNote === 'string'))
  && new Set(types.map(type => type.key)).size === types.length;
const namesValid = (names: readonly StockVariantName[]) => Array.isArray(names) && names.every((name, index) => HOUR(name.firstHour)
  && (name.name === null || typeof name.name === 'string') && typeof name.known === 'boolean'
  && (name.design === null || (typeof name.design === 'string' && name.design.length > 0))
  && (index === 0 || names[index - 1].firstHour < name.firstHour));
const spansValid = (spans: unknown) => Array.isArray(spans) && ordered(spans);
const strings = (values: unknown) => Array.isArray(values) && values.every(value => typeof value === 'string');

export function validateStockpileReply(data: unknown): EquipmentStockpileReply {
  const reply = data as EquipmentStockpileReply;
  if (typeof reply === 'object' && reply != null && Array.isArray(reply.variants)
    && reply.variants.some(variant => variant != null && (variant as Partial<StockVariant>).daily === undefined)) {
    for (const variant of reply.variants) if (variant != null) Object.assign(variant, { daily: [], dailyIc: [], unitCostLow: [], unitCostHigh: [] });
    reply.lineHours = [];
  }
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string' && typeof reply.campaignKey === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && typesValid(reply.types)
    && Array.isArray(reply.variants) && reply.variants.every(variant => RAW.test(variant.id) && typeof variant.token === 'string'
      && (variant.archetype === null || reply.types.some(type => type.key === variant.archetype))
      && namesValid(variant.names)
      && amounts(variant.stock) && amounts(variant.daily) && amounts(variant.dailyIc)
      && amounts(variant.unitCostLow) && amounts(variant.unitCostHigh))
    && new Set(reply.variants.map(variant => variant.id)).size === reply.variants.length
    && ascending(reply.reads)
    && Array.isArray(reply.stockHours) && ordered(reply.stockHours) && Array.isArray(reply.lineHours) && ordered(reply.lineHours)
    && Array.isArray(reply.logistics) && reply.logistics.every(item => typeof item.key === 'string'
      && (item.key.startsWith(UNRESOLVED_MARK) || reply.types.some(type => type.key === item.key)) && amounts(item.balance) && amounts(item.stock) && amounts(item.available))
    && new Set(reply.logistics.map(item => item.key)).size === reply.logistics.length
    && ascending(reply.logisticsReads) && ascending(reply.lineReads) && Array.isArray(reply.balanceHours) && ordered(reply.balanceHours)
    && Array.isArray(reply.aiHours) && ordered(reply.aiHours) && RAW.test(reply.needBoundRaw)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The equipment stockpile reply is malformed.');
  return reply;
}

export function validateStockpileCatalogue(data: unknown): StockpileCatalogueReply {
  const reply = data as StockpileCatalogueReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && Array.isArray(reply.countries) && reply.countries.every((country: unknown) => typeof country === 'string' || typeof country === 'number')
    && typeof reply.campaignKey === 'string' && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && typesValid(reply.types)
    && Array.isArray(reply.variants) && reply.variants.every(variant => RAW.test(variant.id) && typeof variant.token === 'string'
      && (variant.archetype === null || reply.types.some(type => type.key === variant.archetype))
      && namesValid(variant.names) && strings(variant.designs))
    && new Set(reply.variants.map(variant => variant.id)).size === reply.variants.length
    && Array.isArray(reply.logistics) && reply.logistics.every(item => typeof item.key === 'string' && typeof item.nonzero === 'boolean'
      && (item.key.startsWith(UNRESOLVED_MARK) || reply.types.some(type => type.key === item.key)))
    && new Set(reply.logistics.map(item => item.key)).size === reply.logistics.length
    && ascending(reply.reads) && ascending(reply.lineReads) && ascending(reply.logisticsReads)
    && spansValid(reply.stockHours) && spansValid(reply.lineHours) && spansValid(reply.balanceHours) && spansValid(reply.aiHours)
    && RAW.test(reply.needBoundRaw) && typeof reply.choices === 'object' && reply.choices != null
    && Object.values(reply.choices).every(choice => !!choice && PEAK.test(choice.raw) && typeof choice.stockDrawable === 'boolean' && typeof choice.shareDrawable === 'boolean')
    && strings(reply.warnings);
  if (!ok) throw new Error('The equipment stockpile catalogue is malformed.');
  return reply;
}

export function validateStockpileChart(data: unknown, count: number): ChartSummary[] {
  const reply = data as { lines?: unknown; warnings?: unknown; approximate?: unknown };
  const lines = reply && typeof reply === 'object' && Array.isArray(reply.lines) && strings(reply.warnings) ? reply.lines : null;
  const approximate = reply?.approximate ?? [];
  if (!lines || lines.length !== count || !spansValid(approximate)) throw new Error('The equipment stockpile chart is malformed.');
  return lines.map(line => {
    const summary = chartSummaryFrom((line as { summary?: unknown } | null)?.summary);
    if (!summary) throw new Error('The equipment stockpile chart is malformed.');
    return (approximate as HourSpan[]).length ? { ...summary, approximate: approximate as HourSpan[] } : summary;
  });
}

interface ReplyIndex {
  types: Map<string, EquipmentStockpileReply['types'][number]>;
  logistics: Map<string, EquipmentStockpileReply['logistics'][number]>;
  untypedVariants: StockVariant[];
  unresolvedLogistics: EquipmentStockpileReply['logistics'];
}
const indexes = new WeakMap<EquipmentStockpileReply, ReplyIndex>();
function indexOf(reply: EquipmentStockpileReply): ReplyIndex {
  let index = indexes.get(reply);
  if (!index) {
    index = {
      types: new Map(reply.types.map(type => [type.key, type])),
      logistics: new Map(reply.logistics.map(item => [item.key, item])),
      untypedVariants: reply.variants.filter(variant => variant.archetype == null),
      unresolvedLogistics: reply.logistics.filter(item => item.key.startsWith(UNRESOLVED_MARK)),
    };
    indexes.set(reply, index);
  }
  return index;
}

export type Grouping = 'type' | 'variant' | 'domain';

export interface StockGroup {
  key: string;
  label: string;
  domain: Domain | null;
  variants: readonly StockVariant[];
  types: readonly string[];
  kind: Grouping;
  scope: readonly string[];
  designs?: readonly string[];
}

export const labelAt = (group: StockGroup, grouping: Grouping, hour: number) =>
  grouping === 'variant' && group.variants[0] ? variantName(group.variants[0], hour) : group.label;

export const hasRecords = (reply: EquipmentStockpileReply) => {
  const prepared = (reply as PreparedStockpileReply).prepared;
  return reply.reads.length > 0 || reply.lineReads.length > 0 || reply.logisticsReads.length > 0 || reply.aiHours.length > 0
    || !!prepared && (prepared.stockRecorded || prepared.linesRecorded || prepared.balanceRecorded || prepared.aiRecorded);
};

export const variantName = (variant: StockVariant, hour: number) =>
    variantNameAt(variant.names, hour, `Variant ${variant.id}`);

export const variantKey = (reply: Pick<EquipmentStockpileReply, 'campaignKey'>, variant: StockVariant) =>
  campaignToken(reply.campaignKey, `${variant.id}:${variant.token}`);

const typeKey = (variant: StockVariant) => variant.archetype ?? `${UNRESOLVED_MARK}${variant.token || variant.id}`;

export function typeName(reply: EquipmentStockpileReply, key: string): string {
  const type = indexOf(reply).types.get(key);
  if (type) return contentTitle(type.name, type.key, 'Equipment type name unavailable');
  return unresolvedTypeName(key);
}

export function stockGroups(reply: EquipmentStockpileReply, grouping: Grouping, hour: number, included: readonly string[] = []): StockGroup[] {
  const place = new Map(reply.types.map((type, index) => [type.key, index]));
  const typeIndex = indexOf(reply).types;
  const domainOf = (variant: StockVariant) => (variant.archetype != null ? typeIndex.get(variant.archetype)?.domain : null) ?? null;
  const shown = reply.variants.filter(variant => !included.length || included.includes(typeKey(variant)));
  const rank = (key: string) => place.get(key) ?? reply.types.length;
  if (grouping === 'variant') {
    return [...shown].sort((a, b) => rank(typeKey(a)) - rank(typeKey(b)) || typeKey(a).localeCompare(typeKey(b))
      || (BigInt(a.id) < BigInt(b.id) ? -1 : BigInt(a.id) > BigInt(b.id) ? 1 : 0))
      .map(variant => ({ key: variantKey(reply, variant), label: variantName(variant, hour), domain: domainOf(variant), variants: [variant], types: [], kind: grouping, scope: included,
        designs: (reply as PreparedStockpileReply).prepared?.designs[variant.id] }));
  }
  const groups = new Map<string, StockGroup & { variants: StockVariant[]; types: string[] }>();
  const groupOf = (type: string, domain: Domain | null) => {
    const key = grouping === 'domain' ? domain ?? UNRESOLVED_ARM : type;
    const label = grouping === 'domain' ? domainName(key) : typeName(reply, key);
    const group = groups.get(key) ?? { key, label, domain, variants: [], types: [], kind: grouping, scope: included };
    groups.set(key, group);
    return group;
  };
  for (const variant of shown) groupOf(typeKey(variant), domainOf(variant)).variants.push(variant);
  const listed = reply.logistics.filter(item => !included.length || included.includes(item.key)).map(item => {
    const domain = typeIndex.get(item.key)?.domain ?? null;
    return { item, domain, key: grouping === 'domain' ? domain ?? UNRESOLVED_ARM : item.key,
      nonzero: (reply as PreparedStockpileReply).prepared?.nonzeroLogistics.includes(item.key)
        ?? [...item.balance, ...item.stock, ...item.available].some(run => run.amountRaw !== '0') };
  });
  for (const entry of listed) if (entry.nonzero) groupOf(entry.item.key, entry.domain);
  for (const entry of listed) {
    const group = groups.get(entry.key);
    if (group && !group.types.includes(entry.item.key)) group.types.push(entry.item.key);
  }
  const order = (key: string) => grouping === 'domain' ? (DOMAINS.indexOf(key as Domain) + 4) % 4 : rank(key);
  return [...groups.values()].sort((a, b) => order(a.key) - order(b.key) || a.key.localeCompare(b.key));
}

function sumAt(spans: EquipmentStockpileReply['stockHours'], variants: readonly StockVariant[], pick: (variant: StockVariant) => readonly AmountRun[], hour: number): bigint | null {
  if (!runAt(spans, hour)) return null;
  let total = 0n;
  for (const variant of variants) {
    const run = runAt(pick(variant), hour);
    if (!run) continue;
    if (run.amountRaw === null) return null;
    total += BigInt(run.amountRaw);
  }
  return total;
}

type Summed = Pick<StockGroup, 'variants'> & Partial<Pick<StockGroup, 'types' | 'kind'>>;

export function variantsComplete(reply: EquipmentStockpileReply, group: Summed, pick: (variant: StockVariant) => readonly AmountRun[], hour: number): boolean {
  if (group.kind == null || group.kind === 'variant') return true;
  if ((group.types ?? []).some(key => key.startsWith(UNRESOLVED_MARK))) return false;
  return !indexOf(reply).untypedVariants.some(variant => {
    const run = runAt(pick(variant), hour);
    return run != null && run.amountRaw !== '0';
  });
}

const stockOf = (variant: StockVariant) => variant.stock;
const dailyOf = (variant: StockVariant) => variant.daily;
const dailyIcOf = (variant: StockVariant) => variant.dailyIc;

export const stockAt = (reply: EquipmentStockpileReply, group: Summed, hour: number) =>
  variantsComplete(reply, group, stockOf, hour) ? sumAt(reply.stockHours, group.variants, stockOf, hour) : null;

export const dailyAt = (reply: EquipmentStockpileReply, group: Summed, hour: number) =>
  variantsComplete(reply, group, dailyOf, hour) ? sumAt(reply.lineHours, group.variants, dailyOf, hour) : null;

export const dailyIcAt = (reply: EquipmentStockpileReply, group: Summed, hour: number) =>
  variantsComplete(reply, group, dailyIcOf, hour) ? sumAt(reply.lineHours, group.variants, dailyIcOf, hour) : null;

export type BalanceMode = 'player' | 'ai' | 'unknown';

export function balanceMode(reply: EquipmentStockpileReply, hour: number): BalanceMode {
  if (runAt(reply.balanceHours, hour)) return 'player';
  if (runAt(reply.aiHours, hour)) return 'ai';
  return 'unknown';
}

export const UNATTRIBUTED_NOTE = 'Total unavailable.';

export const AI_NOTE = 'The game doesn’t compute surplus or shortage for AI-controlled countries.';
const NEED_UNAVAILABLE = 'Need unavailable.';

export interface GroupBalance { balance: bigint; stock: bigint; need: bigint | null; needNote: string | null }

const decidesCompleteness = (group: Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>, key: string) =>
  group.kind === 'domain' && group.key !== UNRESOLVED_ARM && key.startsWith(UNRESOLVED_MARK) && (!group.scope?.length || group.scope.includes(key));

function unresolvedIn(reply: EquipmentStockpileReply, group: Partial<Pick<StockGroup, 'scope'>>): EquipmentStockpileReply['logistics'] {
  const unresolved = indexOf(reply).unresolvedLogistics;
  return group.scope?.length ? unresolved.filter(item => group.scope!.includes(item.key)) : unresolved;
}

export function typesComplete(reply: EquipmentStockpileReply, group: Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>, hour: number): boolean {
  if (group.kind !== 'domain' || group.key === UNRESOLVED_ARM) return true;
  return !unresolvedIn(reply, group).some(item => [item.balance, item.stock].some(runs => {
    const run = runAt(runs, hour);
    return run != null && run.amountRaw !== '0';
  }));
}

export function balanceOperandsAt(reply: EquipmentStockpileReply, group: Pick<StockGroup, 'types'> & Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>, hour: number): (GroupBalance & { available: bigint | null }) | null {
  if (!group.types.length || balanceMode(reply, hour) !== 'player') return null;
  if (!typesComplete(reply, group, hour)) return null;
  const bound = BigInt(reply.needBoundRaw);
  const index = indexOf(reply);
  let [balance, stock, need] = [0n, 0n, 0n as bigint | null];
  let availableTotal: bigint | null = 0n;
  let needNote: string | null = null;
  for (const key of group.types) {
    const item = index.logistics.get(key);
    const value = (runs: readonly AmountRun[] | undefined) => {
      const run = runs && runAt(runs, hour);
      return !run ? 0n : run.amountRaw === null ? null : BigInt(run.amountRaw);
    };
    const [typeBalance, typeStock] = [value(item?.balance), value(item?.stock)];
    if (typeBalance == null || typeStock == null) return null;
    balance += typeBalance;
    stock += typeStock;
    const type = index.types.get(key);
    const observed = item && runAt(item.available, hour);
    const available = observed?.amountRaw != null ? BigInt(observed.amountRaw) : null;
    const note = available != null ? null : type ? type.needNote : NEED_UNAVAILABLE;
    const nativeAvailable = available ?? (typeStock > 0n ? typeStock : 0n);
    if (note) availableTotal = null;
    else if (availableTotal != null) availableTotal += nativeAvailable;
    const typeNeed = BigInt.asIntN(64, nativeAvailable - typeBalance);
    const reason = note ?? ((typeNeed < 0n ? -typeNeed : typeNeed) >= bound ? NEED_UNAVAILABLE : null);
    if (reason) { need = null; needNote ??= reason; } else if (need != null) need += typeNeed;
  }
  if (group.kind === 'domain' && group.key !== UNRESOLVED_ARM
    && unresolvedIn(reply, group).some(item => runAt(item.balance, hour) != null || runAt(item.stock, hour) != null)) {
    need = null;
    availableTotal = null;
    needNote ??= NEED_UNAVAILABLE;
  }
  return { balance, stock, need, needNote, available: availableTotal };
}

export function balanceAt(reply: EquipmentStockpileReply, group: Pick<StockGroup, 'types'> & Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>, hour: number): GroupBalance | null {
  const value = balanceOperandsAt(reply, group, hour);
  if (!value) return null;
  const { balance, stock, need, needNote } = value;
  return { balance, stock, need, needNote };
}

export function shareOf(value: Pick<GroupBalance, 'balance' | 'need'> | null): number | null {
  if (!value || value.need == null || value.need <= 0n) return null;
  return Number(value.balance) / Number(value.need);
}

export function shareRuns(reply: EquipmentStockpileReply, group: Pick<StockGroup, 'types'> & Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>): Run[] {
  const runs: Run[] = [];
  if (!group.types.length) return runs;
  const hours = new Set<number>(reply.balanceHours.map(span => span.firstHour));
  for (const item of reply.logistics) {
    if (!group.types.includes(item.key) && !decidesCompleteness(group, item.key)) continue;
    for (const run of [...item.balance, ...item.stock, ...item.available]) { hours.add(run.firstHour); hours.add(run.lastHour + 1); }
  }
  const changes = [...hours].sort((a, b) => a - b);
  for (const span of reply.balanceHours) {
    const inside = changes.filter(hour => hour >= span.firstHour && hour <= span.lastHour);
    let broken = true;
    inside.forEach((first, index) => {
      const last = index + 1 < inside.length ? inside[index + 1] - 1 : span.lastHour;
      const share = shareOf(balanceAt(reply, group, first));
      if (share == null) { broken = true; return; }
      const value = share * 100;
      const previous = runs.at(-1);
      if (previous && !broken && previous.value === value) previous.lastHour = last;
      else runs.push({ firstHour: first, lastHour: last, value, breakBefore: broken });
      broken = false;
    });
  }
  return runs;
}

function changeHours(reply: EquipmentStockpileReply, variants: readonly StockVariant[]): number[] {
  const hours = new Set<number>(reply.stockHours.map(span => span.firstHour));
  for (const variant of [...variants, ...indexOf(reply).untypedVariants]) {
    for (const run of variant.stock) { hours.add(run.firstHour); hours.add(run.lastHour + 1); }
  }
  return [...hours].sort((a, b) => a - b);
}

export function stockRuns(reply: EquipmentStockpileReply, group: Summed): Run[] {
  const runs: Run[] = [];
  const changes = changeHours(reply, group.variants);
  for (const span of reply.stockHours) {
    const inside = changes.filter(hour => hour >= span.firstHour && hour <= span.lastHour);
    inside.forEach((first, index) => {
      const last = index + 1 < inside.length ? inside[index + 1] - 1 : span.lastHour;
      const stock = stockAt(reply, group, first);
      if (stock == null) return;
      const value = fixedNumber(stock, reply.decimals);
      const previous = runs.at(-1);
      const joined = previous != null && index > 0 && previous.lastHour + 1 === first;
      if (joined && previous.value === value) previous.lastHour = last;
      else runs.push({ firstHour: first, lastHour: last, value, breakBefore: !joined });
    });
  }
  return runs;
}

export const groupChoices = (groups: readonly StockGroup[], noun: string): ChoiceSource => ({
  lines: key => groups.some(group => group.key === key) ? 1 : 0,
  name: key => groups.find(group => group.key === key)?.label ?? 'Equipment group name unavailable',
  noun,
});

const DESIGN_MARK = '#';
const rowOf = (key: string) => {
  const at = key.lastIndexOf(DESIGN_MARK);
  return at < 0 ? key : key.slice(0, at);
};

export function designAt(variant: StockVariant, hour: number): string | null {
  let design: string | null = null;
  for (const entry of variant.names) {
    if (entry.design == null) continue;
    if (entry.firstHour > hour) return design ?? entry.design;
    design = entry.design;
  }
  return design;
}

export function choiceKey(group: StockGroup, hour: number, shown: readonly string[] = []): string | null {
  const selected = shown.find(key => rowOf(key) === group.key);
  if (selected != null) return selected;
  if (group.kind !== 'variant') return group.key;
  const design = group.variants[0] ? designAt(group.variants[0], hour) : null;
  return design == null ? null : `${group.key}${DESIGN_MARK}${design}`;
}

function chosenGroup(byRow: ReadonlyMap<string, StockGroup>, key: string): StockGroup | undefined {
  const group = byRow.get(rowOf(key));
  if (!group || group.kind !== 'variant') return group?.key === key ? group : undefined;
  const at = key.lastIndexOf(DESIGN_MARK);
  if (at < 0) return undefined;
  const design = key.slice(at + 1);
  return group.designs?.includes(design) || group.variants[0]?.names.some(entry => entry.design === design) ? group : undefined;
}

export function stockChoices(groups: readonly StockGroup[], noun: string, drawable: (group: StockGroup) => boolean = () => true): ChoiceSource {
  const byRow = new Map(groups.map(group => [group.key, group]));
  return {
    lines: key => {
      const group = chosenGroup(byRow, key);
      return group && drawable(group) ? 1 : 0;
    },
    name: key => byRow.get(rowOf(key))?.label ?? 'Equipment name unavailable',
    noun,
  };
}

export const choiceRow = rowOf;

export const shareModeOf = (reply: EquipmentStockpileReply, grouping: Grouping) => grouping !== 'variant'
  && ((reply as PreparedStockpileReply).prepared?.balanceRecorded ?? reply.balanceHours.length > 0);

export function chartDrawable(reply: EquipmentStockpileReply, group: StockGroup, grouping: Grouping): boolean {
  const prepared = (reply as PreparedStockpileReply).prepared;
  if (!prepared) return chartRuns(reply, group, grouping).length > 0;
  const choice = prepared.choices[JSON.stringify([group.kind, group.kind === 'variant' ? group.variants[0]?.id : group.key])];
  return (shareModeOf(reply, grouping) ? choice?.shareDrawable : choice?.stockDrawable) ?? false;
}

export const chartRuns = (reply: EquipmentStockpileReply, group: StockGroup, grouping: Grouping) =>
  shareModeOf(reply, grouping) ? shareRuns(reply, group) : stockRuns(reply, group);

export function chartChoices(reply: EquipmentStockpileReply, grouping: Grouping): { value: string; label: string }[] {
  return stockGroups(reply, grouping, Infinity).flatMap(group => {
    const key = choiceKey(group, Infinity);
    return key != null && chartDrawable(reply, group, grouping) ? [{ value: setToken(key), label: group.label }] : [];
  });
}

export const CHOSEN_KEYS: Record<Grouping, string> = { type: 'chosen', variant: 'chosenVariant', domain: 'chosenDomain' };

const amount = (raw: bigint, decimals: number, digits = 2) => formatAmount(raw, decimals, digits);
const NOT_RECORDED = 'Not recorded';

function headRows(stock: bigint | null, daily: bigint | null, logistics: GroupBalance | null, mode: BalanceMode | 'variant' | 'unattributed', decimals: number): TooltipRow[] {
  const outputRow: TooltipRow = { label: 'Output per day', value: daily == null ? NOT_RECORDED : formatRate(daily, decimals), tone: daily == null ? 'muted' : undefined, divider: true };
  if (logistics) {
    const share = shareOf(logistics);
    return [
      { label: 'In stock', value: amount(logistics.stock, decimals), strong: true, tone: logistics.stock < 0n ? 'negative' : undefined },
      { label: 'Needed', value: logistics.need == null ? 'Unavailable' : amount(logistics.need, decimals), tone: logistics.need == null ? 'muted' : undefined },
      { label: logistics.balance < 0n ? 'Shortage' : 'Surplus', value: formatSigned(logistics.balance, decimals), strong: true,
        tone: logistics.balance < 0n ? 'negative' : logistics.balance > 0n ? 'positive' : undefined },
      ...(share != null ? [{ label: 'Share of need', value: `${formatPercent(share)}` }] : []),
      outputRow,
    ];
  }
  const stockRow: TooltipRow = { label: 'In stock', value: stock == null ? NOT_RECORDED : amount(stock, decimals), strong: true, tone: stock == null ? 'muted' : stock < 0n ? 'negative' : undefined };
  if (mode === 'unknown') return [stockRow, { label: 'Surplus or shortage', value: NOT_RECORDED, tone: 'muted' }, outputRow];
  if (mode === 'unattributed') return [stockRow, { label: 'Surplus or shortage', value: 'Not known', tone: 'muted' }, outputRow];
  return [stockRow, outputRow];
}

export function formatPercent(share: number): string {
  const percent = share * 100;
  const sign = percent > 0 ? '+' : percent < 0 ? '−' : '';
  const size = Math.abs(percent);
  const text = size >= 1 || size === 0 ? gbNumber.format(Math.round(size)) : gbTwoSignificant.format(size);
  return `${sign}${text}%`;
}

export const MAX_VARIANTS = 8;

function variantsInStock(reply: EquipmentStockpileReply, group: StockGroup, hour: number): { name: string; stock: bigint }[] {
  return group.variants.map(variant => ({ name: variantName(variant, hour), stock: stockAt(reply, { variants: [variant] }, hour) ?? 0n }))
    .filter(entry => entry.stock !== 0n)
    .sort((a, b) => (a.stock === b.stock ? a.name.localeCompare(b.name) : a.stock > b.stock ? -1 : 1));
}

export function groupTooltip(reply: EquipmentStockpileReply, group: StockGroup, grouping: Grouping, hour: number,
  head: Pick<TooltipContent, 'title' | 'subtitle' | 'note'>, height: number): TooltipContent {
  const mode = balanceMode(reply, hour);
  const logistics = balanceAt(reply, group, hour);
  const unattributed = mode === 'player' && group.types.length > 0 && !typesComplete(reply, group, hour);
  const rows = headRows(stockAt(reply, group, hour), dailyAt(reply, group, hour), logistics,
    grouping === 'variant' ? 'variant' : unattributed ? 'unattributed' : mode, reply.decimals);
  const note = [head.note,
    grouping === 'variant' ? 'Need is available by equipment type.' : undefined,
    logistics?.needNote ? 'Need unavailable.' : undefined,
    unattributed ? UNATTRIBUTED_NOTE : undefined].filter(Boolean).join(' ');
  const listed = grouping === 'variant' ? [] : variantsInStock(reply, group, hour);
  const typeLine: TooltipRow[] = grouping === 'variant' && group.variants[0]
    ? [{ label: typeName(reply, typeKey(group.variants[0])), tone: 'muted' }] : [];
  const form = (count: number): TooltipContent => {
    const more = listed.slice(count);
    return { ...head, note, rows: [...typeLine, ...rows,
      ...listed.slice(0, count).map(entry => ({ label: entry.name, value: amount(entry.stock, reply.decimals), indent: true })),
      ...(more.length ? [{ label: `${more.length} more ${more.length === 1 ? 'variant' : 'variants'}`, value: amount(more.reduce((sum, entry) => sum + entry.stock, 0n), reply.decimals), tone: 'muted' as const, indent: true }] : [])] };
  };
  function* forms(): Generator<TooltipContent> {
    for (let count = Math.min(MAX_VARIANTS, listed.length); count >= 0; count--) yield form(count);
  }
  return fitTooltip(forms(), height, { keep: 0, full: form(listed.length) });
}

export function groupLabel(reply: EquipmentStockpileReply, group: StockGroup, hour: number): string {
  if (balanceMode(reply, hour) === 'player' && group.types.length && !typesComplete(reply, group, hour)) return `${group.label}: surplus or shortage not known`;
  const logistics = balanceAt(reply, group, hour);
  if (logistics) {
    const share = shareOf(logistics);
    return `${group.label}: ${logistics.balance < 0n ? 'shortage' : 'surplus'} ${formatSigned(logistics.balance, reply.decimals)}${share != null ? `, ${formatPercent(share)} of need` : ''}`;
  }
  const stock = stockAt(reply, group, hour);
  return stock == null ? `${group.label}: not recorded at this time` : `${group.label}: ${amount(stock, reply.decimals)} in stock`;
}
