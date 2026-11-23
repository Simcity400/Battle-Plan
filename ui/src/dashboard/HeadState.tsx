import WidgetState from '../components/WidgetState';
import type { HeadProduct } from './headProduct';

export function headState(head: Pick<HeadProduct<unknown>, 'outside' | 'outsideMessage' | 'outsideDetail' | 'error' | 'retry' | 'data'> | undefined) {
  if (!head) return null;
  if (head.outside) return <WidgetState kind="outside" read={head} />;
  if (head.error && head.data === undefined) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  return null;
}
