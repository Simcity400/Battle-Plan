import type { HeadProduct } from './headProduct';
import type { WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';
import type { PanelModel } from '../draw/panels/view';

export type PanelFeed = Omit<PanelModel, 'cells'> & { exactRead?: (hour: number) => Promise<unknown> };
export function panelFeed<T>(view: string, head: HeadProduct<T>, shape: (reply: T) => string, widget: WidgetSpec, runtime: WidgetRuntime): PanelFeed | null {
  if (!head.data) return null;
  const offset = runtime.tracks.find(t => t.id === widget.trackIds[0])?.axisOffset ?? 0;
  const at = head.shownAt();
  return { view, follow: head.workerFollow, offset, structure: shape(head.data), exactRead: head.exactRead,
    fixed: widget.time.mode === 'follow' ? undefined : head.hour === null ? undefined : head.hour - offset,
    date: at === null ? undefined : { since: at, through: at }, reply: head.workerFollow === undefined ? head.data : undefined };
}
