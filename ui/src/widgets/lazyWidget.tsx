import type { ComponentType } from 'react';
import { loadable } from '../components/loadable';
import WidgetState from '../components/WidgetState';
import DefaultCountryChip from '../dashboard/CountryChip';
import type { FamilyWidgetDeclaration, WidgetProps, WidgetRenderer } from '../dashboard/definitions';
import { familyHost } from '../families/host/registry';
import type { FamilyViewModule } from '../families/manifest';
import type { WidgetSpec } from '../dashboard/model';
import { resolveWidgetActiveView } from './manifest';
import { loadedWidgetRenderer, loadWidgetRenderer } from './rendererLoaders';

type Slot = 'render' | 'countryChip';
const nothing = () => null;
const declaredRenderers = new WeakMap<object, Map<string, ComponentType<WidgetProps>>>();

function rendererForDeclaration(declaration: FamilyWidgetDeclaration, view: string): ComponentType<WidgetProps> {
  let byView = declaredRenderers.get(declaration);
  if (!byView) declaredRenderers.set(declaration, byView = new Map());
  let held = byView.get(view);
  if (!held) {
    const spec: FamilyViewModule = {
      family: declaration.family,
      kind: declaration.kind,
      view,
      projection: declaration.projection,
      declaration,
    };
    held = (props: WidgetProps) => {
      const Host = familyHost(spec.family);
      return Host ? <Host spec={spec} {...props} /> : <WidgetState kind="unavailable" message={`Family host unavailable: ${spec.family}`} />;
    };
    byView.set(view, held);
  }
  return held;
}

const viewedRenderers = new WeakMap<ComponentType<WidgetProps>, Map<string, ComponentType<WidgetProps>>>();

function rendererForView(render: ComponentType<WidgetProps>, view: string): ComponentType<WidgetProps> {
  let byView = viewedRenderers.get(render);
  if (!byView) viewedRenderers.set(render, byView = new Map());
  let held = byView.get(view);
  if (!held) {
    const Render = render;
    held = (props: WidgetProps) => <Render {...props} view={view} />;
    byView.set(view, held);
  }
  return held;
}

export function lazyWidget(kind: string, slots: readonly Slot[]): WidgetRenderer {
  const renderer: WidgetRenderer = {};
  for (const slot of slots) {
    const pick = (view: string, loaded: WidgetRenderer | undefined) => {
      if (slot === 'render' && !loaded?.render && loaded?.declaration) {
        return rendererForDeclaration(loaded.declaration, view) as ComponentType<object>;
      }
      if (slot === 'countryChip' && !loaded?.countryChip) {
        return DefaultCountryChip as ComponentType<object>;
      }
      if (slot === 'render' && loaded?.render) return rendererForView(loaded.render, view) as ComponentType<object>;
      return (loaded?.[slot] as ComponentType<object> | undefined) ?? nothing;
    };
    const byView = new Map<string, ComponentType<object>>();
    const forView = (view: string) => {
      let held = byView.get(view);
      if (!held) {
        held = loadable<object>(
          async () => pick(view, await loadWidgetRenderer(kind, view)),
          () => slot === 'render' ? <WidgetState kind="loading" label="Loading widget…" /> : null,
          () => { const loaded = loadedWidgetRenderer(kind, view); return loaded && pick(view, loaded); },
        );
        byView.set(view, held);
      }
      return held;
    };
    const ViewSlot = (props: { readonly widget?: WidgetSpec }) => {
      const Slot = forView(resolveWidgetActiveView(kind, props.widget?.options));
      return <Slot {...props} />;
    };
    Object.assign(renderer, { [slot]: ViewSlot });
  }
  return renderer;
}
