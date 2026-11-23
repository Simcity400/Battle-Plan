export type ValueStateKind = 'known' | 'zero' | 'missing' | 'notApplicable' | 'partial';

export interface ValueStateProps {
  kind: ValueStateKind;
  text?: string;
  unit?: string;
  reason?: string;
  narrow?: boolean;
  atLeast?: boolean;
  className?: string;
}

export function valueStateText({ kind, text, unit, reason, atLeast }: Pick<ValueStateProps, 'kind' | 'text' | 'unit' | 'reason' | 'atLeast'>): string {
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
      const lead = atLeast ? `at least ${figure}` : figure;
      return reason ? `${lead}, partial: ${reason}` : `${lead}, partial`;
    }
  }
}
