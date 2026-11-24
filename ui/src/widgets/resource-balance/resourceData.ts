import { drawGapSummary, drawLineSummary, summaryExtent } from '../../charts/chartSummary.ts';
import type { ChartSummary } from '../../charts/chartSummary';
import { SECONDARY_DASH } from '../../charts/palette.ts';
import type { ChartLayer } from '../../charts/TimeChart';
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


export const RESOURCE_BALANCE = 'resource-balance';
export const RESOURCE_SIZES = 'resource-sizes';

const RAW = /^-?\d{1,20}$/;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;

export interface ResourcePools {
  id: number;
  producedRaw: string; subjectsRaw: string; importedRaw: string; toExportRaw: string; exportedRaw: string;
  availableRaw: string; afterProjectsRaw: string; productionRaw: string;
}

export const POOL_CELLS = [['produced', 'producedRaw'], ['subjects', 'subjectsRaw'], ['imported', 'importedRaw'], ['toExport', 'toExportRaw'],
  ['exported', 'exportedRaw'], ['available', 'availableRaw'], ['afterProjects', 'afterProjectsRaw'], ['production', 'productionRaw']] as const;

export function poolsAt(values: LiveValues<number>, id: number): ResourcePools | null {
  const pools: Record<string, unknown> = { id };
  for (const [column, field] of POOL_CELLS) {
    const value = values.get(liveKey(String(id), column));
    if (value === undefined || !Number.isFinite(value)) return null;
    pools[field] = BigInt(Math.round(value)).toString();
  }
  return pools as unknown as ResourcePools;
}

export const poolsMissing = (values: LiveValues<number>, id: number) => {
  for (const [column] of POOL_CELLS) if (values.get(liveKey(String(id), column)) === undefined) return values.missing(liveKey(String(id), column));
  return 'Not recorded at this time';
};

export const poolsOf = (reply: ResourceBalanceReply, values: LiveValues<number>): ResourcePools[] =>
  reply.pools.flatMap(id => { const pools = poolsAt(values, id); return pools ? [pools] : []; });

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

export function barParts(balance: Balance): { supply: [string, bigint][]; demand: [string, bigint][] } {
  return { supply: [['produced', balance.produced], ['subjects', balance.fromSubjects], ['imported', balance.imported]],
    demand: [['needed', balance.projects + balance.production], ['overlord', balance.toOverlord], ['exported', balance.exportReserve]] };
}

export const barLength = (parts: readonly (readonly [string, bigint])[]) => parts.reduce((sum, [, raw]) => sum + (raw < 0n ? -raw : raw), 0n);

export function momentScale(pools: readonly ResourcePools[]): bigint {
  let largest = 0n;
  for (const pool of pools) {
    const parts = barParts(balanceOf(pool));
    for (const length of [barLength(parts.supply), barLength(parts.demand)]) if (length > largest) largest = length;
  }
  return largest;
}

export function campaignScale(sizes: ResourceSizesReply | undefined): bigint {
  let largest = 0n;
  for (const size of sizes?.sizes ?? []) if (size.longestRaw !== null && BigInt(size.longestRaw) > largest) largest = BigInt(size.longestRaw);
  return largest;
}

export const partWidth = (raw: bigint, scale: bigint) => scale > 0n ? `${Number((raw < 0n ? -raw : raw) * 100000n / scale) / 1000}%` : '0%';

export type RowOrder = 'game' | 'shortfall' | 'volume';

export function orderResources(reply: ResourceBalanceReply): ResourceInfo[] {
  const byId = new Map(reply.resources.map(item => [item.id, item]));
  return reply.shown.flatMap(id => byId.get(id) ?? []);
}

export function barStructure(reply: ResourceBalanceReply): string {
  const held = new Set(reply.pools);
  return JSON.stringify([reply.poolsMissing, reply.dealsMissing, orderResources(reply).map(item => [item.id, held.has(item.id), item.name, item.icon])]);
}

