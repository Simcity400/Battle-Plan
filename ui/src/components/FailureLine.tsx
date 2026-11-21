import { Icon } from './Icons';

export function DismissButton({ onDismiss }: { onDismiss: () => void }) {
  return <button type="button" className="icon-button small dismiss-button" aria-label="Dismiss" data-tooltip="Dismiss" onClick={onDismiss}><Icon name="close" /></button>;
}

export function FailureLine({ message, detail, onRetry, onDismiss, retryDisabled = false, className, id, alert = true }: {
  message: string; detail?: string | null; onRetry?: () => void; onDismiss?: () => void; retryDisabled?: boolean; className?: string; id?: string; alert?: boolean;
}) {
  return <p id={id} className={className ? `failure-line ${className}` : 'failure-line'} role={alert ? 'alert' : undefined} data-detail={detail || undefined}>
    <Icon name="alert" /><span>{message}</span>
    {onRetry && <button type="button" className="button small quiet" disabled={retryDisabled} onClick={onRetry}>Try again</button>}
    {onDismiss && <DismissButton onDismiss={onDismiss} />}
  </p>;
}
