import { contentTitle } from '../../dashboard/contentTitles.ts';
import { runAt } from '../../charts/series.ts';
import type { Run } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatAmount, formatRate, formatSigned } from '../../charts/valueAxis.ts';
import { gbNumber, gbTwoSignificant } from '../../charts/numberFormat.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import type { ChoiceSource } from '../../dashboard/chosen';
import { contentKey, setToken } from '../../dashboard/options.ts';
import { DOMAINS, UNRESOLVED_ARM, UNRESOLVED_MARK, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import type { AmountRun, Domain, EquipmentStockpileReply, StockVariant } from '../../generated/dashboard';
import type { PreparedStockpileReply } from './preparedStockpileChart';

/** A country's equipment stockpile, from the recorded `Stockpiles` and production
 * lines, and its logistics record: the game's own surplus or shortage per equipment
 * type (the Logistics screen's "Stored" column) with the stock that record counted
 * (see `crates/dashboard/src/products/equipment_stockpile.rs` for each value's
 * meaning). Need uses the available operand of the same native
 * balance write. Older ordinary types derive it from max(0, stock); missing
 * convoy/truck/train observations keep their explicit unknown reason.
 * The game computes the balance only for player-controlled countries. */

const RAW = /^-?\d{1,20}$/;
const HOUR = (value: unknown) => Number.isInteger(value);

const ordered = <T extends { firstHour: number; lastHour: number }>(runs: readonly T[]) =>
  runs.every((run, index) => run.firstHour <= run.lastHour && (index === 0 || runs[index - 1].lastHour < run.firstHour));
const amounts = (runs: readonly AmountRun[]) => Array.isArray(runs) && ordered(runs)
  && runs.every(run => HOUR(run.firstHour) && HOUR(run.lastHour) && (run.amountRaw === null || RAW.test(run.amountRaw)));
const ascending = (hours: unknown) => Array.isArray(hours) && hours.every((hour, index) => HOUR(hour) && (index === 0 || hours[index - 1] < hour));

/** Checks the reply's shape: types in order with a known domain, each variant's runs
 * in hour order without overlaps and with exact raw values, names in hour order, and
 * reads and known spans ascending. */
export function validateStockpileReply(data: unknown): EquipmentStockpileReply {
  const reply = data as EquipmentStockpileReply;
  // A dashboard bridge from before the per-day output sends only a weekly figure, which
  // is not the game's per-day one: until it is rebuilt, output reads as not recorded.
  if (typeof reply === 'object' && reply != null && Array.isArray(reply.variants)
    && reply.variants.some(variant => variant != null && (variant as Partial<StockVariant>).daily === undefined)) {
    for (const variant of reply.variants) if (variant != null) Object.assign(variant, { daily: [], dailyIc: [], unitCostLow: [], unitCostHigh: [] });
    reply.lineHours = [];
  }
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string' && typeof reply.campaignKey === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.types) && reply.types.every(type => typeof type.key === 'string' && type.key.length > 0
      && (type.name === null || typeof type.name === 'string') && DOMAINS.includes(type.domain)
      && (type.needNote === null || typeof type.needNote === 'string'))
    && new Set(reply.types.map(type => type.key)).size === reply.types.length
    && Array.isArray(reply.variants) && reply.variants.every(variant => RAW.test(variant.id) && typeof variant.token === 'string'
      && (variant.archetype === null || reply.types.some(type => type.key === variant.archetype))
      && Array.isArray(variant.names) && variant.names.every((name, index) => HOUR(name.firstHour) && (name.name === null || typeof name.name === 'string') && typeof name.known === 'boolean'
        && (name.design === null || (typeof name.design === 'string' && name.design.length > 0))
        && (index === 0 || variant.names[index - 1].firstHour < name.firstHour))
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

/** Lookups built once per reply (held weakly, so a new reply builds its own): types and
 * logistics records by key, and the variants and logistics types whose membership is
 * unknown, which every completeness check scans instead of every variant. */
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

/** How rows are grouped: by equipment type (the game's archetypes), by exact
 * variant, or by land, air and naval. */
export type Grouping = 'type' | 'variant' | 'domain';

/** One row or line: the variants it adds up, and the equipment types (logistics
 * keys) whose balance it adds up; none for a variant, whose need the game does not
 * count on its own. */
export interface StockGroup {
  key: string;
  label: string;
  /** The arm it serves; null for a variant whose type content does not resolve. */
  domain: Domain | null;
  variants: readonly StockVariant[];
  types: readonly string[];
  /** The grouping it belongs to: a variant row is its own variant; a type or an arm
   * adds up variants whose membership must be known. */
  kind: Grouping;
  /** The included types it was built with (empty: every type). An unresolved type
   * is an archetype of its own, so its key decides whether it is included; an
   * excluded one never makes the group incomplete. A variant of unknown type could
   * belong to any included type, so it always counts. */
  scope: readonly string[];
  /** Branch-wide identity witnesses, separate from the reached naming page. */
  designs?: readonly string[];
}

/** A group's label at an hour: a variant is named as recorded then (it can be renamed);
 * a type or an arm keeps its name. */
export const labelAt = (group: StockGroup, grouping: Grouping, hour: number) =>
  grouping === 'variant' && group.variants[0] ? variantName(group.variants[0], hour) : group.label;

/** Whether the reply holds anything to show: a stockpile read, production, a logistics
 * record or the hours the country was AI-controlled. What was not recorded is said
 * per value; only a reply with none of these is unavailable. */
export const hasRecords = (reply: EquipmentStockpileReply) => {
  const prepared = (reply as PreparedStockpileReply).prepared;
  return reply.reads.length > 0 || reply.lineReads.length > 0 || reply.logisticsReads.length > 0 || reply.aiHours.length > 0
    || !!prepared && (prepared.stockRecorded || prepared.linesRecorded || prepared.balanceRecorded || prepared.aiRecorded);
};

/** A variant's name as recorded at an hour, else its token or handle. */
export const variantName = (variant: StockVariant, hour: number) =>
    variantNameAt(variant.names, hour, `Variant ${variant.id}`);

/** A variant row's key, as chips store it: its campaign (the kit's `campaignToken`),
 * handle and equipment token. A handle is the game's equipment id, so it names one
 * design only within its campaign: a choice saved in one campaign never selects
 * another campaign's design that reuses the handle, even of the same equipment type. */
export const variantKey = (reply: Pick<EquipmentStockpileReply, 'campaignKey'>, variant: StockVariant) =>
  campaignToken(reply.campaignKey, `${variant.id}:${variant.token}`);

/** A variant's type key: its archetype, else its own equipment token (without
 * content), else its handle. Keys of unresolved types are marked so they never
 * collide with an archetype. */
const typeKey = (variant: StockVariant) => variant.archetype ?? `${UNRESOLVED_MARK}${variant.token || variant.id}`;

/** The type's name: content's, else its key. */
export function typeName(reply: EquipmentStockpileReply, key: string): string {
  const type = indexOf(reply).types.get(key);
  if (type) return contentTitle(type.name, type.key, 'Equipment type name unavailable');
  return unresolvedTypeName(key);
}

/** The groups of the reply, in the game's order: types in the equipment database's
 * order (unresolved ones after, by token), variants by their type's place then
 * handle, and land, air, naval. `included` limits the types shown (empty: all). */
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
  // Types the logistics record lists. A recorded zero is a value: every type of a shown
  // row counts toward it (an arm's need depends on all of them, whatever order the record
  // lists them in), but a type with nothing but zeros adds no row of its own.
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

/** A value summed over a group's variants at an hour: null where the table is not
 * known then (outside every span), else the sum, a variant without a run holding none. */
function sumAt(spans: EquipmentStockpileReply['stockHours'], variants: readonly StockVariant[], pick: (variant: StockVariant) => readonly AmountRun[], hour: number): bigint | null {
  if (!runAt(spans, hour)) return null;
  let total = 0n;
  for (const variant of variants) {
    const run = runAt(pick(variant), hour);
    if (!run) continue;
    // Read but not computable (a production rate without a cost): the total is unknown.
    if (run.amountRaw === null) return null;
    total += BigInt(run.amountRaw);
  }
  return total;
}

/** A group that stockAt and dailyAt sum: a row, or one variant alone (no kind). */
type Summed = Pick<StockGroup, 'variants'> & Partial<Pick<StockGroup, 'types' | 'kind'>>;

/** Whether a type's or an arm's variants are all of it at an hour, for one measure: a
 * variant row, or one variant alone, is always its own. A type or an arm is not when it
 * holds a type content does not resolve (marked `UNRESOLVED_MARK`, whose variants cannot be told), or when
 * any variant whose type is unknown (its definition unread, or not resolved) holds a
 * nonzero or unknown value then: it could belong to any of them, so every such total
 * is unknown, never zero or partial. */
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

/** What a group holds at an hour; null where the stockpile is not known or its
 * variants cannot all be told (`variantsComplete`). */
export const stockAt = (reply: EquipmentStockpileReply, group: Summed, hour: number) =>
  variantsComplete(reply, group, stockOf, hour) ? sumAt(reply.stockHours, group.variants, stockOf, hour) : null;

/** What a group's lines make a day at an hour, by the game's per-day formula; null
 * where the lines are not known or its variants cannot all be told. */
export const dailyAt = (reply: EquipmentStockpileReply, group: Summed, hour: number) =>
  variantsComplete(reply, group, dailyOf, hour) ? sumAt(reply.lineHours, group.variants, dailyOf, hour) : null;

/** The factory IC a day a group's lines apply at an hour; null likewise. */
export const dailyIcAt = (reply: EquipmentStockpileReply, group: Summed, hour: number) =>
  variantsComplete(reply, group, dailyIcOf, hour) ? sumAt(reply.lineHours, group.variants, dailyIcOf, hour) : null;

/** Whether the game's surplus or shortage is known at an hour: `player` where the
 * logistics record held the country's rows, `ai` where the country was AI-controlled
 * (the game computes none), `unknown` where the record was not read. */
export type BalanceMode = 'player' | 'ai' | 'unknown';

export function balanceMode(reply: EquipmentStockpileReply, hour: number): BalanceMode {
  if (runAt(reply.balanceHours, hour)) return 'player';
  if (runAt(reply.aiHours, hour)) return 'ai';
  return 'unknown';
}

/** Why an arm's surplus is not known while a type of unknown arm holds values. */
export const UNATTRIBUTED_NOTE = 'Total unavailable.';

/** The note for an AI-controlled country (user decision). */
export const AI_NOTE = 'The game doesn’t compute surplus or shortage for AI-controlled countries.';
const NEED_UNAVAILABLE = 'Need unavailable.';

/** A group's logistics at an hour: surplus or shortage, stock, and signed need
 * from the same native available operand; older ordinary types derive it from
 * stock. Missing supply/convoy observations retain their reason. */
export interface GroupBalance { balance: bigint; stock: bigint; need: bigint | null; needNote: string | null }

/** A group's logistics where the balance is known at the hour; null elsewhere and for
 * a group without types (a variant). A type without a run holds zero. */
/** Whether a group's logistics types are all of it at an hour: a type's own row always
 * is (each archetype has its own record). A resolved arm is not when any type content
 * does not resolve (marked `UNRESOLVED_MARK`, whose arm cannot be told) holds a nonzero or unknown balance
 * or stock then: it could belong to that arm, so the arm's sums are unknown, never
 * partial. The unresolved arm is the sum of exactly those types. */
/** Whether a logistics type decides a group's completeness (`typesComplete`): an
 * unresolved type, for a resolved arm. */
const decidesCompleteness = (group: Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>, key: string) =>
  group.kind === 'domain' && group.key !== UNRESOLVED_ARM && key.startsWith(UNRESOLVED_MARK) && (!group.scope?.length || group.scope.includes(key));

/** The unresolved logistics types a group's completeness depends on: those its
 * included types admit (every one without a filter). */
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
  // Every measure summed from the record (balance, stock, need, share) is unknown
  // wherever the group's types cannot all be told.
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
    // Availability subtraction wraps at signed i64 width. Recover the
    // same signed clamped need from the observed operand and stored result.
    const nativeAvailable = available ?? (typeStock > 0n ? typeStock : 0n);
    if (note) availableTotal = null;
    else if (availableTotal != null) availableTotal += nativeAvailable;
    const typeNeed = BigInt.asIntN(64, nativeAvailable - typeBalance);
    const reason = note ?? ((typeNeed < 0n ? -typeNeed : typeNeed) >= bound ? NEED_UNAVAILABLE : null);
    if (reason) { need = null; needNote ??= reason; } else if (need != null) need += typeNeed;
  }
  // A resolved arm's need: an unresolved type listed now, even at zero, may hold
  // unrecorded supply terms in its available stock, so its zeros prove no zero need.
  if (group.kind === 'domain' && group.key !== UNRESOLVED_ARM
    && unresolvedIn(reply, group).some(item => runAt(item.balance, hour) != null || runAt(item.stock, hour) != null)) {
    need = null;
    availableTotal = null;
    needNote ??= NEED_UNAVAILABLE;
  }
  return { balance, stock, need, needNote, available: availableTotal };
}