export const RESOURCE_PARTS = [
  { id: 'produced', label: 'Produced', measure: 'produced' }, { id: 'subjects', label: 'From subjects', measure: 'fromSubjects' },
  { id: 'imported', label: 'Imported', measure: 'imported' }, { id: 'projects', label: 'Needed by Special Projects', measure: 'projects' },
  { id: 'production', label: 'Needed by Production Lines', measure: 'production' }, { id: 'overlord', label: 'To the overlord', measure: 'toOverlord' },
  { id: 'exported', label: 'Export reserve', measure: 'exportReserve' }, { id: 'unidentified', label: 'Other', measure: 'other' },
] as const;

export function resourceParts(balance: Balance) {
  const amounts = [balance.produced, balance.fromSubjects, balance.imported, -balance.projects, -balance.production,
    -balance.toOverlord, -balance.exportReserve, balance.net - (balance.supply - balance.demand)];
  return RESOURCE_PARTS.map((part, index) => ({ id: part.id, label: part.label, raw: amounts[index] }));
}

export function resourcePartGroups(balance: Balance) {
  const parts = resourceParts(balance);
  return [
    { title: 'Supply', total: balance.supply, values: parts.filter(part => ['produced', 'subjects', 'imported'].includes(part.id)) },
    { title: 'Demand', total: balance.demand, values: parts.filter(part => ['projects', 'production', 'overlord', 'exported'].includes(part.id))
      .map(part => ({ ...part, raw: -part.raw })) },
  ];
}

const resourceChoices = (resources: readonly ResourceInfo[]): ChoiceSource => ({
  lines: token => resources.filter(item => item.token === token).length,
  name: token => resourceName(resources.find(item => item.token === token) ?? { name: null, token, ordinal: 1 }),
  noun: 'resource',
});

export const chosenResources = (resources: readonly ResourceInfo[], chosen: readonly string[], automatic: string | null) =>
  chosenShown(resourceChoices(resources), chosen, automatic);

export const toggleResource = (resources: readonly ResourceInfo[], stored: readonly string[], shown: readonly string[], token: string): ChipAction =>
  toggleKit(resourceChoices(resources), stored, shown, token, Number.MAX_SAFE_INTEGER);

export function dealAccounting(deals: readonly DealAt[]) {
  let widest = 0n;
  for (const deal of deals) {
    if (deal.deliveredRaw === null) continue;
    const raw = BigInt(deal.deliveredRaw), magnitude = raw < 0n ? -raw : raw;
    if (magnitude > widest) widest = magnitude;
  }
  return { total: dealsTotal(deals), widest };
}

export function automaticResource(resources: readonly ResourceInfo[], sizes: ResourceSizesReply | undefined): string | null {
  const longest = new Map((sizes?.sizes ?? []).map(size => [size.id, size.longestRaw]));
  return largestToken(resources.map(item => {
    const raw = longest.get(item.id);
    return { token: item.token, size: raw ? BigInt(raw) : 0n, recorded: raw != null };
  }));
}

export const resourceName = (item: Pick<ResourceInfo, 'name' | 'token' | 'ordinal'>) => `${contentTitle(item.name, item.token, 'Resource name unavailable')}${item.ordinal > 1 ? ` (${item.ordinal})` : ''}`;

export type DealAt = ResourceDeal & { deliveredRaw: string | null };

export function dealAt(deal: ResourceDeal, values: LiveValues<number>): DealAt {
  const value = values.get(liveKey(deal.deal, 'delivered'));
  return { ...deal, deliveredRaw: value === undefined || !Number.isFinite(value) ? null : BigInt(Math.round(value)).toString() };
}

export function dealsTotal(deals: readonly DealAt[]): bigint | null {
  let total = 0n;
  for (const deal of deals) {
    if (deal.deliveredRaw === null) return null;
    total += BigInt(deal.deliveredRaw);
  }
  return total;
}

export interface Deals { imports: readonly DealAt[]; exports: readonly DealAt[] }

export function dealsOf(reply: ResourceBalanceReply, resource: number, values: LiveValues<number>): Deals | null {
  if (reply.dealsMissing !== null) return null;
  const of = reply.deals.filter(deal => deal.resource === resource).map(deal => dealAt(deal, values));
  return { imports: of.filter(deal => deal.direction === 'import'), exports: of.filter(deal => deal.direction === 'export') };
}

export const MAX_DEALS = 8;
export const FULL_BREAKDOWN_MAX = 2;

const factories = (count: number) => `${count} civilian ${count === 1 ? 'factory' : 'factories'}`;

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

