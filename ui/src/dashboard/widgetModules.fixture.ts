import { ensureFamilyProjection } from '../families/registry.ts';
import { FAMILY_MANIFESTS, FAMILY_VIEWS } from '../widgets/familyManifest.ts';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import { widgetDefinitions } from './definitions';

export async function loadEveryWidgetModule(): Promise<number> {
  const definitions = widgetDefinitions();
  await Promise.allSettled([
    ...definitions.map(definition => loadWidgetRenderer(definition.kind)),
    ...FAMILY_VIEWS.map(entry => loadWidgetRenderer(entry.kind, entry.view)),
    ...FAMILY_VIEWS.map(entry => new Promise<void>(resolve => {
      if (ensureFamilyProjection(entry.family, entry.kind, entry.view, resolve) !== 'loading') resolve();
    })),
    ...FAMILY_MANIFESTS.map(manifest => manifest.lane()),
  ]);
  return definitions.length;
}
