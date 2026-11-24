import { countryFromKey, countryKey, isSlotKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsValueField } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { nextMembershipChange, nextRangesChange, type HourRanges } from '../../families/holders.ts';
import { knownValue, notRecorded, type Value } from '../../families/value.ts';
import type { LendLeaseBalanceReply, LendLeaseMeasure, LendLeaseRow } from '../../generated/dashboard';
import { MEASURES } from './definition.ts';

export const LEND_LEASE_BALANCE = 'lend-lease-balance';
const SCALE = 100000;
const NOT_IN_HISTORIES = 'This country is in neither the recorded lend-lease histories nor the war rosters at this date.';

export interface LendLeaseBalanceOptions {
  readonly measure: LendLeaseMeasure;
  readonly chosen: readonly string[] | null;
  readonly membership: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved: HourRanges;
}

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const raw = (value: unknown) => typeof value === 'string' && /^-?\d+$/.test(value);

export function validateLendLeaseBalance(value: unknown): LendLeaseBalanceReply {
  if (!object(value) || value.scale !== SCALE || !(value.holder === null || typeof value.holder === 'string')
    || !(value.missing === null || typeof value.missing === 'string') || !Array.isArray(value.warnings)
    || !Array.isArray(value.rows) || !value.rows.every(row => object(row) && Number.isSafeInteger(row.slot)
      && (row.tag === null || typeof row.tag === 'string') && Array.isArray(row.values) && row.values.length === 4 && row.values.every(raw)))
    throw new Error('The lend-lease balance reply is malformed.');
  return value as unknown as LendLeaseBalanceReply;
}

export const measureOf = (option: unknown): LendLeaseMeasure => option === 'fuel' ? 'fuel' : 'industry';

function amount(text: string): { readonly field: BarsValueField; readonly numeric: number } {
  const value = BigInt(text);
  const hundredths = value / 1000n;
  const field: BarsValueField = value !== 0n && hundredths === 0n
    ? { value: knownValue(value), format: { decimals: 5 } }
    : { value: knownValue(hundredths), format: { decimals: 2 } };
  return { field, numeric: Number(value > 0n ? value : 0n) / SCALE };
}

const inRanges = (ranges: HourRanges | undefined, hour: number) => !!ranges?.some(([first, last]) => first <= hour && hour <= last);

function rowKeys(row: LendLeaseRow): readonly string[] {
  const slot = countryKey(row.slot);
  return row.tag === null ? [slot] : [slot, row.tag];
}

function sideHeaders(measure: LendLeaseMeasure): { readonly left: string; readonly right: string } {
  const label = MEASURES.find(item => item.value === measure)?.label ?? 'Industry';
  return { left: `${label} received`, right: `${label} sent` };
}

function dataRow(readView: BoundedReadView, row: LendLeaseRow, measure: LendLeaseMeasure): BarsDataRow {
  const offset = measure === 'industry' ? 0 : 1;
  const sent = amount(row.values[offset]!);
  const received = amount(row.values[2 + offset]!);
  const key = countryKey(row.slot);
  if (row.tag === null) {
    return {
      kind: 'data', key, label: 'Name not recorded', labelMissing: true,
      left: { value: received.field, numeric: received.numeric },
      right: { value: sent.field, numeric: sent.numeric },
    };
  }
  const pres = countryRowPresentation(readView, row.tag);
  return {
    kind: 'data', key, subject: row.tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    left: { value: received.field, numeric: received.numeric },
    right: { value: sent.field, numeric: sent.numeric },
  };
}

function missingRow(readView: BoundedReadView, key: string, reason: string): BarsDataRow {
  const value: Value = notRecorded(reason);
  if (isSlotKey(key)) {
    return { kind: 'data', key, label: 'Name not recorded', labelMissing: true, left: { value: { value }, numeric: null }, right: { value: { value }, numeric: null } };
  }
  const pres = countryRowPresentation(readView, key);
  return {
    kind: 'data', key, subject: key, label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    left: { value: { value }, numeric: null },
    right: { value: { value }, numeric: null },
  };
}

