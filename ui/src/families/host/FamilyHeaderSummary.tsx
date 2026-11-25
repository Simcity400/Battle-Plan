import type { HeaderSummary } from './headerSummary.ts';

export function FamilyHeaderSummary({ summary }: { readonly summary: HeaderSummary | null }) {
  if (!summary) return null;
  if (summary.hasMarker && summary.tooltip) {
    return (
      <span
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
  return <span className="fh-summary">{summary.label}</span>;
}
