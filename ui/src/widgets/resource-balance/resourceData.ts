import { contentTitle } from '../../dashboard/contentTitles.ts';
import { artworkUrlValue } from '../../dashboard/payload.ts';
import { chosenShown, largestToken, toggleChosen as toggleKit } from '../../dashboard/chosen.ts';
import type { ChipAction, ChoiceSource } from '../../dashboard/chosen';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatSigned } from '../../charts/valueAxis.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { ResourceBalanceReply, ResourceDeal, ResourceInfo, ResourceSizesReply } from '../../generated/dashboard';

/** Where a country's resources come from and where they go, from the recorded pools: supply is produced + received
 * from subjects + imported; demand is what special projects and production lines need (met or not) + sent to the
 * overlord + the export reserve. The net is the game's own surplus or deficit: what is left after special projects
 * minus what the production lines need. */

export const RESOURCE_BALANCE = 'resource-balance';
export const RESOURCE_SIZES = 'resource-sizes';

const RAW = /^-?\d{1,20}$/;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;

/** One resource's recorded pools at an hour, exact raw fixed-point strings. */
export interface ResourcePools {
  id: number;
  producedRaw: string; subjectsRaw: string; importedRaw: string; toExportRaw: string; exportedRaw: string;
  availableRaw: string; afterProjectsRaw: string; productionRaw: string;
}

/** Each pool's live cell column (the product's `POOL_CELLS`) and the field it fills. */
export const POOL_CELLS = [['produced', 'producedRaw'], ['subjects', 'subjectsRaw'], ['imported', 'importedRaw'], ['toExport', 'toExportRaw'],
  ['exported', 'exportedRaw'], ['available', 'availableRaw'], ['afterProjects', 'afterProjectsRaw'], ['production', 'productionRaw']] as const;

/** Resource `id`'s pools from the live cells, or null where any is not known, `missing` saying why. */
export function poolsAt(values: LiveValues<number>, id: number): ResourcePools | null {
  const pools: Record<string, unknown> = { id };
  for (const [column, field] of POOL_CELLS) {
    const value = values.get(liveKey(String(id), column));
    if (value === undefined || !Number.isFinite(value)) return null;
    pools[field] = BigInt(Math.round(value)).toString();
  }
  return pools as unknown as ResourcePools;
}

/** Why resource `id`'s pools are not known in `values`. */
export const poolsMissing = (values: LiveValues<number>, id: number) => {
  for (const [column] of POOL_CELLS) if (values.get(liveKey(String(id), column)) === undefined) return values.missing(liveKey(String(id), column));
  return 'Not recorded at this time';
};

/** The pools of every resource the reply names a row of that `values` holds. */
export const poolsOf = (reply: ResourceBalanceReply, values: LiveValues<number>): ResourcePools[] =>
  reply.pools.flatMap(id => { const pools = poolsAt(values, id); return pools ? [pools] : []; });

/** Checks the reply: resources by ascending native index, pools and shown resources known, deals of known resources. */
export function validateResourceBalance(value: unknown): ResourceBalanceReply {
  const reply = value as ResourceBalanceReply;
  const known = (id: unknown) => object(value) && Array.isArray(value.resources) && value.resources.some(item => object(item) && item.id === id);
  const ok = object(value) && typeof reply.tag === 'string' && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.resources) && reply.resources.every((item, index) => object(item) && Number.isInteger(item.id) && typeof item.token === 'string'
      && item.token.length > 0 && Number.isInteger(item.ordinal) && item.ordinal >= 1 && (item.name === null || typeof item.name === 'string')
      && (item.icon === null || artworkUrlValue(item.icon)) && (index === 0 || reply.resources[index - 1].id < item.id))
    && Array.isArray(reply.pools) && reply.pools.every(known) && Array.isArray(reply.shown) && reply.shown.every(known)
    && (reply.poolsMissing === null || typeof reply.poolsMissing === 'string') && (reply.dealsMissing === null || typeof reply.dealsMissing === 'string')
    && Array.isArray(reply.deals) && reply.deals.every(deal => object(deal) && typeof deal.deal === 'string' && typeof deal.partner === 'string'
      && known(deal.resource) && (deal.direction === 'import' || deal.direction === 'export')
      && Number.isInteger(deal.factories) && Number.isInteger(deal.convoysAssigned) && Number.isInteger(deal.convoysRequired))
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!ok) throw new Error('The resource balance reply is malformed.');
  return reply;
}