export function netValue(balance: Balance, decimals: number): Pick<TooltipRow, 'value' | 'tone'> {
  return { value: formatSigned(balance.net, decimals), tone: balance.net < 0n ? 'negative' : balance.net > 0n ? 'positive' : undefined };
}

export function summaryRows(balance: Balance, decimals: number): TooltipRow[] {
  return [{ label: 'Supply', value: formatAmount(balance.supply, decimals) }, { label: 'Demand', value: formatAmount(balance.demand, decimals) },
    { label: balance.net < 0n ? 'Deficit' : 'Surplus', ...netValue(balance, decimals) }];
}

function dealCounts(deals: readonly (Deals | null)[]): number[] {
  const most = Math.max(1, ...deals.map(each => each ? Math.max(each.imports.length, each.exports.length) : 0));
  return Array.from({ length: Math.min(MAX_DEALS, most) }, (_, index) => Math.min(MAX_DEALS, most) - index);
}

export function resourceTooltip(head: Pick<TooltipContent, 'title' | 'icon' | 'subtitle' | 'note'>, balance: Balance, deals: Deals | null, decimals: number,
  partner: CountryLabel, height: number): TooltipContent {
  function* forms(): Generator<TooltipContent> {
    for (const count of dealCounts([deals])) yield { ...head, rows: breakdownRows(balance, deals, decimals, partner, count) };
    const summary = summaryRows(balance, decimals);
    yield { ...head, rows: [...summary.slice(0, 2), { ...summary[2], divider: true, strong: true }] };
  }
  return fitTooltip(forms(), height, { keep: 1 });
}

export interface LineAt { resource: ResourceInfo; color: string; balance: Balance | null; deals: Deals | null; missing: string | null }

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

export const describeResource = (resource: ResourceInfo, balance: Balance | null, decimals: number) => balance
  ? `${resourceName(resource)}: supply ${formatAmount(balance.supply, decimals)}, demand ${formatAmount(balance.demand, decimals)}, ${balance.net < 0n ? 'deficit' : 'surplus'} ${formatSigned(balance.net, decimals)}`
  : `${resourceName(resource)}: not recorded at this time`;

export const segmentLook = (raw: bigint, colour: string | undefined) => raw < 0n
  ? { background: 'transparent', 'box-shadow': `inset 0 0 0 1.5px ${colour}` } : { background: colour ?? '', 'box-shadow': 'none' };

export const legendFlags = (balances: readonly (Balance | null)[]) =>
  `${balances.some(balance => balance && balance.fromSubjects !== 0n)}|${balances.some(balance => balance && balance.toOverlord !== 0n)}`;

export interface ResourceTimeLine { id: string; color: string }

export function resourceTimeLayers(lines: readonly ResourceTimeLine[], shade: boolean,
  supplyOf: (id: string) => ChartSummary | undefined, demandOf: (id: string) => ChartSummary | undefined,
  untilOf: () => number | null): ChartLayer[] {
  return [
    ...shade && lines.length === 1 ? [{ draw: (ctx, frame, chartTokens, state) => {
      const id = lines[0].id, high = supplyOf(id), low = demandOf(id);
      if (high && low) drawGapSummary(ctx, frame, chartTokens, high, low, chartTokens.positive, chartTokens.error, state.playhead);
    } } satisfies ChartLayer] : [],
    ...lines.flatMap(line => [
      {
        draw: (ctx, frame, chartTokens, state) => {
          const drawn = supplyOf(line.id);
          if (drawn) drawLineSummary(ctx, frame, chartTokens, drawn, line.color, false, state.playhead);
        },
        extent: x => { const drawn = supplyOf(line.id); return drawn ? summaryExtent(drawn, x, untilOf()) : null; },
      },
      {
        draw: (ctx, frame, chartTokens, state) => {
          const drawn = demandOf(line.id);
          if (drawn) drawLineSummary(ctx, frame, chartTokens, drawn, line.color, false, state.playhead, { dash: SECONDARY_DASH, endDot: false });
        },
        extent: x => { const drawn = demandOf(line.id); return drawn ? summaryExtent(drawn, x, untilOf()) : null; },
      },
    ] satisfies ChartLayer[]),
  ];
}
