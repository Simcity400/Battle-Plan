import WidgetState from '../components/WidgetState';
import type { HeadProduct } from './headProduct';

export const OUTSIDE_RECORDING = 'This time is outside the recording.';

export function headState(head: Pick<HeadProduct<unknown>, 'outside' | 'error' | 'retry' | 'data'> | undefined) {
  if (!head) return null;
  if (head.outside) return <WidgetState kind="unavailable" message={OUTSIDE_RECORDING} />;
  if (head.error && head.data === undefined) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  return null;
}
