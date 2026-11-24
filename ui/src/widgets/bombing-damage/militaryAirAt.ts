import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryRowPresentation, formatBarsValueField, type BarsDataRow, type BarsLegendItem, type BarsValueField, type BarsModel, type BarsReadout, type BarsSegment } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import { knownValue, notRecorded, partialValue, zeroValue } from '../../families/value.ts';
import { formatCompactScalar, formatShare } from '../../families/valueFormat.ts';
import type { MilitaryAirAtReply, MilitaryAirAtRow, MilitaryAirCauseName, MilitaryAirFamily, MilitaryAirMetric } from '../../generated/dashboard';

export const MILITARY_AIR_AT = 'military-air-at';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const whole = (value: unknown) => typeof value === 'string' && /^-?\d+$/.test(value);
const text = (value: unknown) => value === null || typeof value === 'string';
const countryRef = (value: unknown) => typeof value === 'string' || Number.isSafeInteger(value);
const causeName = (cause: Record<string, unknown>) => Number.isSafeInteger(cause.cause) && text(cause.name) && text(cause.missing) && (cause.name === null) !== (cause.missing === null);

export function validateMilitaryAirAt(value: unknown): MilitaryAirAtReply {
  if (!object(value) || !Number.isSafeInteger(value.hour) || !Number.isSafeInteger(value.validUntil)
    || !(value.countedFrom === null || Number.isSafeInteger(value.countedFrom)) || !text(value.missing) || !text(value.rankMissing)
    || !Array.isArray(value.unattributed) || value.unattributed.length !== 2 || !value.unattributed.every(whole)
    || !Array.isArray(value.absent) || !value.absent.every(countryRef)
    || !Array.isArray(value.warnings) || !Array.isArray(value.rows) || !value.rows.every(row => object(row) && typeof row.tag === 'string'
      && typeof row.identity === 'string' && Number.isSafeInteger(row.segmentFirstHour) && whole(row.dealt) && whole(row.taken) && typeof row.complete === 'boolean'
      && text(row.partial) && (row.complete === (row.partial === null)) && (row.ratioHundredths === null || whole(row.ratioHundredths))
      && Array.isArray(row.causes) && row.causes.every(cause => object(cause) && causeName(cause) && whole(cause.taken))))
    throw new Error('The military air reply is malformed.');
  return value as unknown as MilitaryAirAtReply;
}

export interface MilitaryAirFilters {
  readonly regions: readonly number[];
  readonly causes: readonly number[];
}

export interface MilitaryAirAtOptions {
  readonly family: MilitaryAirFamily;
  readonly scale: number;
  readonly digits: number;
  readonly leftHeader: string;
  readonly rightHeader: string;
  readonly unit: string;
  readonly ratio: boolean;
  readonly group: boolean;
  readonly none: string;
  readonly filters: MilitaryAirFilters;
  readonly paired?: { readonly solid: string; readonly outline: string };
}

export function militaryAirParams(ctx: FamilyPlanContext, family: MilitaryAirFamily, metric: MilitaryAirMetric, filters: MilitaryAirFilters): Record<string, unknown> {
  const top = ctx.countries.top;
  const group = top == null && !ctx.countries.all ? groupSelection(ctx.widget.countries) : null;
  return {
    branchId: ctx.branch.id,
    family,
    regions: [...filters.regions],
    causes: [...filters.causes],
    ...(ctx.staticSpan && ctx.widget.time.mode === 'range' ? { fromHour: ctx.staticSpan[0] } : {}),
    metric,
    ...(top != null ? { top } : group ? { group } : !ctx.countries.all ? { countries: ctx.countries.tags.map(countryFromKey) } : {}),
  };
}