export function validateResourceSizes(value: unknown): ResourceSizesReply {
  if (!object(value) || typeof value.tag !== 'string' || !Array.isArray(value.sizes) || !value.sizes.every(size => object(size)
    && Number.isInteger(size.id) && (size.longestRaw === null || RAW.test(size.longestRaw as string)))
    || !Array.isArray(value.missing) || !Array.isArray(value.warnings))
    throw new Error('The resource sizes reply is malformed.');
  return value as unknown as ResourceSizesReply;
}

/** One resource's parts as exact raw integers. Needs are positive here. */
export interface Balance {
  produced: bigint; fromSubjects: bigint; toOverlord: bigint; imported: bigint;
  exportReserve: bigint; exported: bigint; available: bigint;
  projects: bigint; production: bigint;
  supply: bigint; demand: bigint; net: bigint;
}

export function balanceOf(pools: ResourcePools): Balance {
  const subjects = BigInt(pools.subjectsRaw);
  const available = BigInt(pools.availableRaw);
  const afterProjects = BigInt(pools.afterProjectsRaw);
  const projects = available - afterProjects;
  const production = -BigInt(pools.productionRaw);
  const produced = BigInt(pools.producedRaw);
  const imported = BigInt(pools.importedRaw);
  const exportReserve = BigInt(pools.toExportRaw);
  const fromSubjects = subjects > 0n ? subjects : 0n;
  const toOverlord = subjects < 0n ? -subjects : 0n;
  return {
    produced, fromSubjects, toOverlord, imported, exportReserve, exported: BigInt(pools.exportedRaw), available, projects, production,
    supply: produced + fromSubjects + imported,
    demand: projects + production + toOverlord + exportReserve,
    net: afterProjects - production,
  };
}

/** The parts each balance bar draws, in order, signed as recorded: supply (produced, from subjects, imported) and
 * demand (needed, to the overlord, export reserve). */
export function barParts(balance: Balance): { supply: [string, bigint][]; demand: [string, bigint][] } {
  return { supply: [['produced', balance.produced], ['subjects', balance.fromSubjects], ['imported', balance.imported]],
    demand: [['needed', balance.projects + balance.production], ['overlord', balance.toOverlord], ['exported', balance.exportReserve]] };
}

/** A bar's drawn length: its parts' magnitudes, so a negative part never shortens it. */
export const barLength = (parts: readonly (readonly [string, bigint])[]) => parts.reduce((sum, [, raw]) => sum + (raw < 0n ? -raw : raw), 0n);

/** The longest supply or demand bar of the pools shown. */
export function momentScale(pools: readonly ResourcePools[]): bigint {
  let largest = 0n;
  for (const pool of pools) {
    const parts = barParts(balanceOf(pool));
    for (const length of [barLength(parts.supply), barLength(parts.demand)]) if (length > largest) largest = length;
  }
  return largest;
}

/** The longest bar any resource draws over the campaign, from `resource-sizes`. */
export function campaignScale(sizes: ResourceSizesReply | undefined): bigint {
  let largest = 0n;
  for (const size of sizes?.sizes ?? []) if (size.longestRaw !== null && BigInt(size.longestRaw) > largest) largest = BigInt(size.longestRaw);
  return largest;
}

/** A part's share of `scale` as a CSS width. */
export const partWidth = (raw: bigint, scale: bigint) => scale > 0n ? `${Number((raw < 0n ? -raw : raw) * 100000n / scale) / 1000}%` : '0%';

export type RowOrder = 'game' | 'shortfall' | 'volume';

/** The resources the bars show, in the order the product ranked them at the reply's hour (`shown`). */
export function orderResources(reply: ResourceBalanceReply): ResourceInfo[] {
  const byId = new Map(reply.resources.map(item => [item.id, item]));
  return reply.shown.flatMap(id => byId.get(id) ?? []);
}

