import { valueStateText, type ValueStateProps } from './valueStateText.ts';
import './ValueState.css';

export type { ValueStateProps };

export function ValueState({ kind, text, unit, reason, narrow = false, className = '' }: ValueStateProps) {
  const extra = className ? ` ${className}` : '';
  switch (kind) {
    case 'known':
      return <span className={`val-state val-known${extra}`} data-value-state="known">
        <span>{text ?? ''}</span>{unit && <span className="val-unit">{unit}</span>}
      </span>;
    case 'zero':
      return <span className={`val-state val-zero${extra}`} data-value-state="zero">
        <span>{text ?? '0'}</span>{unit && <span className="val-unit">{unit}</span>}
      </span>;
    case 'missing': {
      const spoken = valueStateText({ kind, reason });
      return <span className={`val-state val-missing${narrow ? ' is-compact' : ''}${extra}`} data-value-state="missing"
        data-tooltip={spoken} aria-label={spoken}>
        <span className="val-hatch" aria-hidden="true" />
        <span className="val-missing-text" aria-hidden="true">Not recorded</span>
      </span>;
    }
    case 'notApplicable': {
      const spoken = valueStateText({ kind, reason });
      return <span className={`val-state val-na${extra}`} data-value-state="not-applicable"
        data-tooltip={reason || 'Not applicable'} aria-label={spoken}>—</span>;
    }
    case 'partial': {
      const spoken = valueStateText({ kind, text, unit, reason });
      const tooltip = reason ? `Partial: ${reason}` : 'Partial';
      return <span className={`val-state val-partial${narrow ? ' is-compact' : ''}${extra}`} data-value-state="partial"
        data-tooltip={tooltip} aria-label={spoken}>
        <span className="val-partial-figure">{text ?? ''}{unit && <span className="val-unit">{unit}</span>}</span>
        <span className="val-partial-tab" aria-hidden="true" />
        <span className="val-partial-word" aria-hidden="true">partial</span>
      </span>;
    }
  }
}
