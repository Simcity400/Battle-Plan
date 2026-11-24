import type { ComponentType } from 'react';
import { loadable } from '../components/loadable';
import WidgetState from '../components/WidgetState';
import type { WidgetProps, WidgetRenderer } from '../dashboard/definitions';
import { familyHost } from '../families/host/registry';
import { normalizeView, type FamilyViewModule } from '../families/manifest';
import { activeFamilyView, loadedFamilyViewSpec, loadFamilyViewSpec } from './manifest';
import { loadedWidgetRenderer, loadWidgetRenderer } from './rendererLoaders';

type Slot = 'render' | 'countryChip';
const nothing = () => null;
const specRenderers = new WeakMap<FamilyViewModule, ComponentType<WidgetProps>>();
const viewRenderers = new Map<string, ComponentType<WidgetProps>>();
const kindRenderers = new Map<string, ComponentType<WidgetProps>>();

function rendererForSpec(spec: FamilyViewModule): ComponentType<WidgetProps> {
  let held = specRenderers.get(spec);
  if (!held) {
    held = (props: WidgetProps) => {
      const Host = familyHost(spec.family);
      return Host ? <Host spec={spec} {...props} /> : <WidgetState kind="unavailable" message={`Family host unavailable: ${spec.family}`} />;
    };
    specRenderers.set(spec, held);
  }
  return held;
}

function familyViewRenderer(kind: string, view: string): ComponentType<WidgetProps> {
  const key = `${kind}\u0000${view}`;
  let held = viewRenderers.get(key);
  if (!held) {
    held = loadable<WidgetProps>(
      () => loadFamilyViewSpec(kind, view).then(spec => (spec ? rendererForSpec(spec) : nothing)),
      () => <WidgetState kind="loading" label="Loading widget…" />,
      () => {
        const spec = loadedFamilyViewSpec(kind, view);
        return spec && rendererForSpec(spec);
      },
    );
    viewRenderers.set(key, held);
  }
  return held;
}

function familyViewsRenderer(kind: string): ComponentType<WidgetProps> {
  let held = kindRenderers.get(kind);
  if (!held) {
    held = (props: WidgetProps) => {
      const entry = activeFamilyView(kind, props.widget.options);
      if (!entry) return <WidgetState kind="unavailable" message="Widget renderer unavailable." />;
      const View = familyViewRenderer(kind, normalizeView(entry.view));
      return <View {...props} />;
    };
    kindRenderers.set(kind, held);
  }
  return held;
}

export function lazyWidget(kind: string, slots: readonly Slot[]): WidgetRenderer {
  const renderer: WidgetRenderer = {};
  for (const slot of slots) {
    const pick = (loaded: WidgetRenderer | undefined) => {
      if (slot === 'render' && !loaded?.render && loaded?.declaration) {
        return familyViewsRenderer(kind) as ComponentType<object>;
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
