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

export function withHoverText(options: ResolvedViewOptions, text: HoverText | null): ResolvedViewOptions {
  if (!text) return options;
  return {
    ...options,
    hoverRow: text.key,
    hoverDescription: text.description,
    hoverGaps: text.gaps,
    hoverOptions: text.options ? JSON.stringify(text.options) : null,
    hoverMissing: text.missing,
  };
}

function optionNames(value: OptionValue | undefined): readonly (string | null)[] | null {
  if (typeof value !== 'string') return null;
  try {
    const names: unknown = JSON.parse(value);
    return Array.isArray(names) && names.every(name => name === null || typeof name === 'string') ? names : null;
  } catch {
    return null;
  }
}

export function hoverTextOf(options: Readonly<Record<string, OptionValue>>): HoverText | null {
  const text = (value: OptionValue | undefined): string | null => (typeof value === 'string' ? value : null);
  const key = text(options.hoverRow);
  if (key === null) return null;
  return { key, description: text(options.hoverDescription), gaps: text(options.hoverGaps), options: optionNames(options.hoverOptions), missing: text(options.hoverMissing) };
}

const readFor = (hover: HoverText | null, key: string): HoverText | null => (hover?.key === key ? hover : null);

export function descriptionRows(hover: HoverText | null, key: string, listed: string | null | undefined): TooltipExtraRow[] {
  const read = readFor(hover, key);
  const description = read?.description ?? listed ?? null;
  const out: TooltipExtraRow[] = [];
  if (description) out.push({ label: 'Description', value: knownValue(plainScripted(description)), divider: true });
  if (read?.missing) out.push({ label: description ? 'Full description' : 'Description', value: notRecorded(read.missing), ...(description ? {} : { divider: true }) });
  return out;
}

export function gapRows(hover: HoverText | null, key: string): TooltipExtraRow[] {
  const gaps = readFor(hover, key)?.gaps;
  return gaps ? [{ label: 'Text not recorded', value: notRecorded(gaps) }] : [];
}

export function unreadRows(hover: HoverText | null, key: string): TooltipExtraRow[] {
  const missing = readFor(hover, key)?.missing;
  return missing ? [{ label: 'Text not recorded', value: notRecorded(missing) }] : [];
}

export function optionRows(hover: HoverText | null, key: string, letter: (index: number) => string): TooltipExtraRow[] {
  return (readFor(hover, key)?.options ?? []).map((name, index) => ({
    label: `Option ${letter(index)}`,
    value: name !== null ? knownValue(plainScripted(name)) : notRecorded('Option text not recorded'),
  }));
}
