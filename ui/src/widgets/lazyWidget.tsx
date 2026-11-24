import type { ComponentType } from 'react';
import { loadable } from '../components/loadable';
import WidgetState from '../components/WidgetState';
import type { FamilyWidgetDeclaration, WidgetProps, WidgetRenderer } from '../dashboard/definitions';
import { familyHost } from '../families/host/registry';
import type { FamilyViewModule } from '../families/manifest';
import { loadedWidgetRenderer, loadWidgetRenderer } from './rendererLoaders';

type Slot = 'render' | 'countryChip';
const nothing = () => null;
const declaredRenderers = new WeakMap<object, ComponentType<WidgetProps>>();

function rendererForDeclaration(declaration: FamilyWidgetDeclaration): ComponentType<WidgetProps> {
  let held = declaredRenderers.get(declaration);
  if (!held) {
    const spec: FamilyViewModule = {
      family: declaration.family,
      kind: declaration.kind,
      projection: declaration.projection,
      declaration,
    };
    held = (props: WidgetProps) => {
      const Host = familyHost(spec.family);
      return Host ? <Host spec={spec} {...props} /> : <WidgetState kind="unavailable" message={`Family host unavailable: ${spec.family}`} />;
    };
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
