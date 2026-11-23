import type { Value } from './value.ts';
import { formatValueState, type ValueFormat } from './valueFormat.ts';
import './value.css';

export interface ValueTextProps {
  value: Value;
  format?: ValueFormat;
  narrow?: boolean;
}

export function ValueText({ value, format, narrow = false }: ValueTextProps) {
  const formatted = formatValueState(value, format);
  switch (formatted.kind) {
    case 'known':
    case 'zero':
      return <span>{formatted.text}</span>;
    case 'notRecorded':
      return <span className={`fv-missing${narrow ? ' is-narrow' : ''}`} aria-label={formatted.spoken} data-tooltip={formatted.reason}>{formatted.label}</span>;
    case 'notApplicable':
      return <span className="fv-na" aria-label={formatted.spoken} data-tooltip={formatted.reason}>{formatted.label}</span>;
    case 'partial':
      return <span className={`fv-partial${narrow ? ' is-narrow' : ''}`} aria-label={formatted.spoken} data-tooltip={formatted.reason}>
        <span className="fv-partial-value">{formatted.text}</span>
        {!narrow && <span className="fv-partial-word">partial</span>}
      </span>;
  }
}
