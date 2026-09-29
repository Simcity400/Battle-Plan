import { intervalIndexOf } from '../../charts/intervalIndex.ts';
import type { IntervalIndex } from '../../charts/intervalIndex';
import { runAt } from '../../charts/series.ts';
import type { Run } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { fixedNumber, formatAmount, formatSigned } from '../../charts/valueAxis.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { contentSet, setLimit, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options.ts';
import type { PoolRun, ResourceBalanceReply, ResourceInfo, ResourceSeries, TradeRun } from '../../generated/dashboard';

/** Where a country's resources come from and where they go, from the recorded
 * pools (see `crates/dashboard/src/products/resource_balance.rs` for what each
 * pool is and the evidence). Supply is produced + received from subjects +
 * imported; demand is what special projects and production lines need (met or
 * not) + sent to the overlord + the export reserve. The net is the game's own
 * surplus or deficit: what is left after special projects minus what the
 * production lines need. */

const RAW = /^-?\d{1,20}$/;
const HOUR = (value: unknown) => Number.isInteger(value);

const ordered = <T extends { firstHour: number; lastHour: number }>(runs: readonly T[]) =>
  runs.every((run, index) => run.firstHour <= run.lastHour && (index === 0 || runs[index - 1].lastHour < run.firstHour));

const POOLS = ['producedRaw', 'subjectsRaw', 'importedRaw', 'toExportRaw', 'exportedRaw', 'availableRaw', 'afterProjectsRaw', 'productionRaw'] as const;

/** Checks the reply's shape: resources by ascending native index, one series each,
 * runs and deal hours in hour order that never overlap, and exact raw values. */
export function validateResourceBalanceReply(data: unknown): ResourceBalanceReply {
  const reply = data as ResourceBalanceReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && typeof reply.tag === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.resources) && reply.resources.every((item, index) => Number.isInteger(item.id) && typeof item.token === 'string' && item.token.length > 0
      && Number.isInteger(item.ordinal) && item.ordinal >= 1 && (index === 0 || reply.resources[index - 1].id < item.id)
      && (item.name === null || typeof item.name === 'string') && (item.icon === null || (typeof item.icon === 'string' && item.icon.startsWith('data:image/png;base64,'))))
    && Array.isArray(reply.series) && reply.series.length === reply.resources.length
    && reply.series.every((series, index) => series.id === reply.resources[index].id && Array.isArray(series.runs) && ordered(series.runs)
      && series.runs.every(run => HOUR(run.firstHour) && HOUR(run.lastHour) && typeof run.startsSegment === 'boolean' && POOLS.every(key => RAW.test(run[key]))))
    && Array.isArray(reply.dealHours) && ordered(reply.dealHours)
    && Array.isArray(reply.trades) && reply.trades.every(trade => typeof trade.deal === 'string' && typeof trade.partner === 'string'
      && reply.resources.some(item => item.id === trade.resource) && (trade.direction === 'import' || trade.direction === 'export')
      && RAW.test(trade.deliveredRaw) && HOUR(trade.factories) && HOUR(trade.convoysAssigned) && HOUR(trade.convoysRequired)
      && HOUR(trade.firstHour) && HOUR(trade.lastHour) && trade.firstHour <= trade.lastHour && typeof trade.startsSegment === 'boolean')
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The resource balance reply is malformed.');
  return reply;
}

/** One resource's parts as exact raw integers. Needs are positive here. */
export interface Balance {
  produced: bigint; fromSubjects: bigint; toOverlord: bigint; imported: bigint;
  exportReserve: bigint; exported: bigint; available: bigint;
  projects: bigint; production: bigint;
  supply: bigint; demand: bigint; net: bigint;
}

export function balanceOf(run: PoolRun): Balance {
  const subjects = BigInt(run.subjectsRaw);
  const available = BigInt(run.availableRaw);
  const afterProjects = BigInt(run.afterProjectsRaw);
  const projects = available - afterProjects;
  const production = -BigInt(run.productionRaw);
  const produced = BigInt(run.producedRaw);
  const imported = BigInt(run.importedRaw);
  const exportReserve = BigInt(run.toExportRaw);
  const fromSubjects = subjects > 0n ? subjects : 0n;
  const toOverlord = subjects < 0n ? -subjects : 0n;
  return {
    produced, fromSubjects, toOverlord, imported, exportReserve, exported: BigInt(run.exportedRaw), available, projects, production,
    supply: produced + fromSubjects + imported,
    demand: projects + production + toOverlord + exportReserve,
    net: afterProjects - production,
  };
}

