import type { ComponentType } from 'react';
import { loadable } from '../components/loadable';
import type { WidgetRenderer } from '../dashboard/definitions';
import { loadWidgetRenderer } from './rendererLoaders';
type Slot = 'render' | 'headerControls' | 'countryChip';
const nothing = () => null;
/** Components keep their chrome slots while code is fetched independently. */
export function lazyWidget(kind: string, slots: readonly Slot[]): WidgetRenderer {
  const renderer: WidgetRenderer = {};
  for (const slot of slots) {
    const wrapper = loadable<object>(async () => ((await loadWidgetRenderer(kind))?.[slot] as ComponentType<object> | undefined) ?? nothing,
      () => slot === 'render' ? <div className="dash-empty" role="status"
        {...(import.meta.env.MODE === 'parity' ? { 'data-parity-loading': true } : {})}>Loading widget…</div> : null);
    Object.assign(renderer, { [slot]: wrapper });
  }
  return renderer;
}