/** Preserve the existing overview's independent stock/balance/need contract. */
export function balanceAt(reply: EquipmentStockpileReply, group: Pick<StockGroup, 'types'> & Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>, hour: number): GroupBalance | null {
  const value = balanceOperandsAt(reply, group, hour);
  if (!value) return null;
  const { balance, stock, need, needNote } = value;
  return { balance, stock, need, needNote };
}

/** The surplus or shortage as a share of what is needed; null where the need is not
 * derived or there is none. */
export function shareOf(value: Pick<GroupBalance, 'balance' | 'need'> | null): number | null {
  if (!value || value.need == null || value.need <= 0n) return null;
  // A float division of the two exact amounts: never truncated to zero for a nonzero
  // surplus, however small its share.
  return Number(value.balance) / Number(value.need);
}

/** A group's surplus as a percentage of need over time, broken where the balance is
 * not known or the need is not derived. */
export function shareRuns(reply: EquipmentStockpileReply, group: Pick<StockGroup, 'types'> & Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>): Run[] {
  const runs: Run[] = [];
  if (!group.types.length) return runs;
  const hours = new Set<number>(reply.balanceHours.map(span => span.firstHour));
  // Split wherever the group's own types change, and wherever a type that decides its
  // completeness (an unresolved type, for a resolved arm) changes.
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

/** Hours at which a group's stock can change: each span's start and every run's start
 * inside it, ascending. */
function changeHours(reply: EquipmentStockpileReply, variants: readonly StockVariant[]): number[] {
  const hours = new Set<number>(reply.stockHours.map(span => span.firstHour));
  // A variant of unknown type can make any total unknown where its value changes.
  for (const variant of [...variants, ...indexOf(reply).untypedVariants]) {
    for (const run of variant.stock) { hours.add(run.firstHour); hours.add(run.lastHour + 1); }
  }
  return [...hours].sort((a, b) => a - b);
}

/** A group's stock as plotting runs, broken where the recording breaks: one run per
 * stretch of equal value inside each known span. */
export function stockRuns(reply: EquipmentStockpileReply, group: Summed): Run[] {
  const runs: Run[] = [];
  const changes = changeHours(reply, group.variants);
  for (const span of reply.stockHours) {
    const inside = changes.filter(hour => hour >= span.firstHour && hour <= span.lastHour);
    inside.forEach((first, index) => {
      const last = index + 1 < inside.length ? inside[index + 1] - 1 : span.lastHour;
      const stock = stockAt(reply, group, first);
      // An unknown value breaks the line, never drawn as zero.
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

/** Hours at which a group's logistics values can change: each balance span's start and
 * every run edge of its types and of the types that decide its completeness. */
export function balanceChangeHours(reply: EquipmentStockpileReply, group: Pick<StockGroup, 'types'> & Partial<Pick<StockGroup, 'kind' | 'key' | 'scope'>>): number[] {
  const hours = new Set<number>(reply.balanceHours.map(span => span.firstHour));
  const index = indexOf(reply);
  const items = [...group.types.map(key => index.logistics.get(key)), ...(group.kind === 'domain' && group.key !== UNRESOLVED_ARM ? unresolvedIn(reply, group) : [])];
  for (const item of items) {
    if (!item) continue;
    for (const run of [...item.balance, ...item.stock, ...item.available]) { hours.add(run.firstHour); hours.add(run.lastHour + 1); }
  }
  return [...hours].sort((a, b) => a - b);
}

/** The largest stock a group held, or need it had, at any read. */
function peak(reply: EquipmentStockpileReply, group: StockGroup): bigint {
  const prepared = (reply as PreparedStockpileReply).prepared;
  if (prepared) return BigInt(prepared.choices[JSON.stringify([group.kind, group.kind === 'variant' ? group.variants[0]?.id : group.key])]?.raw ?? '0');
  let most = changeHours(reply, group.variants).reduce((best, hour) => {
    const stock = stockAt(reply, group, hour) ?? 0n;
    return stock > best ? stock : best;
  }, 0n);
  for (const hour of balanceChangeHours(reply, group)) {
    const need = balanceAt(reply, group, hour)?.need ?? 0n;
    if (need > most) most = need;
  }
  return most;
}

/** The chart's choice until the user picks: the main land and air types (the three
 * land and two air types with the most in stock or needed at any read), the five
 * variants with the most, or every arm. */
export function automaticGroups(reply: EquipmentStockpileReply, groups: readonly StockGroup[], grouping: Grouping): string[] {
  if (grouping === 'domain') return groups.map(group => group.key);
  const ranked = groups.map(group => ({ group, peak: peak(reply, group) })).filter(entry => entry.peak > 0n)
    .sort((a, b) => (a.peak === b.peak ? 0 : a.peak > b.peak ? -1 : 1));
  if (grouping === 'variant') return ranked.slice(0, 5).map(entry => entry.group.key);
  const pick = (domain: Domain, count: number) => ranked.filter(entry => entry.group.domain === domain).slice(0, count).map(entry => entry.group.key);
  const main = [...pick('land', 3), ...pick('air', 2)];
  return main.length ? main : ranked.slice(0, 5).map(entry => entry.group.key);
}

/** The chart's lines until the user picks: `automaticGroups` among the groups that have
 * something to draw, so a type with no share (trucks, trains) never takes a place;
 * any places left are filled with the next drawable groups in the game's order. */
export function automaticLines(reply: EquipmentStockpileReply, groups: readonly StockGroup[], grouping: Grouping, drawable: (group: StockGroup) => boolean, count = 5): string[] {
  const shown = groups.filter(drawable);
  const main = automaticGroups(reply, shown, grouping);
  if (grouping === 'domain' || main.length >= count) return main;
  return [...main, ...shown.map(group => group.key).filter(key => !main.includes(key))].slice(0, Math.max(main.length, count));
}

/** The groups as chips choose them: one line each. */
export const groupChoices = (groups: readonly StockGroup[], noun: string): ChoiceSource => ({
  lines: key => groups.some(group => group.key === key) ? 1 : 0,
  name: key => groups.find(group => group.key === key)?.label ?? 'Equipment group name unavailable',
  noun,
});

/** Separates a variant row's key from its design in a stored choice (`#` starts a
 * comment in the game's scripts, so no token or key holds it). */
const DESIGN_MARK = '#';
const rowOf = (key: string) => {
  const at = key.lastIndexOf(DESIGN_MARK);
  return at < 0 ? key : key.slice(0, at);
};

/** A variant's design as recorded by the hour (the product's fingerprint of its name,
 * modules and upgrade levels): the latest recorded at or before it, else its first;
 * null when never recorded. */
export function designAt(variant: StockVariant, hour: number): string | null {
  let design: string | null = null;
  for (const entry of variant.names) {
    if (entry.design == null) continue;
    if (entry.firstHour > hour) return design ?? entry.design;
    design = entry.design;
  }
  return design;
}

/** What a chip stores for a group (the kit's `setToken` encodes it): a type or an arm
 * by its key; a variant by its key and its design as recorded by the hour shown, since
 * a reload restores the game's variant index and a later branch can give the handle to
 * another design. A shown choice keeps the fingerprint that selected it when toggled.
 * Null for a variant whose design was never recorded: a choice of it
 * could not be told from another design. */
export function choiceKey(group: StockGroup, hour: number, shown: readonly string[] = []): string | null {
  const selected = shown.find(key => rowOf(key) === group.key);
  if (selected != null) return selected;
  if (group.kind !== 'variant') return group.key;
  const design = group.variants[0] ? designAt(group.variants[0], hour) : null;
  return design == null ? null : `${group.key}${DESIGN_MARK}${design}`;
}

/** Stored choice tokens with one per row, the first kept: a list saved before a
 * variant's design changed can hold two for one row. */
export function oneChoicePerRow(tokens: readonly string[]): string[] {
  const rows = new Set<string>();
  return tokens.filter(token => {
    const row = choiceRow(contentKey(token));
    if (rows.has(row)) return false;
    rows.add(row);
    return true;
  });
}

/** The group a stored choice selects: its row, and for a variant only while that row's
 * variant has had the stored design in this branch (a handle is one design within a
 * branch; a rename changes the fingerprint, so the branch's whole history is matched,
 * never only the hour shown). None for another design under the same handle. */
function chosenGroup(byRow: ReadonlyMap<string, StockGroup>, key: string): StockGroup | undefined {
  const group = byRow.get(rowOf(key));
  if (!group || group.kind !== 'variant') return group?.key === key ? group : undefined;
  const at = key.lastIndexOf(DESIGN_MARK);
  if (at < 0) return undefined;
  const design = key.slice(at + 1);
  return group.designs?.includes(design) || group.variants[0]?.names.some(entry => entry.design === design) ? group : undefined;
}

/** What the chips over the chart choose from, over the keys `choiceKey` gives: a group
 * draws one line only when it can be drawn (`drawable`) and, for a variant, only when
 * the stored design is its own. */
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

/** The row a stored choice names, whatever design it was stored with. */
export const choiceRow = rowOf;

export const SAVED_DESIGN_UNAVAILABLE = 'Saved design unavailable. Choose another.';

/** Whether the chart shows each group's share of need (a recorded balance, and not by
 * variant, which has none of its own) or its stock. */
export const shareModeOf = (reply: EquipmentStockpileReply, grouping: Grouping) => grouping !== 'variant'
  && ((reply as PreparedStockpileReply).prepared?.balanceRecorded ?? reply.balanceHours.length > 0);

export function chartDrawable(reply: EquipmentStockpileReply, group: StockGroup, grouping: Grouping): boolean {
  const prepared = (reply as PreparedStockpileReply).prepared;
  if (!prepared) return chartRuns(reply, group, grouping).length > 0;
  const choice = prepared.choices[JSON.stringify([group.kind, group.kind === 'variant' ? group.variants[0]?.id : group.key])];
  return (shareModeOf(reply, grouping) ? choice?.shareDrawable : choice?.stockDrawable) ?? false;
}

/** A group's line over time in the chart's mode; empty when it has nothing to draw. */
export const chartRuns = (reply: EquipmentStockpileReply, group: StockGroup, grouping: Grouping) =>
  shareModeOf(reply, grouping) ? shareRuns(reply, group) : stockRuns(reply, group);

/** The settings' choices of lines over time for a grouping, as `setToken(choiceKey)`:
 * only what can be drawn for this country and grouping. A type whose need cannot be
 * derived has no share, and a variant whose design was never recorded cannot be told
 * from another design under its handle; the chips over the chart say why of each. */
export function chartChoices(reply: EquipmentStockpileReply, grouping: Grouping): { value: string; label: string }[] {
  return stockGroups(reply, grouping, Infinity).flatMap(group => {
    const key = choiceKey(group, Infinity);
    return key != null && chartDrawable(reply, group, grouping) ? [{ value: setToken(key), label: group.label }] : [];
  });
}

export const GROUP_NOUNS: Record<Grouping, string> = { type: 'equipment type', variant: 'variant', domain: 'arm' };

/** The setting that keeps the lines chosen over time for a grouping: each grouping's
 * choice is its own, so switching grouping never carries another's keys over. */
export const CHOSEN_KEYS: Record<Grouping, string> = { type: 'chosen', variant: 'chosenVariant', domain: 'chosenDomain' };

const amount = (raw: bigint, decimals: number, digits = 2) => formatAmount(raw, decimals, digits);
const NOT_RECORDED = 'Not recorded';

/** The rows every group's tooltip starts with, by what is known at the hour: for a
 * player-controlled country the logistics record's stock, what is needed, the surplus
 * or shortage and its share of need; else what is in stock; then the output per day. */
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

/** A share of need as the tooltip and end labels write it: signed, whole per cent. */
export function formatPercent(share: number): string {
  const percent = share * 100;
  const sign = percent > 0 ? '+' : percent < 0 ? '−' : '';
  const size = Math.abs(percent);
  // Whole per cent from one up; below it, two significant digits, so a nonzero share
  // never reads as 0%.
  const text = size >= 1 || size === 0 ? gbNumber.format(Math.round(size)) : gbTwoSignificant.format(size);
  return `${sign}${text}%`;
}

/** Most variants a group's tooltip lists; the rest share one row. */
export const MAX_VARIANTS = 8;

/** The variants of a group in stock at an hour, most first. */
function variantsInStock(reply: EquipmentStockpileReply, group: StockGroup, hour: number): { name: string; stock: bigint }[] {
  return group.variants.map(variant => ({ name: variantName(variant, hour), stock: stockAt(reply, { variants: [variant] }, hour) ?? 0n }))
    .filter(entry => entry.stock !== 0n)
    .sort((a, b) => (a.stock === b.stock ? a.name.localeCompare(b.name) : a.stock > b.stock ? -1 : 1));
}

/** One group's tooltip at an hour, fitted to a window `height` pixels tall: in stock,
 * need and surplus (not recorded), output per day, and the variants in stock (as
 * many as fit, the rest summed). A screen reader gets the full list either way. */
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
  // The eye gets at most MAX_VARIANTS; a screen reader gets every variant.
  return fitTooltip(forms(), height, { keep: 0, full: form(listed.length) });
}

/** A group's line for assistive technology: its surplus or shortage where known,
 * else its stock. */
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

/** A chosen group drawn over time, at the hour a tooltip shows. */
export interface LineAt { group: StockGroup; color: string; stock: bigint | null; daily: bigint | null; logistics?: GroupBalance | null }

/** The over-time tooltip for the chosen groups at an hour, fitted to the window: each
 * group's surplus or shortage with its share of need (the share chart) or its stock,
 * and output per day, else one row per group, and a row counting any left out. */
export function overTimeTooltip(title: string, lines: readonly LineAt[], decimals: number, note: string | undefined, height: number): TooltipContent {
  const value = (line: LineAt) => {
    if (line.logistics !== undefined) {
      if (!line.logistics) return { value: NOT_RECORDED, tone: 'muted' as const };
      const share = shareOf(line.logistics);
      return { value: `${formatSigned(line.logistics.balance, decimals)}${share != null ? ` (${formatPercent(share)})` : ''}`,
        tone: line.logistics.balance < 0n ? 'negative' as const : undefined };
    }
    return line.stock == null ? { value: NOT_RECORDED, tone: 'muted' as const } : { value: amount(line.stock, decimals) };
  };
  function* forms(): Generator<TooltipContent> {
    yield { title, note, rows: lines.flatMap((line, index) => [
      { label: line.group.label, swatch: line.color, strong: true, divider: index > 0, ...value(line) },
      ...(line.logistics ? [{ label: 'Needed', indent: true, ...(line.logistics.need == null ? { value: 'Unavailable', tone: 'muted' as const } : { value: amount(line.logistics.need, decimals) }) }] : []),
      { label: 'Output per day', indent: true, ...(line.daily == null ? { value: NOT_RECORDED, tone: 'muted' as const } : { value: formatRate(line.daily, decimals) }) },
    ]) };
    yield { title, note, rows: lines.map(line => ({ label: line.group.label, swatch: line.color, short: true, ...value(line) })) };
  }
  return fitTooltip(forms(), height, { more: hidden => `${hidden} more` });
}