/** Whether the deals at an hour are known: the deal table was recorded at that
 * hour, as the product records it. Where it was not, the deals are unknown, not
 * none, even while the pools go on. */
export function dealsKnown(reply: Pick<ResourceBalanceReply, 'dealHours'>, hour: number): boolean {
  return runAt(reply.dealHours, hour) != null;
}

/** Deals covering an hour for one resource (by native index), largest first. */
export function tradesAt(index: IntervalIndex<TradeRun>, resource: number, hour: number): Deals {
  const covering = index.at(hour).filter(trade => trade.resource === resource)
    .sort((a, b) => Number(BigInt(b.deliveredRaw) - BigInt(a.deliveredRaw)) || a.partner.localeCompare(b.partner));
  return { imports: covering.filter(trade => trade.direction === 'import'), exports: covering.filter(trade => trade.direction === 'export') };
}

/** One resource's deals at an hour, each way. */
export interface Deals { imports: readonly TradeRun[]; exports: readonly TradeRun[] }

/** The reply's deals, indexed once per reply so no hover scans them (`IntervalIndex`). */
export const tradeIndex = (reply: ResourceBalanceReply) => intervalIndexOf(reply, () => reply.trades);

/** The countries trading with this one at an hour, for any of `resources` (all when
 * absent), each once, in the order first seen. */
export function partnersAt(index: IntervalIndex<TradeRun>, hour: number, resources?: readonly number[]): string[] {
  const partners = new Set<string>();
  for (const trade of index.at(hour)) {
    if (!resources || resources.includes(trade.resource)) partners.add(trade.partner);
  }
  return [...partners];
}

/** The hours where any of `resources` (native indices) changes, ascending: a pool
 * run, a deal or the deals' coverage (`dealHours`) starts, or one ends the hour
 * before, so a change to "not recorded" is a stop too. The over-time chart's
 * keyboard steps through them (the kit keeps those in view). */
export function changeHours(reply: ResourceBalanceReply, resources: readonly number[]): number[] {
  const hours = new Set<number>();
  const span = (run: { firstHour: number; lastHour: number }) => { hours.add(run.firstHour); hours.add(run.lastHour + 1); };
  const chosen = reply.series.filter(series => resources.includes(series.id));
  chosen.forEach(series => series.runs.forEach(span));
  for (const trade of reply.trades) {
    if (resources.includes(trade.resource)) span(trade);
  }
  // A breakdown shown there says whether the deals are known (the pool ends are stops already).
  for (const run of reply.dealHours) {
    for (const hour of [run.firstHour, run.lastHour + 1]) {
      if (chosen.some(series => runAt(series.runs, hour))) hours.add(hour);
    }
  }
  return [...hours].sort((a, b) => a - b);
}

/** The largest supply or demand of any resource: over the whole branch, or at one
 * hour. Bars share it, so a whole-campaign scale never rescales during playback. */
export function barScale(series: readonly ResourceSeries[], hour: number | null): bigint {
  let largest = 0n;
  for (const item of series) {
    const runs = hour == null ? item.runs : [runAt(item.runs, hour)].filter((run): run is PoolRun => run != null);
    for (const run of runs) {
      const balance = balanceOf(run);
      if (balance.supply > largest) largest = balance.supply;
      if (balance.demand > largest) largest = balance.demand;
    }
  }
  return largest;
}

export type RowOrder = 'game' | 'shortfall' | 'volume';

/** Resources in the game's order, largest shortfall first, or largest supply or
 * demand first. Resources not recorded at the hour keep the game's order after the others. */
