import { Icon } from './Icons';
import type { IconName } from './Icons';
import '../charts/charts.css';

/** The standard states every widget uses, so they look the same everywhere:
 * loading, nothing yet, an error with Retry, and data this recording doesn't have. */
export type WidgetStateProps =
  | { kind: 'loading'; label: string }
  | { kind: 'empty'; message: string; detail?: string; icon?: IconName }
  | { kind: 'error'; message: string; onRetry?: () => void }
  | { kind: 'unavailable'; message?: string };

export default function WidgetState(props: WidgetStateProps) {
  switch (props.kind) {
    case 'loading':
      return <div className="widget-state" role="status" aria-label={props.label}><span className="skeleton skeleton-chart" aria-hidden="true" /></div>;
    case 'empty':
      return <div className="widget-state">{props.icon && <Icon name={props.icon} />}<span>{props.message}</span>{props.detail && <small>{props.detail}</small>}</div>;
    case 'error':
      return <div className="widget-state is-error" role="alert"><Icon name="alert" /><span>{props.message}</span>
        {props.onRetry && <button type="button" className="button small" onClick={props.onRetry}>Retry</button>}</div>;
    case 'unavailable':
      return <div className="widget-state"><Icon name="info" /><span>{props.message ?? 'Not in this recording.'}</span></div>;
  }
}