/** What a bars render depends on: which resources in which order, and whether each has pools; amounts are painted. */
export function barStructure(reply: ResourceBalanceReply): string {
  const held = new Set(reply.pools);
  return JSON.stringify([reply.poolsMissing, reply.dealsMissing, orderResources(reply).map(item => [item.id, held.has(item.id), item.name, item.icon])]);
}

/** Every part of a resource's balance as its breakdown draws it, signed as it adds to the net: needs negative. */
export const RESOURCE_PARTS = [
  { id: 'produced', label: 'Produced', measure: 'produced' }, { id: 'subjects', label: 'From subjects', measure: 'fromSubjects' },
  { id: 'imported', label: 'Imported', measure: 'imported' }, { id: 'projects', label: 'Needed by Special Projects', measure: 'projects' },
  { id: 'production', label: 'Needed by Production Lines', measure: 'production' }, { id: 'overlord', label: 'To the overlord', measure: 'toOverlord' },
  { id: 'exported', label: 'Export reserve', measure: 'exportReserve' }, { id: 'unidentified', label: 'Other', measure: 'other' },
] as const;

/** Each part's signed amount, in `RESOURCE_PARTS` order; the last is what the game's rounding leaves beside the net. */
export function resourceParts(balance: Balance) {
  const amounts = [balance.produced, balance.fromSubjects, balance.imported, -balance.projects, -balance.production,
    -balance.toOverlord, -balance.exportReserve, balance.net - (balance.supply - balance.demand)];
  return RESOURCE_PARTS.map((part, index) => ({ id: part.id, label: part.label, raw: amounts[index] }));
}

/** The breakdown's two groups: supply's parts, and demand's as positive needs. */
export function resourcePartGroups(balance: Balance) {
  const parts = resourceParts(balance);
  return [
    { title: 'Supply', total: balance.supply, values: parts.filter(part => ['produced', 'subjects', 'imported'].includes(part.id)) },
    { title: 'Demand', total: balance.demand, values: parts.filter(part => ['projects', 'production', 'overlord', 'exported'].includes(part.id))
      .map(part => ({ ...part, raw: -part.raw })) },
  ];
}

/** The recording's resources as chips choose them: a token draws a line per declaration. */
const resourceChoices = (resources: readonly ResourceInfo[]): ChoiceSource => ({
  lines: token => resources.filter(item => item.token === token).length,
  name: token => resourceName(resources.find(item => item.token === token) ?? { name: null, token, ordinal: 1 }),
  noun: 'resource',
});

/** The chosen resources this recording has, in the order chosen; `automatic` when none are chosen or recorded here. */
export const chosenResources = (resources: readonly ResourceInfo[], chosen: readonly string[], automatic: string | null) =>
  chosenShown(resourceChoices(resources), chosen, automatic);

/** Clicking one resource chip (the kit's `toggleChosen`): the resources to store, or why it cannot act. */
export const toggleResource = (resources: readonly ResourceInfo[], stored: readonly string[], shown: readonly string[], token: string): ChipAction =>
  toggleKit(resourceChoices(resources), stored, shown, token, Number.MAX_SAFE_INTEGER);

/** A deal list's sums over every deal, also those not on screen: what they deliver (null where any is not known) and
 * the widest bar of those known. */
export function dealAccounting(deals: readonly DealAt[]) {
  let widest = 0n;
  for (const deal of deals) {
    if (deal.deliveredRaw === null) continue;
    const raw = BigInt(deal.deliveredRaw), magnitude = raw < 0n ? -raw : raw;
    if (magnitude > widest) widest = magnitude;
  }
  return { total: dealsTotal(deals), widest };
}

/** The token of the resource with the longest bar over the campaign: the over-time view's choice until the user picks. */
export function automaticResource(resources: readonly ResourceInfo[], sizes: ResourceSizesReply | undefined): string | null {
  const longest = new Map((sizes?.sizes ?? []).map(size => [size.id, size.longestRaw]));
  return largestToken(resources.map(item => {
    const raw = longest.get(item.id);
    return { token: item.token, size: raw ? BigInt(raw) : 0n, recorded: raw != null };
  }));
}

