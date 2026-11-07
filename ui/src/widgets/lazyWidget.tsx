import type { ComponentType } from 'react';
import { loadable } from '../components/loadable';
import type { WidgetRenderer } from '../dashboard/definitions';
import { loadedWidgetRenderer, loadWidgetRenderer } from './rendererLoaders';
type Slot = 'render' | 'headerControls' | 'countryChip';
const nothing = () => null;
/** Components keep their chrome slots while code is fetched independently. */
export function lazyWidget(kind: string, slots: readonly Slot[]): WidgetRenderer {
  const renderer: WidgetRenderer = {};
  for (const slot of slots) {
    const pick = (renderer: WidgetRenderer | undefined) => (renderer?.[slot] as ComponentType<object> | undefined) ?? nothing;
    const wrapper = loadable<object>(async () => pick(await loadWidgetRenderer(kind)),
      () => slot === 'render' ? <div className="dash-empty" role="status"
        {...(import.meta.env.MODE === 'parity' ? { 'data-parity-loading': true } : {})}>Loading widget…</div> : null,
      () => { const renderer = loadedWidgetRenderer(kind); return renderer && pick(renderer); });
    Object.assign(renderer, { [slot]: wrapper });
  }
  return renderer;
}