export function orderResources(reply: ResourceBalanceReply, hour: number, order: RowOrder): ResourceInfo[] {
  const at = new Map(reply.series.map(series => [series.id, runAt(series.runs, hour)]));
  const key = (id: number): bigint | null => {
    const run = at.get(id);
    if (!run) return null;
    const balance = balanceOf(run);
    return order === 'shortfall' ? balance.net : -(balance.supply > balance.demand ? balance.supply : balance.demand);
  };
  if (order === 'game') return [...reply.resources];
  return reply.resources.map((item, index) => ({ item, index, key: key(item.id) }))
    .sort((a, b) => {
      if ((a.key == null) !== (b.key == null)) return a.key == null ? 1 : -1;
      if (a.key != null && b.key != null && a.key !== b.key) return a.key < b.key ? -1 : 1;
      return a.index - b.index;
    })
    .map(entry => entry.item);
}

/** The token of the resource with the largest supply or demand anywhere in the
 * branch: the over-time view's choice until the user picks. */
export function automaticResource(reply: ResourceBalanceReply): string | null {
  let best: { token: string; size: bigint } | null = null;
  for (let index = 0; index < reply.series.length; index++) {
    const series = reply.series[index];
    const size = barScale([series], null);
    if (series.runs.length && (!best || size > best.size)) best = { token: reply.resources[index].token, size };
  }
  return best?.token ?? reply.resources[0]?.token ?? null;
}

/** The resources shown for chosen tokens, in the order chosen: every declaration
 * of a token, since the game keeps a repeated token as another resource. */
export function chosenLines(reply: ResourceBalanceReply, tokens: readonly string[]): ResourceInfo[] {
  return tokens.flatMap(token => reply.resources.filter(item => item.token === token));
}

/** The chosen resources this recording has, in the order chosen; the automatic
 * choice when none are chosen or none are recorded here. */
export function chosenResources(reply: ResourceBalanceReply, chosen: readonly string[]): string[] {
  const known = chosen.filter(token => reply.resources.some(item => item.token === token));
  if (known.length) return known;
  const automatic = automaticResource(reply);
  return automatic ? [automatic] : [];
}

/** What clicking one in-widget chip does: the resources to store, or why it cannot. */
export type ChipAction = { value: string } | { reason: string };

/** Clicking one in-widget chip: that token toggled in the stored list, as the
 * settings panel's chips do (`toggleContentSet`). With nothing stored, the list
 * starts from the automatic choice shown, when that can be stored at all: choosing
 * another resource never requires storing one that cannot be. Tokens this campaign
 * lacks, such as another campaign's modded resource, are kept and count toward
 * `max`. A click that cannot act says why: its token cannot be stored, the limit is
 * reached, or it would leave nothing this campaign shows. */
export function toggleChosen(reply: ResourceBalanceReply, stored: readonly string[], shown: readonly string[], token: string, max: number): ChipAction {
  const base = stored.length ? stored : shown.filter(storableToken);
  if (!base.includes(token) && !storableToken(token)) return { reason: UNSTORABLE };
  const next = toggleContentSet(base.join(' '), token, max);
  if (next == null) return { reason: `At most ${setLimit(max)} can be chosen` };
  const lines = chosenLines(reply, contentSet(next));
  if (!lines.length) return { reason: 'One resource always stays chosen' };
  // The limit counts lines drawn: a repeated token draws one line per declaration.
  if (!base.includes(token) && lines.length > max) {
    const added = reply.resources.filter(item => item.token === token);
    return { reason: `At most ${max} lines can be drawn; ${resourceName(added[0])} adds ${added.length}` };
  }
  return { value: next };
}

/** Supply and demand as plotting runs, broken where the recording breaks. */
export function supplyDemandRuns(series: ResourceSeries, decimals: number): { supply: Run[]; demand: Run[] } {
  const supply: Run[] = [];
  const demand: Run[] = [];
  for (const run of series.runs) {
    const balance = balanceOf(run);
    supply.push({ firstHour: run.firstHour, lastHour: run.lastHour, value: fixedNumber(balance.supply, decimals), breakBefore: run.startsSegment });
    demand.push({ firstHour: run.firstHour, lastHour: run.lastHour, value: fixedNumber(balance.demand, decimals), breakBefore: run.startsSegment });
  }
  return { supply, demand };
}

/** A resource's name: content's, else its token; a repeated declaration of a
 * token is numbered ("Steel (2)"), as the game keeps it as another resource. */
export const resourceName = (item: Pick<ResourceInfo, 'name' | 'token' | 'ordinal'>) => `${item.name ?? item.token}${item.ordinal > 1 ? ` (${item.ordinal})` : ''}`;

