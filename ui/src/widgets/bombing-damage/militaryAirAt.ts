import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { airCauseName } from '../air-kills-losses/lossCauses.ts';
import { countryRowPresentation, formatBarsValueField, type BarsDataRow, type BarsLegendItem, type BarsModel, type BarsReadout, type BarsSegment } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { knownValue, notRecorded, partialValue, type Value } from '../../families/value.ts';
import { formatCompactScalar } from '../../families/valueFormat.ts';
import type { MilitaryAirAtReply, MilitaryAirAtRow, MilitaryAirFamily, MilitaryAirMetric } from '../../generated/dashboard';

export const MILITARY_AIR_AT = 'military-air-at';
const PARTIAL = 'Some events since the totals’ start may be missing.';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const whole = (value: unknown) => typeof value === 'string' && /^-?\d+$/.test(value);
const text = (value: unknown) => value === null || typeof value === 'string';

export function validateMilitaryAirAt(value: unknown): MilitaryAirAtReply {
  if (!object(value) || !Number.isSafeInteger(value.hour) || !Number.isSafeInteger(value.validUntil)
    || !(value.countedFrom === null || Number.isSafeInteger(value.countedFrom)) || !text(value.missing) || !text(value.rankMissing)
    || !Array.isArray(value.unattributed) || value.unattributed.length !== 2 || !value.unattributed.every(whole)
    || !Array.isArray(value.warnings) || !Array.isArray(value.rows) || !value.rows.every(row => object(row) && typeof row.tag === 'string'
      && typeof row.identity === 'string' && Number.isSafeInteger(row.segmentFirstHour) && whole(row.dealt) && whole(row.taken) && typeof row.complete === 'boolean'
      && Array.isArray(row.causes) && row.causes.every(cause => object(cause) && Number.isSafeInteger(cause.cause) && whole(cause.taken))))
    throw new Error('The military air reply is malformed.');
  return value as unknown as MilitaryAirAtReply;
}

export interface MilitaryAirAtOptions {
  readonly family: MilitaryAirFamily;
  readonly scale: number;
  readonly digits: number;
  readonly leftHeader: string;
  readonly rightHeader: string;
  readonly unit: string;
  readonly ratio: boolean;
  readonly chosen: readonly string[] | null;
  readonly paired?: { readonly solid: string; readonly outline: string };
}

export function militaryAirParams(ctx: FamilyPlanContext, family: MilitaryAirFamily, metric: MilitaryAirMetric, filters: { readonly regions?: readonly number[]; readonly causes?: readonly number[] } = {}): Record<string, unknown> {
  const top = ctx.countries.top;
  return {
    branchId: ctx.branch.id,
    family,
    regions: [...(filters.regions ?? [])],
    causes: [...(filters.causes ?? [])],
    ...(ctx.staticSpan && ctx.widget.time.mode === 'range' ? { fromHour: ctx.staticSpan[0] } : {}),
    metric,
    ...(top != null ? { top } : {}),
    ...(!ctx.countries.all && top == null ? { countries: ctx.countries.tags.map(countryFromKey) } : {}),
  };
}

