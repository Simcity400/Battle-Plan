import type { ResolvedViewOptions } from '../../dashboard/definitions';
import type { OptionValue } from '../../dashboard/options';
import type { TooltipExtraRow } from '../../families/table/model.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import { plainScripted } from './scriptedGaps.ts';

export interface HoverText {
  readonly key: string;
  readonly description: string | null;
  readonly gaps: string | null;
  readonly missing: string | null;
  readonly effectsMissing: readonly string[];
}

const SEPARATOR = '\u0000';

export function withHoverText(options: ResolvedViewOptions, text: HoverText | null): ResolvedViewOptions {
  if (!text) return options;
  return {
    ...options,
    hoverRow: text.key,
    hoverDescription: text.description,
    hoverGaps: text.gaps,
    hoverMissing: text.missing,
    hoverEffectsMissing: text.effectsMissing.length ? text.effectsMissing.join(SEPARATOR) : null,
  };
}

export function hoverTextOf(options: Readonly<Record<string, OptionValue>>): HoverText | null {
  const text = (value: OptionValue | undefined): string | null => (typeof value === 'string' ? value : null);
  const key = text(options.hoverRow);
  if (key === null) return null;
  const effects = text(options.hoverEffectsMissing);
  return { key, description: text(options.hoverDescription), gaps: text(options.hoverGaps), missing: text(options.hoverMissing), effectsMissing: effects ? effects.split(SEPARATOR) : [] };
}

export function descriptionRows(hover: HoverText | null, key: string, listed: string | null | undefined): TooltipExtraRow[] {
  const read = hover?.key === key ? hover : null;
  const description = read?.description ?? listed ?? null;
  const out: TooltipExtraRow[] = [];
  if (description) out.push({ label: 'Description', value: knownValue(plainScripted(description)), divider: true });
  if (read?.missing) out.push({ label: description ? 'Full description' : 'Description', value: notRecorded(read.missing), ...(description ? {} : { divider: true }) });
  for (const reason of read?.effectsMissing ?? []) out.push({ label: 'Authored effects', value: notRecorded(reason) });
  return out;
}

export function gapRows(hover: HoverText | null, key: string): TooltipExtraRow[] {
  const gaps = hover?.key === key ? hover.gaps : null;
  return gaps ? [{ label: 'Text not recorded', value: notRecorded(gaps) }] : [];
}
