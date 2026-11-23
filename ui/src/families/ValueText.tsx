import type { Value } from './value.ts';
import { formatValueState, NOT_APPLICABLE_LABEL, NOT_RECORDED_LABEL, PARTIAL_LABEL, type ValueFormat } from './valueFormat.ts';
import './value.css';

export interface ValueStateProps {
  readonly kind: Value['kind'];
  readonly text?: string;
  readonly reason?: string;
  readonly narrow?: boolean;
}

function ValueStateStandIn({ kind, text, reason, narrow = false }: ValueStateProps) {
  switch (kind) {
    case 'known':
    case 'zero':
      return <span>{text ?? ''}</span>;
    case 'notRecorded':
      return <span className={`fv-missing${narrow ? ' is-narrow' : ''}`} aria-label={reason ? `${NOT_RECORDED_LABEL}: ${reason}` : NOT_RECORDED_LABEL} data-tooltip={reason}>{NOT_RECORDED_LABEL}</span>;
    case 'notApplicable':
      return <span className="fv-na" aria-label={reason ? `Not applicable: ${reason}` : 'Not applicable'} data-tooltip={reason}>{NOT_APPLICABLE_LABEL}</span>;
    case 'partial':
      return <span className={`fv-partial${narrow ? ' is-narrow' : ''}`} aria-label={reason ? `${text ?? ''}, ${PARTIAL_LABEL}: ${reason}` : `${text ?? ''}, ${PARTIAL_LABEL}`} data-tooltip={reason}>
        <span className="fv-partial-value">{text ?? ''}</span>
        {!narrow && <span className="fv-partial-word">{PARTIAL_LABEL}</span>}
      </span>;
  }
}

export interface ValueTextProps {
  readonly value: Value;
  readonly format?: ValueFormat;
  readonly narrow?: boolean;
}

export function ValueText({ value, format, narrow = false }: ValueTextProps) {
  const formatted = formatValueState(value, format);
  switch (formatted.kind) {
    case 'known':
    case 'zero':
      return <ValueStateStandIn kind={formatted.kind} text={formatted.text} narrow={narrow} />;
    case 'notRecorded':
    case 'notApplicable':
      return <ValueStateStandIn kind={formatted.kind} reason={formatted.reason} narrow={narrow} />;
    case 'partial':
      return <ValueStateStandIn kind="partial" text={formatted.text} reason={formatted.reason} narrow={narrow} />;
  }
}
