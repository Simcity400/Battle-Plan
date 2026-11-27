import { useLayoutEffect, useMemo, type ComponentType } from 'react';
import { loadable } from '../components/loadable';
import WidgetState from '../components/WidgetState';
import DefaultCountryChip from '../dashboard/CountryChip';
import type { FamilyViewProps, FamilyWidgetDeclaration, WidgetProps, WidgetRenderer } from '../dashboard/definitions';
import { familyHost } from '../families/host/registry';
import type { FamilyViewModule } from '../families/manifest';
import type { WidgetSpec } from '../dashboard/model';
import { loadedWidgetRenderer, loadWidgetRenderer } from './rendererLoaders';
import { viewSlotProps } from './viewProps.ts';
import { busy } from '../shell/busy.ts';

type Slot = 'render' | 'countryChip' | 'subjectMenu';
const nothing = () => null;
function RendererLoading({ active }: { active: boolean }) {
  useLayoutEffect(() => active ? busy('waits') : undefined, [active]);
  return <WidgetState kind="loading" label="Loading widget…" />;
}
const declaredRenderers = new WeakMap<object, Map<string, ComponentType<FamilyViewProps>>>();

function rendererForDeclaration(declaration: FamilyWidgetDeclaration, view: string): ComponentType<FamilyViewProps> {
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
    held = (props: FamilyViewProps) => {
      const Host = familyHost(spec.family);
      return Host ? <Host spec={spec} {...props} /> : <WidgetState kind="unavailable" message={`Family host unavailable: ${spec.family}`} />;
    };
    byView.set(view, held);
  }
  return held;
}

const viewedRenderers = new WeakMap<ComponentType<WidgetProps>, WeakMap<FamilyWidgetDeclaration, Map<string, ComponentType<WidgetProps>>>>();

function rendererForView(render: ComponentType<WidgetProps>, view: string, declaration: FamilyWidgetDeclaration): ComponentType<WidgetProps> {
  let byDeclaration = viewedRenderers.get(render);
  if (!byDeclaration) viewedRenderers.set(render, byDeclaration = new WeakMap());
  let byView = byDeclaration.get(declaration);
  if (!byView) byDeclaration.set(declaration, byView = new Map());
  let held = byView.get(view);
  if (!held) {
    const Render = render;
    held = (props: WidgetProps) => <Render {...props} view={view} manifestDeclaration={declaration} />;
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
      if (slot === 'render' && loaded?.render && loaded.declaration) return rendererForView(loaded.render, view, loaded.declaration) as ComponentType<object>;
      return (loaded?.[slot] as ComponentType<object> | undefined) ?? nothing;
    };
    const byView = new Map<string, ComponentType<object>>();
    const forView = (view: string) => {
      let held = byView.get(view);
      if (!held) {
        held = loadable<object>(
          async () => pick(view, await loadWidgetRenderer(kind, view)),
          props => slot === 'render' ? <RendererLoading active={'active' in props && props.active === true} /> : null,
          () => { const loaded = loadedWidgetRenderer(kind, view); return loaded && pick(view, loaded); },
        );
        byView.set(view, held);
      }
      return held;
    };
    const ViewSlot = (props: { readonly widget?: WidgetSpec }) => {
      const handed = useMemo(() => viewSlotProps(kind, props), [props]);
      const Slot = forView(handed.view);
      return <Slot {...handed} />;
    };
    Object.assign(renderer, { [slot]: ViewSlot });
  }
  return renderer;
}
