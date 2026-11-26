import { colour } from '../../charts/palette.ts';
import type { BoundedReadView } from '../contract.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { knownValue, notRecorded, zeroValue, type Value } from '../value.ts';
import { formatShare, formatValueState, type ValueFormat } from '../valueFormat.ts';
import type { ForceAtReply, ForcePart } from '../../generated/dashboard';
import { forcePartLabel, forceRowIdentity, GROUP_EMPTY, witnessedPartItems } from './forceAtRace.ts';
import { type BarsLegendItem, type BarsModel, type BarsSegment } from './model.ts';
import { buildShareRowsModel, type ShareRowEntry } from './shareRows.ts';

export interface ForceAtMixOptions {
  readonly top: number | null;
  readonly all?: boolean;
  readonly group: boolean;
  readonly surface: string;
}

export interface ForceAtMixConfig {
  readonly decimals: number;
  readonly valueFormat: ValueFormat;
  readonly valueLabel: string;
  readonly unit: string;
  readonly partsLabel: string;
  readonly emptyMessage: string;
  readonly referenceLabel?: string;
  readonly note?: string;
}

function scaleRaw(raw: number | null, decimals: number, missingReason: string | null): { readonly value: Value<bigint | number>; readonly numeric: number | null } {
  if (raw === null) return { value: notRecorded(missingReason ?? 'Not recorded at this hour'), numeric: null };
  if (decimals === 0) return { value: raw === 0 ? zeroValue() : knownValue(raw), numeric: raw };
  const num = raw / (10 ** decimals);
  return { value: num === 0 ? zeroValue() : knownValue(num), numeric: num };
}

function formatScaled(value: Value<bigint | number>, format: ValueFormat): string {
  const fmt = formatValueState(value, format);
  if (fmt.kind === 'known' || fmt.kind === 'zero' || fmt.kind === 'partial') return fmt.exact ?? fmt.text;
  return fmt.label;
}

function sharePct(share: number): string {
  return formatShare(share);
}