export function militaryAirSources(owner: string, ctx: FamilyPlanContext, params: Record<string, unknown>): SourcePlan {
  const top = ctx.countries.top;
  return {
    owner: `${owner}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: top != null
          ? { mode: 'population', form: 'top', limit: top }
          : ctx.countries.all ? { mode: 'population', form: 'all' } : { mode: 'keyed', keys: ctx.countries.tags },
        product: MILITARY_AIR_AT,
        followQuery: null,
        params,
        dynamicKeys: ['hour'],
        provenInterval: (data, hour) => {
          const reply = data as MilitaryAirAtReply;
          return reply.hour <= hour && hour <= reply.validUntil ? [reply.hour, reply.validUntil] : null;
        },
        validate: validateMilitaryAirAt,
      },
    }],
  };
}

function amount(raw: string, complete: boolean, options: MilitaryAirAtOptions): { readonly value: Value; readonly format: { readonly decimals: number }; readonly numeric: number } {
  const big = BigInt(raw);
  const hundredths = big / BigInt(options.scale / 100);
  const [shown, format] = big !== 0n && hundredths === 0n ? [big, { decimals: options.digits }] : [hundredths, { decimals: 2 }];
  const value = complete ? knownValue(shown) : partialValue(shown, PARTIAL, { atLeast: true });
  return { value, format, numeric: Math.max(0, Number(big)) / options.scale };
}

function ratioFigure(row: MilitaryAirAtRow): string | null {
  const dealt = BigInt(row.dealt), taken = BigInt(row.taken);
  if (!row.complete || dealt < 0n || taken <= 0n) return null;
  const hundredths = (dealt * 100n + taken / 2n) / taken;
  return `${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')}`;
}

function wholeCompact(raw: string, options: MilitaryAirAtOptions): string {
  const scale = BigInt(options.scale);
  const whole = (BigInt(raw) + scale / 2n) / scale;
  return formatCompactScalar(whole, { decimals: 0 }).text;
}

function pairedReadout(row: MilitaryAirAtRow, options: MilitaryAirAtOptions): BarsReadout {
  const ratio = ratioFigure(row);
  const figures = `${wholeCompact(row.dealt, options)} / ${wholeCompact(row.taken, options)}`;
  return { primary: ratio ? `${figures} \u00b7 ${ratio}` : figures };
}

function missingRow(readView: BoundedReadView, key: string, reason: string): BarsDataRow {
  const pres = countryRowPresentation(readView, key);
  const value = notRecorded(reason);
  return {
    kind: 'data', key, subject: key, label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    left: { value: { value }, numeric: null },
    right: { value: { value }, numeric: null },
  };
}

type Read = { readonly reply: MilitaryAirAtReply } | { readonly model: Pick<BarsModel, 'rows' | 'empty' | 'emptyKind' | 'warnings'> };

function readReply(readView: BoundedReadView): Read {
  const refused = readView.refused('main');
  if (refused) return { model: { rows: [], empty: refused, emptyKind: 'unavailable' } };
  const panel = readView.panel('main');
  if (panel === undefined) return { model: { rows: [] } };
  const reply = validateMilitaryAirAt(panel);
  if (reply.missing !== null) return { model: { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings } };
  if (reply.rankMissing !== null && reply.rows.length === 0) return { model: { rows: [], empty: reply.rankMissing, emptyKind: 'unavailable', warnings: reply.warnings } };
  return { reply };
}

function causeColour(cause: number): string {
  return colour(DEFAULT_CARD_SURFACE, cause);
}

export function lossCausesModel(readView: BoundedReadView, options: MilitaryAirAtOptions): BarsModel {
  const base = { form: 'stacked' as const, hour: readView.hour, animateOrder: true };
  const read = readReply(readView);
  if ('model' in read) return { ...base, ...read.model };
  const { reply } = read;
  const rows: BarsDataRow[] = [];
  const answered = new Set<string>();
  const seen = new Set<number>();
  for (const row of reply.rows) {
    const pres = countryRowPresentation(readView, row.tag);
    const taken = BigInt(row.taken);
    answered.add(row.tag);
    const segments: BarsSegment[] = [];
    if (taken > 0n) {
      for (const cause of row.causes) {
        const part = amount(cause.taken, row.complete, options);
        seen.add(cause.cause);
        segments.push({
          key: String(cause.cause), label: airCauseName(cause.cause), share: Number((BigInt(cause.taken) * 10_000n) / taken) / 10_000,
          value: part.value, format: part.format, color: causeColour(cause.cause),
        });
      }
    }
    const total = amount(row.taken, row.complete, options);
    rows.push({
      kind: 'data', key: `${row.tag}\u0000${row.identity}`, subject: row.tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
      ...(pres.color ? { color: pres.color } : {}),
      segments,
      readout: { primary: `${wholeCompact(row.taken, options)} lost` },
      tooltip: {
        title: pres.labelMissing ? 'Name not recorded' : pres.label,
        hour: readView.hour,
        items: [
          { label: options.leftHeader, value: formatBarsValueField({ value: total.value, format: total.format }).text },
          ...segments.map(segment => ({ label: segment.label, swatch: segment.color, value: `${formatBarsValueField({ value: segment.value, format: segment.format }).text} (${Math.round(segment.share * 100)}%)` })),
        ],
      },
    });
  }
  if (options.chosen) {
    for (const key of options.chosen) {
      if (answered.has(key)) continue;
      const pres = countryRowPresentation(readView, key);
      rows.push({ kind: 'data', key, subject: key, label: pres.label, labelMissing: pres.labelMissing, media: pres.media, segments: [], value: { value: notRecorded('This country is not present at this date.') } });
    }
  }
  const legend: BarsLegendItem[] = [...seen].sort((a, b) => a - b).map(cause => ({ key: String(cause), label: airCauseName(cause), color: causeColour(cause) }));
  return {
    ...base,
    rows,
    legend,
    ...(rows.length ? {} : { empty: 'Nothing recorded at this date.', emptyKind: 'empty' as const }),
    ...(reply.rankMissing ? { notes: [reply.rankMissing] } : {}),
    warnings: reply.warnings,
  };
}

export function militaryAirModel(readView: BoundedReadView, options: MilitaryAirAtOptions): BarsModel {
  const base = {
    form: 'two-sided' as const, hour: readView.hour, animateOrder: true, leftHeader: options.leftHeader, rightHeader: options.rightHeader, scaleUnit: options.unit,
    ...(options.paired
      ? { twoSidedLayout: 'stacked-pair' as const, legend: [{ key: 'solid', label: options.paired.solid, color: '' }, { key: 'outline', label: options.paired.outline, color: '' }] }
      : options.ratio ? { readoutHeader: 'Ratio' } : {}),
  };
  const read = readReply(readView);
  if ('model' in read) return { ...base, ...read.model };
  const { reply } = read;
  const rows: BarsDataRow[] = [];
  const answered = new Set<string>();
  for (const row of reply.rows) {
    const pres = countryRowPresentation(readView, row.tag);
    const taken = amount(row.taken, row.complete, options);
    const dealt = amount(row.dealt, row.complete, options);
    const ratio = options.ratio ? ratioFigure(row) : null;
    const readout: BarsReadout | undefined = options.paired ? pairedReadout(row, options) : ratio ? { primary: `${ratio} : 1`, secondary: 'Ratio' } : undefined;
    const key = `${row.tag}\u0000${row.identity}`;
    answered.add(row.tag);
    answered.add(countryKey(Number(row.identity.split(':')[0])));
    rows.push({
      kind: 'data', key, subject: row.tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
      ...(pres.color ? { color: pres.color } : {}),
      left: { value: { value: taken.value, format: taken.format }, numeric: taken.numeric },
      right: { value: { value: dealt.value, format: dealt.format }, numeric: dealt.numeric },
      ...(readout ? { readout } : {}),
    });
  }
  if (options.chosen) {
    for (const key of options.chosen) {
      if (!answered.has(key)) rows.push(missingRow(readView, key, 'This country is not present at this date.'));
    }
  }
  return {
    ...base,
    rows,
    ...(rows.length ? {} : { empty: 'Nothing recorded at this date.', emptyKind: 'empty' as const }),
    ...(reply.rankMissing ? { notes: [reply.rankMissing] } : {}),
    warnings: reply.warnings,
  };
}
