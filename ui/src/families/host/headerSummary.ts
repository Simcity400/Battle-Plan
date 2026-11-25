export interface HeaderSummaryInput {
  readonly summaryText?: string | null;
  readonly summaryPartial?: string | null;
  readonly notes?: readonly string[];
}

export interface HeaderSummary {
  readonly label: string;
  readonly tooltip: string | null;
  readonly spoken: string;
  readonly hasMarker: boolean;
}

function uniqueTexts(items: readonly (string | null | undefined)[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const item of items) {
    const trimmed = item?.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    out.push(trimmed);
  }
  return out;
}

export function resolveHeaderSummary(input: HeaderSummaryInput): HeaderSummary | null {
  const summaryText = input.summaryText?.trim() || null;
  const summaryPartial = input.summaryPartial?.trim() || null;
  const details = uniqueTexts([summaryPartial, ...(input.notes ?? [])]);
  const hasMarker = details.length > 0;
  const label = summaryText ?? (summaryPartial ? 'Partial' : hasMarker ? 'Note' : null);
  if (!label) return null;
  const tooltip = hasMarker ? details.join('\n') : null;
  const spoken = hasMarker ? `${label}. ${details.join(' ')}` : label;
  return { label, tooltip, spoken, hasMarker };
}

export function mergeDiagnosticNotes(
  extraWarnings: readonly string[] | undefined,
  warnings: readonly string[],
  notes: readonly string[],
): string[] {
  return uniqueTexts([...(extraWarnings ?? []), ...warnings, ...notes]);
}

export function mergeAccessibleDescription(
  existing: string | null,
  summary: HeaderSummary | null,
): string | null {
  const base = existing?.trim() || null;
  if (!summary?.hasMarker) return base;
  if (!base) return summary.spoken;
  if (base.includes(summary.spoken)) return base;
  return `${base}. ${summary.spoken}`;
}
