import type { CSSProperties } from 'react';
import { valueStateText, type ValueStateKind, type ValueStateProps } from './valueStateText.ts';
import './ValueState.css';

export { valueStateText };
export type { ValueStateKind, ValueStateProps };

export interface MissingPictureProps {
  kind?: 'flag' | 'picture';
  reason?: string;
  width?: number | string;
  height?: number | string;
  className?: string;
  decorative?: boolean;
}

export interface MissingNameProps {
  reason?: string;
  className?: string;
}

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

export function MissingPicture({ kind = 'picture', reason, width, height, className = '', decorative = false }: MissingPictureProps) {
  const label = reason ? `${kind === 'flag' ? 'Flag' : 'Picture'} not recorded: ${reason}` : `${kind === 'flag' ? 'Flag' : 'Picture'} not recorded`;
  const style: CSSProperties | undefined = width !== undefined || height !== undefined
    ? { ...(width !== undefined ? { width } : {}), ...(height !== undefined ? { height } : {}) } : undefined;
  return <span className={`val-picture-missing${kind === 'flag' ? ' is-flag' : ''}${className ? ` ${className}` : ''}`}
    style={style} data-value-state="missing-picture" data-tooltip={label}
    {...(decorative ? { 'aria-hidden': true } : { role: 'img', 'aria-label': label })} />;
}

export function MissingName({ reason, className = '' }: MissingNameProps) {
  const label = reason ? `Name not recorded: ${reason}` : 'Name not recorded';
  return <span className={`val-name-missing${className ? ` ${className}` : ''}`} data-value-state="missing-name"
    data-tooltip={reason} aria-label={label}>Name not recorded</span>;
}