export function projectForceAtMix(
  readView: BoundedReadView,
  reply: ForceAtReply,
  options: ForceAtMixOptions,
  config: ForceAtMixConfig,
): BarsModel {
  const hour = readView.hour;

  if (reply.missing !== null) {
    return buildShareRowsModel({
      hour,
      rows: [],
      empty: reply.missing,
      emptyKind: 'unavailable',
      warnings: reply.warnings,
    });
  }

  const partColours = reply.parts.map((_part, index) => colour(options.surface, index));
  const usedParts = new Set<number>();
  const rows: ShareRowEntry[] = [];

  for (const item of reply.rows) {
    const { key, tag: displayTag, pres } = forceRowIdentity(readView, item);
    const { value, numeric } = scaleRaw(item.total, config.decimals, item.missing);
    const countText = value.kind === 'notRecorded' ? undefined : formatScaled(value, config.valueFormat);
    const totalItem: TooltipListItem = value.kind === 'notRecorded'
      ? { label: config.valueLabel, missingReason: value.reason }
      : { label: config.valueLabel, value: countText! };
    const tooltipItems: TooltipListItem[] = [totalItem];

    if (config.referenceLabel && item.referenceTotal !== undefined) {
      if (item.referenceTotal === null) {
        tooltipItems.push({ label: config.referenceLabel, missingReason: 'Not recorded at this hour' });
      } else {
        const refScaled = scaleRaw(item.referenceTotal, config.decimals, null);
        tooltipItems.push({ label: config.referenceLabel, value: formatScaled(refScaled.value, config.valueFormat) });
      }
    }

    if (value.kind === 'notRecorded') {
      rows.push({
        key,
        subject: displayTag,
        label: pres.label,
        labelMissing: pres.labelMissing, ...(pres.labelLoading ? { labelLoading: true } : {}),
        media: pres.media,
        segments: [],
        missing: value,
        format: config.valueFormat,
        tooltip: {
          title: pres.labelMissing ? 'Name not recorded' : pres.label,
          hour,
          missingReason: value.reason,
          items: tooltipItems,
          ...(config.note ? { note: config.note } : {}),
        },
        meta: { tag: displayTag, key },
      });
      continue;
    }

    if (numeric === 0) {
      rows.push({
        key,
        subject: displayTag,
        label: pres.label,
        labelMissing: pres.labelMissing, ...(pres.labelLoading ? { labelLoading: true } : {}),
        media: pres.media,
        segments: [],
        countText,
        tooltip: {
          title: pres.labelMissing ? 'Name not recorded' : pres.label,
          hour,
          items: tooltipItems,
          ...(config.note ? { note: config.note } : {}),
        },
        meta: { tag: displayTag, key },
      });
      continue;
    }

    if (item.partsMissing !== null) {
      tooltipItems.push({ label: config.partsLabel, missingReason: item.partsMissing }, ...witnessedPartItems(reply, item, config.decimals, config.valueFormat));
      rows.push({
        key,
        subject: displayTag,
        label: pres.label,
        labelMissing: pres.labelMissing, ...(pres.labelLoading ? { labelLoading: true } : {}),
        media: pres.media,
        segments: [],
        countText,
        missing: notRecorded(item.partsMissing),
        format: config.valueFormat,
        tooltip: {
          title: pres.labelMissing ? 'Name not recorded' : pres.label,
          hour,
          missingReason: item.partsMissing,
          items: tooltipItems,
          ...(config.note ? { note: config.note } : {}),
        },
        meta: { tag: displayTag, key },
      });
      continue;
    }

    const segments: BarsSegment[] = [];
    let anyMissing = reply.parts.length === 0 || item.values.length !== reply.parts.length;
    const rawParts: { readonly index: number; readonly part: ForcePart; readonly scaled: { readonly value: Value<bigint | number>; readonly numeric: number | null } }[] = [];
    if (!anyMissing) {
      for (let i = 0; i < reply.parts.length; i++) {
        const rawPart = item.values[i] ?? null;
        if (rawPart === null) {
          anyMissing = true;
          break;
        }
        if (rawPart <= 0) continue;
        rawParts.push({ index: i, part: reply.parts[i]!, scaled: scaleRaw(rawPart, config.decimals, null) });
      }
    }

    if (anyMissing) {
      const reason = 'Part breakdown not recorded at this hour';
      tooltipItems.push({ label: config.partsLabel, missingReason: reason });
      rows.push({
        key,
        subject: displayTag,
        label: pres.label,
        labelMissing: pres.labelMissing, ...(pres.labelLoading ? { labelLoading: true } : {}),
        media: pres.media,
        segments: [],
        countText,
        missing: notRecorded(reason),
        format: config.valueFormat,
        tooltip: {
          title: pres.labelMissing ? 'Name not recorded' : pres.label,
          hour,
          missingReason: reason,
          items: tooltipItems,
          ...(config.note ? { note: config.note } : {}),
        },
        meta: { tag: displayTag, key },
      });
      continue;
    }

    const denom = numeric ?? 0;
    if (denom > 0) {
      for (const partItem of rawParts) {
        const partNum = partItem.scaled.numeric ?? 0;
        const share = partNum / denom;
        const label = forcePartLabel(partItem.part);
        const color = partColours[partItem.index]!;
        usedParts.add(partItem.index);
        segments.push({
          key: partItem.part.key,
          label,
          share,
          value: partItem.scaled.value,
          format: config.valueFormat,
          color,
          inlineLabel: `${label} ${sharePct(share)}`,
        });
        tooltipItems.push({
          label,
          swatch: color,
          value: `${formatScaled(partItem.scaled.value, config.valueFormat)} (${sharePct(share)})`,
        });
      }
    }

    rows.push({
      key,
      subject: displayTag,
      label: pres.label,
      labelMissing: pres.labelMissing, ...(pres.labelLoading ? { labelLoading: true } : {}),
      media: pres.media,
      segments,
      countText,
      tooltip: {
        title: pres.labelMissing ? 'Name not recorded' : pres.label,
        hour,
        items: tooltipItems,
        ...(config.note ? { note: config.note } : {}),
      },
      meta: { tag: displayTag, key },
    });
  }

  const legend: BarsLegendItem[] = [];
  for (let i = 0; i < reply.parts.length; i++) {
    if (!usedParts.has(i)) continue;
    const part = reply.parts[i]!;
    legend.push({ key: part.key, label: forcePartLabel(part), color: partColours[i]! });
  }

  return buildShareRowsModel({
    hour,
    rows,
    legend,
    unit: config.unit,
    warnings: reply.warnings,
    ...(!rows.length ? { empty: options.group ? GROUP_EMPTY : config.emptyMessage, emptyKind: 'empty' as const } : {}),
  });
}