/** A resource's name: content's, else its token; a repeated declaration of a token is numbered ("Steel (2)"), as the
 * game keeps it as another resource. */
export const resourceName = (item: Pick<ResourceInfo, 'name' | 'token' | 'ordinal'>) => `${contentTitle(item.name, item.token, 'Resource name unavailable')}${item.ordinal > 1 ? ` (${item.ordinal})` : ''}`;

/** A deal with its daily delivery from its live cell (`deal`, `delivered`), raw; null where the cell is not known. */
export type DealAt = ResourceDeal & { deliveredRaw: string | null };

/** `deal`'s delivery in `values`. */
export function dealAt(deal: ResourceDeal, values: LiveValues<number>): DealAt {
  const value = values.get(liveKey(deal.deal, 'delivered'));
  return { ...deal, deliveredRaw: value === undefined || !Number.isFinite(value) ? null : BigInt(Math.round(value)).toString() };
}

/** What `deals` deliver together; null where any is not known. */
export function dealsTotal(deals: readonly DealAt[]): bigint | null {
  let total = 0n;
  for (const deal of deals) {
    if (deal.deliveredRaw === null) return null;
    total += BigInt(deal.deliveredRaw);
  }
  return total;
}

/** One resource's deals at the hour, each way, largest first as the reply orders them. */
export interface Deals { imports: readonly DealAt[]; exports: readonly DealAt[] }

/** The deals of `resource` with their deliveries in `values`, or null where the deals are not recorded at the hour;
 * the imports, then the exports. */
export function dealsOf(reply: ResourceBalanceReply, resource: number, values: LiveValues<number>): Deals | null {
  if (reply.dealsMissing !== null) return null;
  const of = reply.deals.filter(deal => deal.resource === resource).map(deal => dealAt(deal, values));
  return { imports: of.filter(deal => deal.direction === 'import'), exports: of.filter(deal => deal.direction === 'export') };
}

/** Most deals listed per direction; the rest share one row. */
export const MAX_DEALS = 8;
/** Beyond this many resources over time, a tooltip never lists full breakdowns. */
export const FULL_BREAKDOWN_MAX = 2;

const factories = (count: number) => `${count} civilian ${count === 1 ? 'factory' : 'factories'}`;

/** The game-style breakdown of one resource at one hour: where it comes from, where it goes (with each trade
 * partner), what is available and what is needed, then the surplus or deficit. Each direction lists its `maxDeals`
 * largest deals and sums the rest in one row. `trades` is null when the deals were not recorded at the hour: then no
 * partner and no remainder is shown, only that they were not recorded. */
