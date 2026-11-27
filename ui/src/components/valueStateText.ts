export type ValueStateKind = 'known' | 'zero' | 'missing' | 'notApplicable' | 'partial' | 'pending';

export interface ValueStateProps {
  kind: ValueStateKind;
  text?: string;
  unit?: string;
  reason?: string;
  narrow?: boolean;
  className?: string;
}

export function valueStateText({ kind, text, unit, reason }: Pick<ValueStateProps, 'kind' | 'text' | 'unit' | 'reason'>): string {
  switch (kind) {
    case 'known':
      return unit ? `${text ?? ''} ${unit}` : (text ?? '');
    case 'zero': {
      const zero = text ?? '0';
      return unit ? `${zero} ${unit}` : zero;
    }
    case 'missing':
      return reason ? `Not recorded: ${reason}` : 'Not recorded';
    case 'notApplicable':
      return reason ? `Not applicable: ${reason}` : 'Not applicable';
    case 'partial': {
      const figure = unit ? `${text ?? ''} ${unit}` : (text ?? '');
      return reason ? `${figure}, partial: ${reason}` : `${figure}, partial`;
    }
    case 'pending':
      return reason ? `Loading: ${reason}` : 'Loading…';
  }
}
