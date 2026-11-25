import type { Ref } from 'react';
import type { HeaderSummary } from './headerSummary.ts';

export function FamilyHeaderSummary({ summary, ref }: { readonly summary: HeaderSummary | null; readonly ref?: Ref<HTMLSpanElement> }) {
  if (!summary) return null;
  if (summary.hasMarker && summary.tooltip) {
    return (
      <span
        ref={ref}
        className="fh-summary fh-summary-partial"
        tabIndex={0}
        data-tooltip={summary.tooltip}
        aria-label={summary.spoken}
      >
        {summary.label}
        <span className="fh-summary-hatch" aria-hidden="true" />
      </span>
    );
  }
  return <span ref={ref} className="fh-summary">{summary.label}</span>;
}
