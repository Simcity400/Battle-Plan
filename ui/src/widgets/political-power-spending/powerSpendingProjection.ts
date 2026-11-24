import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionField } from '../../dashboard/options';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { artState } from '../../families/table/identityCell.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY, tableFamilyOptions, type TableBase } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, partialValue, zeroValue, type Value } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { BreakdownPart, PowerSpendingBreakdownReply, SpendCurrency } from '../../generated/dashboard';
import { spendingCategoryArt } from './spendingCategoryArt.ts';

const UNIT: Readonly<Record<SpendCurrency, string>> = { Political: 'PP', Command: 'CP' };

export function powerSpendingColumns(currency: SpendCurrency): TableColumnSpec[] {
  return [
    { key: 'picture', label: '', as: 'plain', cellKind: 'kind', offered: OFFERED.text, priority: 0, sortable: false, hideable: false, pinned: true },
    { key: 'item', label: 'Spent on', as: 'plain', cellKind: 'item', offered: OFFERED.text, priority: 0, text: true, hideable: false, pinned: true, widest: ['Organisation upkeep'] },
    { key: 'spent', label: 'Spent', unit: UNIT[currency], as: 'faint', barEligible: true, offered: OFFERED.number, priority: 1, numeric: true, widest: ['99,999'] },
    { key: 'share', label: 'Share', as: 'plain', format: 'share', barEligible: false, offered: OFFERED.number, priority: 2, numeric: true, widest: ['100%', '<0.1%'] },
    { key: 'last', label: 'Last', as: 'plain', cellKind: 'date', offered: OFFERED.number, priority: 3, numeric: true, widest: ['28 Aug'] },
  ];
}

export const POWER_SPENDING_BASE: TableBase = { density: 'normal', barsBehind: 'all', showRank: false, sort: { column: 'spent', direction: 'descending' } };

export function powerSpendingOptions(currency: SpendCurrency): readonly OptionField[] {
  return tableFamilyOptions(powerSpendingColumns(currency), POWER_SPENDING_BASE);
}

export interface PowerSpendingProjectionOptions {
  readonly currency: SpendCurrency;
}

function validate(raw: unknown): PowerSpendingBreakdownReply {
  const reply = raw as Partial<PowerSpendingBreakdownReply> | null;
  if (!reply || typeof reply.hour !== 'number' || typeof reply.through !== 'number' || !reply.breakdown || !Array.isArray(reply.breakdown.parts)) {
    throw new Error('Invalid power spending breakdown');
  }
  return reply as PowerSpendingBreakdownReply;
}

function amountOf(part: Pick<BreakdownPart, 'raw' | 'state' | 'reason'>, what: string): Value {
  if (part.raw === null || part.state === 'notRecorded') return notRecorded(part.reason ?? `${what} was not recorded`);
  const raw = BigInt(part.raw);
  if (part.state === 'partial' || part.state === 'atLeast') return partialValue(raw, part.reason ?? 'Some payments were not recorded', { atLeast: part.state === 'atLeast' });
  return raw === 0n ? zeroValue(0n) : knownValue(raw);
}

function picture(readView: BoundedReadView, part: BreakdownPart): TableCell {
  const handle = spendingCategoryArt(part.key);
  return { value: knownValue(part.name), artShape: 'square', artName: part.name, ...(handle ? { iconSlot: true, ...artState(readView, handle) } : {}) };
}

function row(readView: BoundedReadView, part: BreakdownPart, decimals: number, top: boolean, keyPrefix: string): TableRow {
  const key = `${keyPrefix}${part.key}`;
  const format = { decimals, maxDecimals: 0 };
  return {
    key,
    name: part.name,
    cells: {
      ...(top ? { picture: picture(readView, part) } : {}),
      item: part.nameMissing ? { value: notRecorded(part.nameMissing), missingFigure: part.name } : { value: knownValue(part.name) },
      spent: { value: amountOf(part, part.name), format },
      share: top
        ? { value: part.share === null ? notRecorded(part.reason ?? 'No share where the whole is a lower bound') : part.share === 0 ? zeroValue(0) : knownValue(part.share) }
        : { value: notApplicable('A share is shown for each category'), blank: true },
      last: { value: part.last === null ? notRecorded('Date not recorded') : knownValue(part.last) },
    },
    ...(part.reason ? { tooltipNote: part.reason } : {}),
    ...(part.children.length ? { children: part.children.map(child => row(readView, child, decimals, false, `${key}/`)) } : {}),
  };
}

