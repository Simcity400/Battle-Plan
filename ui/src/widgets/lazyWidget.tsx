import type { ComponentType } from 'react';
import { loadable } from '../components/loadable';
import WidgetState from '../components/WidgetState';
import type { WidgetProps, WidgetRenderer } from '../dashboard/definitions';
import DeclaredTableWidget from '../families/table/DeclaredTableWidget';
import type { AnyTableWidgetDeclaration } from '../families/table/declaration';
import { loadedWidgetRenderer, loadWidgetRenderer } from './rendererLoaders';

type Slot = 'render' | 'countryChip';
const nothing = () => null;
const declaredRenderers = new WeakMap<AnyTableWidgetDeclaration, ComponentType<WidgetProps>>();

function rendererForDeclaration(declaration: AnyTableWidgetDeclaration): ComponentType<WidgetProps> {
  let held = declaredRenderers.get(declaration);
  if (!held) {
    held = (props: WidgetProps) => <DeclaredTableWidget declaration={declaration} {...props} />;
    declaredRenderers.set(declaration, held);
  }
  return held;
}

export function lazyWidget(kind: string, slots: readonly Slot[]): WidgetRenderer {
  const renderer: WidgetRenderer = {};
  for (const slot of slots) {
    const pick = (loaded: WidgetRenderer | undefined) => {
      if (slot === 'render' && !loaded?.render && loaded?.declaration) {
        return rendererForDeclaration(loaded.declaration) as ComponentType<object>;
      }
      return (loaded?.[slot] as ComponentType<object> | undefined) ?? nothing;
    };
    const wrapper = loadable<object>(
      async () => pick(await loadWidgetRenderer(kind)),
      () => slot === 'render' ? <WidgetState kind="loading" label="Loading widget…" /> : null,
      () => { const loaded = loadedWidgetRenderer(kind); return loaded && pick(loaded); },
    );
    Object.assign(renderer, { [slot]: wrapper });
  }
  return renderer;
}
