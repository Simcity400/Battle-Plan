import type { CSSProperties } from 'react';
import './ValueState.css';

export type ValueStateData =
  | { kind: 'known'; text: string; unit?: string }
  | { kind: 'zero'; unit?: string }
  | { kind: 'missing'; reason: string }
  | { kind: 'notApplicable'; reason: string }
  | { kind: 'partial'; text: string; unit?: string; reason: string; atLeast?: boolean };

export interface ValueStateProps {
  value: ValueStateData;
  compact?: boolean;
  className?: string;
}

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

export function valueStateText(value: ValueStateData): string {
  switch (value.kind) {
    case 'known':
      return value.unit ? `${value.text} ${value.unit}` : value.text;
    case 'zero':
      return value.unit ? `0 ${value.unit}` : '0';
    case 'missing':
      return value.reason ? `Not recorded: ${value.reason}` : 'Not recorded';
    case 'notApplicable':
      return value.reason ? `Not applicable: ${value.reason}` : 'Not applicable';
    case 'partial': {
      const figure = value.unit ? `${value.text} ${value.unit}` : value.text;
      const lead = value.atLeast ? `at least ${figure}` : figure;
      return value.reason ? `${lead}, partial: ${value.reason}` : `${lead}, partial`;
    }
  }
}

export function ValueState({ value, compact = false, className = '' }: ValueStateProps) {
  const extra = className ? ` ${className}` : '';
  switch (value.kind) {
    case 'known':
      return <span className={`val-state val-known${extra}`} data-value-state="known">
        <span>{value.text}</span>{value.unit && <span className="val-unit">{value.unit}</span>}
      </span>;
    case 'zero':
      return <span className={`val-state val-zero${extra}`} data-value-state="zero">
        <span>0</span>{value.unit && <span className="val-unit">{value.unit}</span>}
      </span>;
    case 'missing': {
      const spoken = valueStateText(value);
      return <span className={`val-state val-missing${compact ? ' is-compact' : ''}${extra}`} data-value-state="missing"
        data-tooltip={spoken} aria-label={spoken}>
        <span className="val-hatch" aria-hidden="true" />
        <span className="val-missing-text" aria-hidden="true">Not recorded</span>
      </span>;
    }
    case 'notApplicable': {
      const spoken = valueStateText(value);
      return <span className={`val-state val-na${extra}`} data-value-state="not-applicable"
        data-tooltip={value.reason || 'Not applicable'} aria-label={spoken}>—</span>;
    }
    case 'partial': {
      const spoken = valueStateText(value);
      const tooltip = value.reason ? `Partial: ${value.reason}` : 'Partial';
      return <span className={`val-state val-partial${compact ? ' is-compact' : ''}${extra}`} data-value-state="partial"
        data-tooltip={tooltip} aria-label={spoken}>
        <span className="val-partial-figure">{value.text}{value.unit && <span className="val-unit">{value.unit}</span>}</span>
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
