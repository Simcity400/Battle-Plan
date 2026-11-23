import { failureKey } from './queryError.ts';
import { useSyncExternalStore } from 'react';
import WidgetState from '../components/WidgetState';
import type { HeldFailures } from './heldFailures';

export default function HeldFailureNotices({ failures }: { failures: HeldFailures }) {
  const errors = useSyncExternalStore(failures.subscribe, failures.read, failures.read);
  if (!errors.length) return null;
  return <div className="dash-held-failures">
    {errors.map(error => <WidgetState key={failureKey(error)} scale="section" kind="error" failure={error} onRetry={failures.retryable(error) ? () => failures.retry(error) : undefined} />)}
  </div>;
}
