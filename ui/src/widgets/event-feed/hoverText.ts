import type { ResolvedViewOptions } from '../../dashboard/definitions';
import type { OptionValue } from '../../dashboard/options';
import type { TooltipExtraRow } from '../../families/table/model.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import type { EventFeedGap } from '../../generated/dashboard';
import { gapSummary, gapsForField, isEventFeedGap, scriptedValue } from './scriptedGaps.ts';

export interface HoverText {
  readonly key: string;
  readonly description: string | null;
  readonly gaps: readonly EventFeedGap[] | null;
  readonly options: readonly (string | null)[] | null;
  readonly missing: string | null;
}

export type TooltipTexts = readonly HoverText[];

export function withHoverText(options: ResolvedViewOptions, texts: TooltipTexts): ResolvedViewOptions {
  return texts.length ? { ...options, tooltipText: JSON.stringify(texts) } : options;
}

const textOrNull = (value: unknown): value is string | null => value === null || typeof value === 'string';

function hoverText(value: unknown): value is HoverText {
  if (typeof value !== 'object' || value === null) return false;
  const text = value as Record<string, unknown>;
  return typeof text.key === 'string' && textOrNull(text.description) && textOrNull(text.missing)
    && (text.gaps === null || (Array.isArray(text.gaps) && text.gaps.every(isEventFeedGap)))
    && (text.options === null || (Array.isArray(text.options) && text.options.every(textOrNull)));
}

export function hoverTextOf(options: Readonly<Record<string, OptionValue>>): TooltipTexts {
  const raw = options.tooltipText;
  if (typeof raw !== 'string') return [];
  try {
    const texts: unknown = JSON.parse(raw);
    return Array.isArray(texts) && texts.every(hoverText) ? texts : [];
  } catch {
    return [];
  }
}

const readFor = (texts: TooltipTexts | null | undefined, key: string): HoverText | null => texts?.find(text => text.key === key) ?? null;

function effectiveGaps(
  texts: TooltipTexts | null | undefined,
  key: string,
  listed?: readonly EventFeedGap[] | null,
): readonly EventFeedGap[] {
  const hovered = readFor(texts, key)?.gaps;
  if (hovered && hovered.length > 0) return hovered;
  return listed ?? [];
}

function gapFieldLabel(gap: EventFeedGap, letter?: (index: number) => string): string {
  if (gap.field === 'title') return 'Text not recorded (title)';
  if (gap.field === 'description') return 'Text not recorded (description)';
  if (gap.ordinal !== undefined && letter) return `Text not recorded (option ${letter(gap.ordinal)})`;
  return 'Text not recorded (option)';
}

export function descriptionRows(
  texts: TooltipTexts | null | undefined,
  key: string,
  listed: string | null | undefined,
  listedGaps?: readonly EventFeedGap[] | null,
): TooltipExtraRow[] {
  const read = readFor(texts, key);
  const description = read?.description ?? listed ?? null;
  const descGaps = gapsForField(effectiveGaps(texts, key, listedGaps), 'description');
  const out: TooltipExtraRow[] = [];
  if (description) out.push({ label: 'Description', value: scriptedValue(description, descGaps), divider: true });
  if (read?.missing) out.push({ label: description ? 'Full description' : 'Description', value: notRecorded(read.missing), ...(description ? {} : { divider: true }) });
  return out;
}

export function hasTooltipGaps(
  texts: TooltipTexts | null | undefined,
  key: string,
  listedGaps?: readonly EventFeedGap[] | null,
): boolean {
  return effectiveGaps(texts, key, listedGaps).length > 0;
}

export function gapRows(
  texts: TooltipTexts | null | undefined,
  key: string,
  listedGaps?: readonly EventFeedGap[] | null,
  letter?: (index: number) => string,
): TooltipExtraRow[] {
  return effectiveGaps(texts, key, listedGaps).map(gap => ({
    label: gapFieldLabel(gap, letter),
    value: knownValue(gapSummary(gap)),
    tone: 'muted',
  }));
}

export function unreadRows(texts: TooltipTexts | null | undefined, key: string): TooltipExtraRow[] {
  const missing = readFor(texts, key)?.missing;
  return missing ? [{ label: 'Text not recorded', value: notRecorded(missing) }] : [];
}

export function optionRows(
  texts: TooltipTexts | null | undefined,
  key: string,
  letter: (index: number) => string,
  listedGaps?: readonly EventFeedGap[] | null,
): TooltipExtraRow[] {
  return optionLines(readFor(texts, key)?.options ?? [], letter, effectiveGaps(texts, key, listedGaps));
}

export function optionLines(
  names: readonly (string | null)[],
  letter: (index: number) => string,
  gaps?: readonly EventFeedGap[] | null,
  ordinals?: readonly number[],
): TooltipExtraRow[] {
  return names.map((name, index) => {
    const ordinal = ordinals?.[index] ?? index;
    const optGaps = gapsForField(gaps, 'option', ordinal);
    return {
      label: `Option ${letter(index)}`,
      value: name !== null ? scriptedValue(name, optGaps) : notRecorded('Option text not recorded'),
    };
  });
}
