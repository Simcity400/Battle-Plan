import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BoundedReadView } from '../contract.ts';
import { holderTagAt, inHourRanges, type HolderRun, type HourRanges } from '../holders.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { knownValue, notRecorded, zeroValue, type Value } from '../value.ts';
import { formatValueState, type ValueFormat } from '../valueFormat.ts';
import type { ForceAtReply, ForceAtRow, ForcePart, NavalLossesAtReply } from '../../generated/dashboard';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsSegment } from './model.ts';

const PART_SURFACE = DEFAULT_CARD_SURFACE;

export interface ForceAtRaceBaseOptions {
  readonly top: number | null;
  readonly all?: boolean;
  readonly split: boolean;
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
}

export interface ForceAtProjectConfig {
  readonly decimals: number;
  readonly valueFormat: ValueFormat;
  readonly scaleFormat: ValueFormat;
  readonly scaleUnit: string;
  readonly valueLabel: string;
  readonly partsLabel: string;
  readonly emptyMessage: string;
  readonly referenceLabel?: string;
  readonly note?: string;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const isTextOrNull = (v: unknown): v is string | null => v === null || typeof v === 'string';
const isNumOrNull = (v: unknown): v is number | null => v === null || (typeof v === 'number' && Number.isFinite(v));

function isForcePart(v: unknown): v is ForcePart {
  return isObj(v) && typeof v.key === 'string' && isTextOrNull(v.name) && typeof v.unresolved === 'boolean';
}

function isForceAtRow(v: unknown): v is ForceAtRow {
  return isObj(v)
    && (typeof v.country === 'string' || typeof v.country === 'number')
    && typeof v.tag === 'string'
    && isNumOrNull(v.total)
    && (v.referenceTotal === undefined || isNumOrNull(v.referenceTotal))
    && isTextOrNull(v.missing)
    && Array.isArray(v.values) && v.values.every(isNumOrNull)
    && isTextOrNull(v.partsMissing);
}

export function validateForceAtReply(value: unknown): ForceAtReply {
  if (
    !isObj(value)
    || !Array.isArray(value.parts) || !value.parts.every(isForcePart)
    || !Array.isArray(value.rows) || !value.rows.every(isForceAtRow)
    || !isTextOrNull(value.missing)
    || !Array.isArray(value.warnings) || !value.warnings.every(w => typeof w === 'string')
  ) {
    throw new Error('The force snapshot reply is malformed.');
  }
  return value as unknown as ForceAtReply;
}

export function validateNavalLossesAtReply(value: unknown): NavalLossesAtReply {
  if (
    !isObj(value)
    || typeof value.firstHour !== 'number' || !Number.isInteger(value.firstHour)
    || typeof value.lastHour !== 'number' || !Number.isInteger(value.lastHour)
  ) {
    throw new Error('The naval losses snapshot reply is malformed.');
  }
  validateForceAtReply(value);
  return value as unknown as NavalLossesAtReply;
}

export function forcePartLabel(part: ForcePart): string {
  return part.name ?? (part.unresolved ? 'Name unavailable' : part.key);
}

function scaleRaw(raw: number | null, decimals: number, missingReason: string | null): { readonly value: Value<bigint | number>; readonly numeric: number | null } {
  if (raw === null) {
    return { value: notRecorded(missingReason ?? 'Not recorded at this hour'), numeric: null };
  }
  if (decimals === 0) {
    return { value: raw === 0 ? zeroValue() : knownValue(raw), numeric: raw };
  }
  const num = raw / (10 ** decimals);
  return { value: num === 0 ? zeroValue() : knownValue(num), numeric: num };
}

function formatScaled(value: Value<bigint | number>, format: ValueFormat): string {
  const fmt = formatValueState(value, format);
  if (fmt.kind === 'known' || fmt.kind === 'zero' || fmt.kind === 'partial') return fmt.exact ?? fmt.text;
  return fmt.label;
}

function compareExplicitRows(a: BarsDataRow, b: BarsDataRow): number {
  const aMissing = a.numeric === null || a.numeric === undefined;
  const bMissing = b.numeric === null || b.numeric === undefined;
  if (aMissing !== bMissing) return aMissing ? 1 : -1;
  if (!aMissing && !bMissing && a.numeric !== b.numeric) return b.numeric! - a.numeric!;
  return compareTags(a.subject ?? a.key, b.subject ?? b.key);
}

export function projectForceAtReply(
  readView: BoundedReadView,
  reply: ForceAtReply,
  options: ForceAtRaceBaseOptions,
  config: ForceAtProjectConfig,
): BarsModel {
  const hour = readView.hour;
  const ranked = options.top != null || options.all === true;
  const baseModel = {
    form: 'ordered' as const,
    hour,
    animateOrder: true,
    showAxis: true,
    scaleUnit: config.scaleUnit,
    scaleFormat: config.scaleFormat,
  };

  if (reply.missing !== null) {
    return { ...baseModel, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
  }

  const partColours = reply.parts.map((_part, index) => colour(PART_SURFACE, index));
  const rows: BarsDataRow[] = [];
  for (const item of reply.rows) {
    const key = countryKey(item.country);
    const displayTag = options.holders ? (holderTagAt(options.holders[key], hour) ?? item.tag) : item.tag;
    if (!ranked && options.membership) {
      const inMember = inHourRanges(options.membership[key], hour) || inHourRanges(options.membership[displayTag], hour);
      if (!inMember) continue;
    }
    const pres = countryRowPresentation(readView, displayTag);
    const { value, numeric } = scaleRaw(item.total, config.decimals, item.missing);
    const totalItem: TooltipListItem = value.kind === 'notRecorded'
      ? { label: config.valueLabel, missingReason: value.reason }
      : { label: config.valueLabel, value: formatScaled(value, config.valueFormat) };
    const tooltipItems: TooltipListItem[] = [totalItem];

    if (config.referenceLabel && item.referenceTotal !== undefined) {
      if (item.referenceTotal === null) {
        tooltipItems.push({ label: config.referenceLabel, missingReason: 'Not recorded at this hour' });
      } else {
        const refScaled = scaleRaw(item.referenceTotal, config.decimals, null);
        tooltipItems.push({ label: config.referenceLabel, value: formatScaled(refScaled.value, config.valueFormat) });
      }
    }

    let segments: BarsSegment[] | undefined;
    if (options.split && numeric !== null && numeric > 0) {
      if (item.partsMissing !== null) {
        tooltipItems.push({ label: config.partsLabel, missingReason: item.partsMissing });
      } else if (reply.parts.length > 0 && item.values.length === reply.parts.length) {
        const built: BarsSegment[] = [];
        let knownSum = 0;
        let anyMissing = false;
        const partEntries: { readonly part: ForcePart; readonly color: string; readonly scaled: { readonly value: Value<bigint | number>; readonly numeric: number | null } }[] = [];
        for (let i = 0; i < reply.parts.length; i++) {
          const rawPart = item.values[i] ?? null;
          if (rawPart === null) {
            anyMissing = true;
            break;
          }
          if (rawPart <= 0) continue;
          const scaled = scaleRaw(rawPart, config.decimals, null);
          knownSum += scaled.numeric ?? 0;
          partEntries.push({ part: reply.parts[i]!, color: partColours[i]!, scaled });
        }
        if (anyMissing) {
          tooltipItems.push({ label: config.partsLabel, missingReason: 'Parts not recorded at this hour' });
        } else if (knownSum > 0) {
          for (const entry of partEntries) {
            const partNum = entry.scaled.numeric ?? 0;
            const partName = forcePartLabel(entry.part);
            built.push({
              key: entry.part.key,
              label: partName,
              share: partNum / knownSum,
              value: entry.scaled.value,
              format: config.valueFormat,
              color: entry.color,
            });
            tooltipItems.push({ label: partName, swatch: entry.color, value: formatScaled(entry.scaled.value, config.valueFormat) });
          }
          if (built.length) segments = built;
        }
      }
    }

    rows.push({
      kind: 'data',
      key,
      subject: displayTag,
      label: pres.label,
      labelMissing: pres.labelMissing,
      media: pres.media,
      ...(pres.color ? { color: pres.color } : {}),
      value: { value, format: config.valueFormat, label: config.valueLabel },
      numeric,
      ...(segments ? { segments } : {}),
      tooltip: {
        title: pres.labelMissing ? 'Name not recorded' : pres.label,
        hour,
        items: tooltipItems,
        ...(config.note ? { note: config.note } : {}),
      },
      meta: { tag: displayTag, key },
    });
  }

  if (!ranked) rows.sort(compareExplicitRows);
  return {
    ...baseModel,
    rows,
    warnings: reply.warnings,
    ...(!rows.length ? { empty: config.emptyMessage, emptyKind: 'empty' as const } : {}),
  };
}
