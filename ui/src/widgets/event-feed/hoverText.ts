import type { ResolvedViewOptions } from '../../dashboard/definitions';
import type { OptionValue } from '../../dashboard/options';
import type { TooltipExtraRow } from '../../families/table/model.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import { plainScripted } from './scriptedGaps.ts';

export interface HoverText {
  readonly key: string;
  readonly description: string | null;
  readonly gaps: string | null;
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
  return typeof text.key === 'string' && textOrNull(text.description) && textOrNull(text.gaps) && textOrNull(text.missing)
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

export function descriptionRows(texts: TooltipTexts | null | undefined, key: string, listed: string | null | undefined): TooltipExtraRow[] {
  const read = readFor(texts, key);
  const description = read?.description ?? listed ?? null;
  const out: TooltipExtraRow[] = [];
  if (description) out.push({ label: 'Description', value: knownValue(plainScripted(description)), divider: true });
  if (read?.missing) out.push({ label: description ? 'Full description' : 'Description', value: notRecorded(read.missing), ...(description ? {} : { divider: true }) });
  return out;
}

export function gapRows(texts: TooltipTexts | null | undefined, key: string): TooltipExtraRow[] {
  const gaps = readFor(texts, key)?.gaps;
  return gaps ? [{ label: 'Text not recorded', value: notRecorded(gaps) }] : [];
}

export function unreadRows(texts: TooltipTexts | null | undefined, key: string): TooltipExtraRow[] {
  const missing = readFor(texts, key)?.missing;
  return missing ? [{ label: 'Text not recorded', value: notRecorded(missing) }] : [];
}

export function optionRows(texts: TooltipTexts | null | undefined, key: string, letter: (index: number) => string): TooltipExtraRow[] {
  return (readFor(texts, key)?.options ?? []).map((name, index) => ({
    label: `Option ${letter(index)}`,
    value: name !== null ? knownValue(plainScripted(name)) : notRecorded('Option text not recorded'),
  }));
}
