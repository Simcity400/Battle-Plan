import type { ComponentType } from 'react';
import { loadable } from '../components/loadable';
import WidgetState from '../components/WidgetState';
import type { WidgetRenderer } from '../dashboard/definitions';
import { loadedWidgetRenderer, loadWidgetRenderer } from './rendererLoaders';
type Slot = 'render' | 'countryChip';
const nothing = () => null;
export function lazyWidget(kind: string, slots: readonly Slot[]): WidgetRenderer {
  const renderer: WidgetRenderer = {};
  for (const slot of slots) {
    const pick = (renderer: WidgetRenderer | undefined) => (renderer?.[slot] as ComponentType<object> | undefined) ?? nothing;
    const wrapper = loadable<object>(async () => pick(await loadWidgetRenderer(kind)),
      () => slot === 'render' ? <WidgetState kind="loading" label="Loading widget…" /> : null,
      () => { const renderer = loadedWidgetRenderer(kind); return renderer && pick(renderer); });
    Object.assign(renderer, { [slot]: wrapper });
  }
  return renderer;
}
