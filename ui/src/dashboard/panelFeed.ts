import type { HeadProduct } from './headProduct';
import type { WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';
import type { PanelModel } from '../draw/panels/view';
import type { Pipeline } from './follow/pipeline';

export type PanelFeed = Omit<PanelModel, 'cells'> & { exactRead?: (hour: number) => Promise<unknown>; pipeline?: Pipeline | null; pipe?: number };
export function panelFeed<T>(view: string, head: HeadProduct<T>, shape: (reply: T) => string, widget: WidgetSpec, runtime: WidgetRuntime): PanelFeed | null {
  const shown = head;
  if (!shown.data) return null;
  const offset = runtime.tracks.find(t => t.id === widget.trackIds[0])?.axisOffset ?? 0;
  const at = shown.shownAt();
  return { view, follow: shown.workerFollow, offset, structure: shape(shown.data), exactRead: shown.exactRead,
    fixed: widget.time.mode === 'follow' ? undefined : shown.hour === null ? undefined : shown.hour - offset,
    date: at === null ? undefined : { since: at, through: at }, reply: shown.workerFollow === undefined ? shown.data : undefined,
    pipeline: head.pipeline, pipe: head.followedWidget };
}