/** Most deals listed per direction; the rest share one row. */
export const MAX_DEALS = 8;

/** Beyond this many resources over time, a tooltip never lists full breakdowns. */
export const FULL_BREAKDOWN_MAX = 2;

const factories = (count: number) => `${count} civilian ${count === 1 ? 'factory' : 'factories'}`;

/** The game-style breakdown of one resource at one hour: where it comes from,
 * where it goes (with each trade partner), what is available and what is needed,
 * then the surplus or deficit. Each direction lists its `maxDeals` largest deals
 * (they come largest first) and sums the rest in one row, so the totals always fit.
 * `trades` is null when the deals were not recorded at this hour: then no
 * partner and no remainder is shown, only that they were not recorded. */
export function breakdownRows(balance: Balance, trades: Deals | null, decimals: number, partner: CountryLabel, maxDeals = MAX_DEALS): TooltipRow[] {
  const amount = (raw: bigint) => formatAmount(raw, decimals);
  // A difference too small to show at two decimals is not listed as a part.
  const shown = (raw: bigint) => amount(raw < 0n ? -raw : raw) !== '0';
  const deal = (trade: TradeRun): TooltipRow => {
    const who = partner(trade.partner);
    const convoys = trade.convoysRequired > 0 ? ` · ${trade.convoysAssigned}/${trade.convoysRequired} convoys` : '';
    return { label: `${who.name} · ${factories(trade.factories)}${convoys}`, icon: who.flag, value: amount(BigInt(trade.deliveredRaw)), indent: true, short: true };
  };
  const total = (deals: readonly TradeRun[]) => deals.reduce((sum, trade) => sum + BigInt(trade.deliveredRaw), 0n);
  const limit = Math.max(1, maxDeals);
  // The largest deals by name; any beyond the limit share one row, and whatever
  // no deal accounts for is its own part, never folded into a deal.
  const deals = (pool: bigint, list: readonly TradeRun[] | undefined): TooltipRow[] => {
    // Unknown deals: nothing can be said about partners or what they leave out.
    if (!list) return pool ? [{ label: 'Trade partners not recorded at this hour', tone: 'muted', indent: true }] : [];
    const more = list.slice(limit);
    const rest = pool - total(list);
    return [...list.slice(0, limit).map(deal),
      ...(more.length ? [{ label: `${more.length} more ${more.length === 1 ? 'deal' : 'deals'}`, value: amount(total(more)), tone: 'muted' as const, indent: true }] : []),
      ...(shown(rest) ? [{ label: 'Not from a listed deal', value: formatSigned(rest, decimals), tone: 'muted' as const, indent: true }] : [])];
  };
  const rows: TooltipRow[] = [{ label: 'Produced', value: amount(balance.produced) }];
  if (balance.fromSubjects) rows.push({ label: 'From subjects', value: `+${amount(balance.fromSubjects)}` });
  rows.push({ label: 'Imported', value: balance.imported ? `+${amount(balance.imported)}` : '0' });
  rows.push(...deals(balance.imported, trades?.imports));
  if (balance.toOverlord) rows.push({ label: 'To the overlord', value: `−${amount(balance.toOverlord)}` });
  rows.push({ label: 'Exported (trade law)', value: balance.exportReserve ? `−${amount(balance.exportReserve)}` : '0' });
  rows.push(...deals(balance.exported, trades?.exports));
  const unsold = balance.exportReserve - balance.exported;
  if (shown(unsold)) rows.push({ label: unsold > 0n ? 'Set aside, not bought' : 'Bought beyond the reserve', value: amount(unsold < 0n ? -unsold : unsold), tone: 'muted', indent: true });
  rows.push({ label: 'Available', value: amount(balance.available), strong: true, divider: true });
  if (balance.projects) rows.push({ label: 'Needed by special projects', value: `−${amount(balance.projects)}` });
  rows.push({ label: 'Needed by production lines', value: balance.production ? `−${amount(balance.production)}` : '0' });
  rows.push({ label: balance.net < 0n ? 'Deficit' : 'Surplus', value: formatSigned(balance.net, decimals), strong: true, divider: true,
    tone: balance.net < 0n ? 'negative' : balance.net > 0n ? 'positive' : undefined });
  return rows;
}

