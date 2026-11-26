import { ensureFamilyProjection } from '../families/registry.ts';
import { FAMILY_MANIFESTS, FAMILY_VIEWS } from '../widgets/familyManifest.ts';
import { FAMILY_VIEW_UI, NATIVE_WIDGETS, OPTION_CHOICES } from '../widgets/manifest';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import { widgetDefinitions } from './definitions';

export async function loadEveryWidgetModule(): Promise<number> {
  const definitions = widgetDefinitions();
  await Promise.allSettled([
    ...NATIVE_WIDGETS.map(entry => entry.load()),
    ...Object.values(FAMILY_VIEW_UI).map(entry => entry.load()),
    ...Object.values(OPTION_CHOICES).map(load => load()),
    ...FAMILY_VIEWS.map(entry => entry.loadSpec()),
    ...FAMILY_MANIFESTS.map(manifest => manifest.lane()),
  ]);
  await Promise.allSettled([
    ...definitions.map(definition => loadWidgetRenderer(definition.kind)),
    ...FAMILY_VIEWS.map(entry => loadWidgetRenderer(entry.kind, entry.view)),
    ...FAMILY_VIEWS.map(entry => new Promise<void>(resolve => {
      if (ensureFamilyProjection(entry.family, entry.kind, entry.view, resolve) !== 'loading') resolve();
    })),
  ]);
  return definitions.length;
}
