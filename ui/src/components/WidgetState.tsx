import type { ReactNode } from 'react';
import { failureText, RefusedError, type Failure } from '../dashboard/queryError.ts';
import { Icon } from './Icons';
import type { IconName } from './Icons';
import { isNoMembers } from '../dashboard/noMembers.ts';
import '../charts/charts.css';

export type WidgetStateProps =
  | { kind: 'loading'; label: string }
  | { kind: 'empty'; message: string; detail?: ReactNode; icon?: IconName }
  | { kind: 'error'; what?: string; failure?: Failure; onRetry?: () => void }
  /** `reason`: the missing data's detail, in the tooltip; the message says plainly what is missing. */
  | { kind: 'unavailable'; message?: string; detail?: ReactNode; reason?: string };

export default function WidgetState(props: WidgetStateProps) {
  switch (props.kind) {
    case 'loading':
      return <div className="widget-state" data-state="loading" role="status" aria-label={props.label}><span className="skeleton skeleton-chart" aria-hidden="true" /></div>;
    case 'empty':
      return <div className="widget-state" data-state="empty">{props.icon && <Icon name={props.icon} />}<span>{props.message}</span>{props.detail && <small>{props.detail}</small>}</div>;
    case 'error':
      return <div className="widget-state is-error" data-state="error" role="alert" data-detail={props.failure?.message}><Icon name="alert" /><span>{failureText(props.failure, props.what ? `Couldn’t load ${props.what}.` : 'Couldn’t load.')}</span>
        {props.onRetry && !(props.failure instanceof RefusedError) && <button type="button" className="button small" onClick={props.onRetry}>Retry</button>}</div>;
    case 'unavailable':
      // A recorded group with no one in it is an empty selection, not missing data.
      if (isNoMembers(props.message)) return <WidgetState kind="empty" message={props.message!} detail={props.detail} />;
      return <div className="widget-state" data-state="unavailable" data-tooltip={props.reason}><Icon name="info" /><span>{props.message ?? 'Not in this recording.'}</span>{props.detail && <small>{props.detail}</small>}</div>;
  }
}