export const lendLeaseBalanceProjection: BarsProjection<LendLeaseBalanceOptions> = registerFamilyProjection({
  kind: LEND_LEASE_BALANCE,
  family: 'bars',
  nextChange(readView, options) {
    return Math.min(nextMembershipChange(options.membership, readView.hour), nextRangesChange(options.unobserved, readView.hour));
  },
  project(readView, options): BarsModel {
    const hour = readView.hour;
    const headers = sideHeaders(options.measure);
    const base = { form: 'two-sided' as const, hour, animateOrder: true, leftHeader: headers.left, rightHeader: headers.right };
    if (options.chosen && options.unobserved.some(([first, last]) => first <= hour && hour <= last)) {
      return { ...base, rows: [], empty: 'This group’s members were not recorded at this date.', emptyKind: 'unavailable' };
    }
    const refused = readView.refused('main');
    if (refused) return { ...base, rows: [], empty: refused, emptyKind: 'unavailable' };
    const panel = readView.panel('main');
    if (panel === undefined) return { ...base, rows: [] };
    const reply = validateLendLeaseBalance(panel);
    if (reply.missing !== null) return { ...base, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    const wanted = options.chosen?.filter(key => !options.membership || inRanges(options.membership[key], hour)) ?? null;
    const rows: BarsDataRow[] = [];
    const answered = new Set<string>();
    for (const row of reply.rows) {
      const keys = rowKeys(row);
      if (wanted && !keys.some(key => wanted.includes(key))) continue;
      for (const key of keys) answered.add(key);
      rows.push(dataRow(readView, row, options.measure));
    }
    if (wanted) {
      for (const key of wanted) if (!answered.has(key)) rows.push(missingRow(readView, key, NOT_IN_HISTORIES));
    }
    if (!rows.length) {
      return { ...base, rows, empty: wanted ? 'No country of this group at this date.' : `No ${options.measure === 'fuel' ? 'fuel' : 'industry'} lend-lease at this date.`, emptyKind: 'empty', warnings: reply.warnings };
    }
    return { ...base, rows, warnings: reply.warnings };
  },
});

function panelQuery(ctx: FamilyPlanContext): Record<string, unknown> {
  const measure = measureOf(ctx.options.measure);
  const top = ctx.countries.top;
  return {
    branchId: ctx.branch.id,
    rank: measure,
    ...(top != null ? { top } : {}),
    ...(!ctx.countries.all && top == null ? { countries: ctx.countries.tags.map(countryFromKey) } : {}),
  };
}

function lendLeaseSources(ctx: FamilyPlanContext): SourcePlan {
  const params = panelQuery(ctx);
  const top = ctx.countries.top;
  return {
    owner: `${LEND_LEASE_BALANCE}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: top != null
          ? { mode: 'population', form: 'top', limit: top }
          : ctx.countries.all ? { mode: 'population', form: 'all' } : { mode: 'keyed', keys: ctx.countries.tags },
        product: LEND_LEASE_BALANCE,
        followQuery: { panel: LEND_LEASE_BALANCE, ...params },
        params,
        validate: validateLendLeaseBalance,
      },
    }],
  };
}

export const lendLeaseBalanceDeclaration: BarsWidgetDeclaration<LendLeaseBalanceOptions> = {
  family: 'bars',
  kind: LEND_LEASE_BALANCE,
  label: 'Lend-lease received and sent',
  subject: 'lend-lease',
  emptyCampaign: 'Load a campaign to see lend-lease.',
  icon: 'chart-bars',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: lendLeaseSources,
  projection: lendLeaseBalanceProjection,
  projectionOptions: ctx => {
    const keyed = !ctx.countries.all && ctx.countries.top == null;
    return {
      measure: measureOf(ctx.options.measure),
      chosen: keyed ? ctx.countries.tags : null,
      membership: keyed ? ctx.countries.membership : null,
      unobserved: keyed ? ctx.countries.unobserved : [],
    };
  },
  jumpToDate: true,
  keyByBranch: true,
};
