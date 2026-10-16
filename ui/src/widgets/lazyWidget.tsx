import { createElement, lazy, Suspense } from 'react';
import type { ComponentType } from 'react';
import type { WidgetRenderer } from '../dashboard/definitions';
import { loadWidgetRenderer } from './rendererLoaders';
type Slot = 'render' | 'headerControls' | 'countryChip';
/** Components keep their chrome slots while code is fetched independently. */
export function lazyWidget(kind: string, slots: readonly Slot[]): WidgetRenderer {
  const renderer: WidgetRenderer = {};
  for (const slot of slots) {
    const Component = lazy(async () => ({ default: (await loadWidgetRenderer(kind))?.[slot] ?? (() => null) }));
    const wrapper = (props: object) => <Suspense fallback={slot === 'render' ? <div className="dash-empty" role="status"
      {...(import.meta.env.MODE === 'parity' ? { 'data-parity-loading': true } : {})}>Loading widget?</div> : null}>
      {createElement(Component as ComponentType<object>, props)}
    </Suspense>;
    Object.assign(renderer, { [slot]: wrapper });
  }
  return renderer;
}