export function militaryAirSources(owner: string, ctx: FamilyPlanContext, params: Record<string, unknown>): SourcePlan {
  const top = ctx.countries.top;
  const keyed = top == null && !ctx.countries.all && !params.group;
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
        coverage: top != null ? { mode: 'population', form: 'top', limit: top } : keyed ? { mode: 'keyed', keys: ctx.countries.tags } : { mode: 'population', form: 'all' },
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

export interface MilitaryAirRowMeta {
  readonly tag: string;
  readonly identity: string;
  readonly countedFrom: number | null;
  readonly filters: MilitaryAirFilters;
}

export const AIR_LOSSES_AT = { scale: 100_000, digits: 5 } as const;

export function militaryAirAmount(raw: string, partial: string | null, options: Pick<MilitaryAirAtOptions, 'digits'>): BarsValueField {
  const big = BigInt(raw);
  const value = partial !== null ? partialValue(big, partial) : big === 0n ? zeroValue(0n) : knownValue(big);
  return { value, format: { decimals: options.digits } };
}

function magnitude(raw: string, options: Pick<MilitaryAirAtOptions, 'scale'>): number {
  return Math.max(0, Number(BigInt(raw))) / options.scale;
}

export function causeLabel(cause: MilitaryAirCauseName): string {
  return cause.name ?? `Cause ${cause.cause}`;
}

function compact(raw: string, options: Pick<MilitaryAirAtOptions, 'digits'>): string {
  return formatCompactScalar(BigInt(raw), { decimals: options.digits }).text;
}

function ratioText(row: MilitaryAirAtRow): string | null {
  if (row.ratioHundredths === null) return null;
  const hundredths = BigInt(row.ratioHundredths);
  return `${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')}`;
}

function pairedReadout(row: MilitaryAirAtRow, options: MilitaryAirAtOptions): BarsReadout {
  const ratio = ratioText(row);
  const figures = `${compact(row.dealt, options)} / ${compact(row.taken, options)}`;
  return { primary: ratio ? `${figures} · ${ratio}` : figures, ...(row.partial ? { tone: 'muted' as const } : {}) };
}

function slotKeyOf(row: MilitaryAirAtRow): string {
  return countryKey(Number(row.identity.split(':')[0]));
}

function rowMeta(row: MilitaryAirAtRow, reply: MilitaryAirAtReply, options: MilitaryAirAtOptions): MilitaryAirRowMeta {
  return { tag: row.tag, identity: row.identity, countedFrom: reply.countedFrom, filters: options.filters };
}

const ABSENT = 'This country is not present at this date.';

function absentRows(readView: BoundedReadView, reply: MilitaryAirAtReply, shape: 'two-sided' | 'stacked'): BarsDataRow[] {
  return reply.absent.map(country => {
    const key = countryKey(country);
    const pres = countryRowPresentation(readView, key);
    const value = notRecorded(ABSENT);
    const shared = { kind: 'data' as const, key, subject: key, label: pres.label, labelMissing: pres.labelMissing, media: pres.media, ...(pres.color ? { color: pres.color } : {}) };
    return shape === 'two-sided'
      ? { ...shared, left: { value: { value }, numeric: null }, right: { value: { value }, numeric: null } }
      : { ...shared, segments: [], value: { value } };
  });
}

function unattributedNote(reply: MilitaryAirAtReply, options: MilitaryAirAtOptions): string | null {
  const [dealt, taken] = reply.unattributed;
  if (dealt === '0' && taken === '0') return null;
  const field = (raw: string) => formatBarsValueField(militaryAirAmount(raw, null, options)).text;
  return `No recorded country: ${options.leftHeader} ${field(taken)}, ${options.rightHeader} ${field(dealt)}`;
}

type Read = { readonly reply: MilitaryAirAtReply } | { readonly model: Pick<BarsModel, 'rows' | 'empty' | 'emptyKind' | 'warnings' | 'notes'> };

function readReply(readView: BoundedReadView, options: MilitaryAirAtOptions): Read {
  const refused = readView.refused('main');
  if (refused) return { model: { rows: [], empty: refused, emptyKind: 'unavailable' } };
  const panel = readView.panel('main');
  if (panel === undefined) return { model: { rows: [] } };
  const reply = validateMilitaryAirAt(panel);
  if (reply.missing !== null) return { model: { rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings } };
  const note = unattributedNote(reply, options);
  if (reply.rankMissing !== null && reply.rows.length === 0) {
    return { model: { rows: [], empty: reply.rankMissing, emptyKind: 'unavailable', warnings: reply.warnings, ...(note ? { notes: [note] } : {}) } };
  }
  return { reply };
}

function notes(reply: MilitaryAirAtReply, options: MilitaryAirAtOptions): readonly string[] | undefined {
  const list = [reply.rankMissing, unattributedNote(reply, options)].filter((item): item is string => item !== null);
  return list.length ? list : undefined;
}

function emptyText(options: MilitaryAirAtOptions): string {
  return options.group ? 'No country of this group at this date.' : options.none;
}

function causeColour(cause: number): string {
  return colour(DEFAULT_CARD_SURFACE, cause);
}

export function lossCausesModel(readView: BoundedReadView, options: MilitaryAirAtOptions): BarsModel {
  const base = { form: 'stacked' as const, hour: readView.hour, animateOrder: true };
  const read = readReply(readView, options);
  if ('model' in read) return { ...base, ...read.model };
  const { reply } = read;
  const seen = new Map<number, MilitaryAirCauseName>();
  const rows: BarsDataRow[] = reply.rows.map(row => {
    const pres = countryRowPresentation(readView, slotKeyOf(row));
    const taken = BigInt(row.taken);
    const total = militaryAirAmount(row.taken, row.partial, options);
    const segments: BarsSegment[] = row.complete && taken > 0n
      ? row.causes.map(cause => {
        seen.set(cause.cause, cause);
        return {
          key: String(cause.cause), label: causeLabel(cause), share: Number((BigInt(cause.taken) * 10_000n) / taken) / 10_000,
          ...militaryAirAmount(cause.taken, null, options), color: causeColour(cause.cause),
        };
      })
      : [];
    return {
      kind: 'data', key: row.identity, subject: slotKeyOf(row), label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
      ...(pres.color ? { color: pres.color } : {}),
      segments,
      ...(row.complete ? {} : { value: total }),
      meta: rowMeta(row, reply, options),
      readout: { primary: `${compact(row.taken, options)} lost`, ...(row.partial ? { tone: 'muted' as const } : {}) },
      tooltip: {
        title: pres.labelMissing ? 'Name not recorded' : pres.label,
        hour: readView.hour,
        items: [
          row.partial ? { label: options.leftHeader, value: formatBarsValueField(total).text, partialReason: row.partial } : { label: options.leftHeader, value: formatBarsValueField(total).text },
          ...row.causes.map(cause => ({
            label: causeLabel(cause),
            ...(cause.missing ? { missingReason: cause.missing } : {}),
            value: `${formatBarsValueField(militaryAirAmount(cause.taken, row.partial, options)).text}${row.complete && taken > 0n ? ` (${formatShare(Number((BigInt(cause.taken) * 10_000n) / taken) / 10_000)})` : ''}`,
          })),
        ],
      },
    };
  });
  rows.push(...absentRows(readView, reply, 'stacked'));
  const legend: BarsLegendItem[] = [...seen.values()].sort((a, b) => a.cause - b.cause).map(cause => ({ key: String(cause.cause), label: causeLabel(cause), color: causeColour(cause.cause) }));
  const shownNotes = notes(reply, options);
  return {
    ...base,
    rows,
    legend,
    ...(rows.length ? {} : { empty: emptyText(options), emptyKind: 'empty' as const }),
    ...(shownNotes ? { notes: shownNotes } : {}),
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
  const read = readReply(readView, options);
  if ('model' in read) return { ...base, ...read.model };
  const { reply } = read;
  const rows: BarsDataRow[] = reply.rows.map(row => {
    const pres = countryRowPresentation(readView, slotKeyOf(row));
    const ratio = options.ratio ? ratioText(row) : null;
    const readout: BarsReadout | undefined = options.paired ? pairedReadout(row, options) : ratio ? { primary: `${ratio} : 1`, secondary: 'Ratio' } : undefined;
    return {
      kind: 'data', key: row.identity, subject: slotKeyOf(row), label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
      ...(pres.color ? { color: pres.color } : {}),
      left: { value: militaryAirAmount(row.taken, row.partial, options), numeric: magnitude(row.taken, options) },
      right: { value: militaryAirAmount(row.dealt, row.partial, options), numeric: magnitude(row.dealt, options) },
      meta: rowMeta(row, reply, options),
      ...(readout ? { readout } : {}),
    };
  });
  rows.push(...absentRows(readView, reply, 'two-sided'));
  const shownNotes = notes(reply, options);
  return {
    ...base,
    rows,
    ...(rows.length ? {} : { empty: emptyText(options), emptyKind: 'empty' as const }),
    ...(shownNotes ? { notes: shownNotes } : {}),
    warnings: reply.warnings,
  };
}