/** The net as a tooltip value: signed, green for a surplus and red for a deficit. */
export function netValue(balance: Balance, decimals: number): Pick<TooltipRow, 'value' | 'tone'> {
  return { value: formatSigned(balance.net, decimals), tone: balance.net < 0n ? 'negative' : balance.net > 0n ? 'positive' : undefined };
}

/** Supply, demand and the surplus or deficit. */
export function summaryRows(balance: Balance, decimals: number): TooltipRow[] {
  return [{ label: 'Supply', value: formatAmount(balance.supply, decimals) }, { label: 'Demand', value: formatAmount(balance.demand, decimals) },
    { label: balance.net < 0n ? 'Deficit' : 'Surplus', ...netValue(balance, decimals) }];
}

/** Deal counts to list per direction, fullest first: up to [`MAX_DEALS`], down to one. */
function dealCounts(deals: readonly (Deals | null)[]): number[] {
  const most = Math.max(1, ...deals.map(each => each ? Math.max(each.imports.length, each.exports.length) : 0));
  return Array.from({ length: Math.min(MAX_DEALS, most) }, (_, index) => Math.min(MAX_DEALS, most) - index);
}

/** One resource's tooltip at an hour, fitted to a window `height` pixels tall
 * (`fitTooltip`): its breakdown with as many deals each way as fit, else its
 * supply, demand and net with the net always shown. A screen reader gets the full
 * breakdown either way. */
export function resourceTooltip(head: Pick<TooltipContent, 'title' | 'icon' | 'subtitle' | 'note'>, balance: Balance, deals: Deals | null, decimals: number,
  partner: CountryLabel, height: number): TooltipContent {
  function* forms(): Generator<TooltipContent> {
    for (const count of dealCounts([deals])) yield { ...head, rows: breakdownRows(balance, deals, decimals, partner, count) };
    const summary = summaryRows(balance, decimals);
    yield { ...head, rows: [...summary.slice(0, 2), { label: 'A taller window shows where it comes from and goes.', tone: 'muted' }, { ...summary[2], divider: true, strong: true }] };
  }
  return fitTooltip(forms(), height, { keep: 1 });
}

/** A resource drawn over time, at the hour a tooltip shows. */
export interface LineAt { resource: ResourceInfo; color: string; balance: Balance | null; deals: Deals | null }

/** The over-time tooltip for the chosen resources at an hour, fitted to a window
 * `height` pixels tall so every resource's net shows: each breakdown (for at most
 * [`FULL_BREAKDOWN_MAX`] resources) with as many deals as fit, else supply, demand
 * and net per resource, else one row per resource with its net, and a row counting
 * any left out. A screen reader gets the fullest form. */
export function overTimeTooltip(title: string, lines: readonly LineAt[], decimals: number, partner: CountryLabel, height: number): TooltipContent {
  const head = (line: LineAt, index: number): TooltipRow => ({ label: resourceName(line.resource), swatch: line.color, strong: true, divider: index > 0 });
  const missing: TooltipRow = { label: 'Not recorded', tone: 'muted' };
  function* forms(): Generator<TooltipContent> {
    if (lines.length <= FULL_BREAKDOWN_MAX) {
      for (const count of dealCounts(lines.map(line => line.deals))) {
        yield { title, rows: lines.flatMap((line, index) => [head(line, index), ...(line.balance ? breakdownRows(line.balance, line.deals, decimals, partner, count) : [missing])]) };
      }
    }
    yield { title, rows: lines.flatMap((line, index) => [head(line, index), ...(line.balance ? summaryRows(line.balance, decimals) : [missing])]),
      note: lines.length > FULL_BREAKDOWN_MAX ? `Choose at most ${FULL_BREAKDOWN_MAX} resources to see where each comes from and goes.` : 'A taller window shows where each comes from and goes.' };
    yield { title, note: 'Each resource’s net: a taller window shows supply and demand.',
      rows: lines.map(line => ({ label: resourceName(line.resource), swatch: line.color, short: true, ...(line.balance ? netValue(line.balance, decimals) : { value: 'Not recorded', tone: 'muted' as const }) })) };
  }
  return fitTooltip(forms(), height, { more: hidden => `${hidden} more ${hidden === 1 ? 'resource' : 'resources'}: a taller window shows them` });
}