function summary(reply: PowerSpendingBreakdownReply, currency: SpendCurrency): { readonly text: string; readonly partial: string | null } {
  const decimals = reply.breakdown.decimals;
  const amount = (raw: string) => formatScalar(BigInt(raw), { decimals, maxDecimals: 0 });
  const total = reply.breakdown.total;
  const parts = [`${total.raw === null ? 'Not recorded' : amount(total.raw)} spent`];
  if (reply.gainedRaw !== null) parts.push(`${amount(reply.gainedRaw)} gained`);
  if (currency === 'Command' && reply.lostAtCapRaw !== null && BigInt(reply.lostAtCapRaw) > 0n) parts.push(`${amount(reply.lostAtCapRaw)} lost at the cap`);
  const reasons = [
    ...(total.state !== 'known' && total.reason ? [total.reason] : []),
    ...(reply.gainedReason ? [reply.gainedReason] : reply.gainedRaw === null ? ['Gains were not recorded'] : []),
  ];
  return { text: parts.join(' · '), partial: reasons.length ? reasons.join('; ') : null };
}

function spendingProjection(kind: 'political-power-spending' | 'command-power-spending'): TableProjection<PowerSpendingProjectionOptions> {
  return registerTableProjection({ kind, family: 'table', project: projectSpending });
}

function projectSpending(readView: BoundedReadView, options: PowerSpendingProjectionOptions): TableModel {
  const hour = readView.hour;
  const columns = powerSpendingColumns(options.currency);
  const refused = readView.refused('main');
  if (refused !== undefined) return { columns, rows: [], hour, refused };
  const reply = readView.panel<PowerSpendingBreakdownReply>('main');
  if (!reply) return { columns, rows: [], hour };
  const rows = reply.breakdown.parts.map(part => row(readView, part, reply.breakdown.decimals, true, ''));
  const said = summary(reply, options.currency);
  const base: TableModel = {
    columns,
    rows,
    hour,
    titleSummary: said.text,
    titleSummaryPartial: said.partial,
    ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
  };
  if (rows.length) return base;
  if (reply.breakdown.total.state === 'notRecorded') {
    return { ...base, empty: 'Not recorded at this time.', emptyDetail: reply.breakdown.total.reason ?? 'Spending was not recorded.', emptyKind: 'unavailable' };
  }
  if (reply.breakdown.total.state !== 'known') {
    return { ...base, empty: 'No spending recorded', emptyDetail: reply.breakdown.total.reason ?? 'Some hours were not recorded.', emptyKind: 'unavailable' };
  }
  return { ...base, empty: 'Nothing spent yet' };
}

export const politicalSpendingProjection = spendingProjection('political-power-spending');
export const commandSpendingProjection = spendingProjection('command-power-spending');

function sources(ctx: FamilyPlanContext, kind: string, currency: SpendCurrency): SourcePlan {
  const tag = ctx.countryTag;
  return {
    owner: `${kind}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${tag ?? ''}`,
    entries: tag ? [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: { mode: 'population', form: 'parts' },
        product: 'power-spending-breakdown',
        followQuery: null,
        params: { branchId: ctx.branch.id, country: countryFromKey(tag), currency, from: ctx.branch.firstHour },
        hourParam: 'hour',
        provenInterval: (data, at) => {
          const reply = data as PowerSpendingBreakdownReply;
          return reply.hour <= at && at <= reply.through ? [reply.hour, reply.through] : [at, at];
        },
        validate,
      },
    }] : [],
  };
}

export function powerSpendingDeclaration(kind: 'political-power-spending' | 'command-power-spending', currency: SpendCurrency, label: string):
  TableWidgetDeclaration<PowerSpendingProjectionOptions> {
  const subject = label.toLowerCase();
  return {
    family: 'table',
    kind,
    label,
    subject,
    emptyCampaign: `Load a campaign to see ${subject}.`,
    icon: 'chart-line',
    countries: { mode: 'single', choose: `Choose a country to see its ${subject}.` },
    sources: ctx => sources(ctx, kind, currency),
    projection: kind === 'political-power-spending' ? politicalSpendingProjection : commandSpendingProjection,
    columns: powerSpendingColumns(currency),
    settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: POWER_SPENDING_BASE },
    projectionOptions: () => ({ currency }),
    keyByBranch: true,
  };
}

export const politicalSpendingDeclaration = powerSpendingDeclaration('political-power-spending', 'Political', 'Political power spending');
export const commandSpendingDeclaration = powerSpendingDeclaration('command-power-spending', 'Command', 'Command power spending');