export function breakdownRows(balance: Balance, trades: Deals | null, decimals: number, partner: CountryLabel, maxDeals = MAX_DEALS): TooltipRow[] {
  const amount = (raw: bigint) => formatAmount(raw, decimals);
  const shown = (raw: bigint) => amount(raw < 0n ? -raw : raw) !== '0';
  const deal = (trade: DealAt): TooltipRow => {
    const who = partner(trade.partner);
    const convoys = trade.convoysRequired > 0 ? ` · ${trade.convoysAssigned}/${trade.convoysRequired} convoys` : '';
    return { label: `${who.name} · ${factories(trade.factories)}${convoys}`, countryFlag: who,
      value: trade.deliveredRaw === null ? '—' : amount(BigInt(trade.deliveredRaw)), indent: true, short: true };
  };
  const limit = Math.max(1, maxDeals);
  const deals = (pool: bigint, list: readonly DealAt[] | undefined): TooltipRow[] => {
    if (!list) return pool ? [{ label: 'Trade partners not recorded at this hour', tone: 'muted', indent: true }] : [];
    const more = list.slice(limit), all = dealsTotal(list), rest = all === null ? null : pool - all, moreTotal = dealsTotal(more);
    return [...list.slice(0, limit).map(deal),
      ...(more.length ? [{ label: `${more.length} more ${more.length === 1 ? 'deal' : 'deals'}`, value: moreTotal === null ? '—' : amount(moreTotal), tone: 'muted' as const, indent: true }] : []),
      ...(rest !== null && shown(rest) ? [{ label: 'Not from a listed deal', value: formatSigned(rest, decimals), tone: 'muted' as const, indent: true }] : [])];
  };
  const added = (raw: bigint) => formatSigned(raw, decimals), taken = (raw: bigint) => formatSigned(-raw, decimals);
  const rows: TooltipRow[] = [{ label: 'Produced', value: amount(balance.produced) }];
  if (balance.fromSubjects) rows.push({ label: 'From subjects', value: added(balance.fromSubjects) });
  rows.push({ label: 'Imported', value: added(balance.imported) });
  rows.push(...deals(balance.imported, trades?.imports));
  if (balance.toOverlord) rows.push({ label: 'To the overlord', value: taken(balance.toOverlord) });
  rows.push({ label: 'Exported (trade law)', value: taken(balance.exportReserve) });
  rows.push(...deals(balance.exported, trades?.exports));
  const unsold = balance.exportReserve - balance.exported;
  if (shown(unsold)) rows.push({ label: unsold > 0n ? 'Set aside, not bought' : 'Bought beyond the reserve', value: amount(unsold < 0n ? -unsold : unsold), tone: 'muted', indent: true });
  rows.push({ label: 'Available', value: amount(balance.available), strong: true, divider: true });
  if (balance.projects) rows.push({ label: 'Needed by special projects', value: taken(balance.projects) });
  rows.push({ label: 'Needed by production lines', value: taken(balance.production) });
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

function dealCounts(deals: readonly (Deals | null)[]): number[] {
  const most = Math.max(1, ...deals.map(each => each ? Math.max(each.imports.length, each.exports.length) : 0));
  return Array.from({ length: Math.min(MAX_DEALS, most) }, (_, index) => Math.min(MAX_DEALS, most) - index);
}

/** One resource's tooltip at an hour, fitted to a window `height` pixels tall: its breakdown with as many deals each
 * way as fit, else its supply, demand and net with the net always shown. */
export function resourceTooltip(head: Pick<TooltipContent, 'title' | 'icon' | 'subtitle' | 'note'>, balance: Balance, deals: Deals | null, decimals: number,
  partner: CountryLabel, height: number): TooltipContent {
  function* forms(): Generator<TooltipContent> {
    for (const count of dealCounts([deals])) yield { ...head, rows: breakdownRows(balance, deals, decimals, partner, count) };
    const summary = summaryRows(balance, decimals);
    yield { ...head, rows: [...summary.slice(0, 2), { ...summary[2], divider: true, strong: true }] };
  }
  return fitTooltip(forms(), height, { keep: 1 });
}

/** A resource drawn over time, at the hour a tooltip shows. */
export interface LineAt { resource: ResourceInfo; color: string; balance: Balance | null; deals: Deals | null; missing: string | null }

/** The over-time tooltip for the chosen resources at an hour, fitted so every resource's net shows. */
export function overTimeTooltip(title: string, lines: readonly LineAt[], decimals: number, partner: CountryLabel, height: number): TooltipContent {
  const head = (line: LineAt, index: number): TooltipRow => ({ label: resourceName(line.resource), swatch: line.color, strong: true, divider: index > 0 });
  const missing = (line: LineAt): TooltipRow => ({ label: line.missing ?? 'Not recorded', tone: 'muted' });
  function* forms(): Generator<TooltipContent> {
    if (lines.length <= FULL_BREAKDOWN_MAX) {
      for (const count of dealCounts(lines.map(line => line.deals))) {
        yield { title, rows: lines.flatMap((line, index) => [head(line, index), ...(line.balance ? breakdownRows(line.balance, line.deals, decimals, partner, count) : [missing(line)])]) };
      }
    }
    yield { title, rows: lines.flatMap((line, index) => [head(line, index), ...(line.balance ? summaryRows(line.balance, decimals) : [missing(line)])]) };
    yield { title,
      rows: lines.map(line => ({ label: resourceName(line.resource), swatch: line.color, short: true, ...(line.balance ? netValue(line.balance, decimals) : { value: 'Not recorded', tone: 'muted' as const }) })) };
  }
  return fitTooltip(forms(), height, { more: hidden => `${hidden} more` });
}
