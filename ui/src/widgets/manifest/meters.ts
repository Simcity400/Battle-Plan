import { DEFAULT_VIEW, type FamilyManifestModule, type FamilyViewEntry, type FamilyViewModule } from '../../families/manifest.ts';
import type { AnyMeterWidgetDeclaration } from '../../families/meter/declaration.ts';
import { meterHostAdapter } from '../../families/meter/hostAdapter.ts';
import { familyMeterLaneView } from '../../families/meter/laneView.ts';
import type { MeterProjection } from '../../families/meter/model.ts';

function meterSpec<TModule>(
  kind: string,
  view: string,
  load: () => Promise<TModule>,
  pick: (mod: TModule) => { readonly projection: MeterProjection<any>; readonly declaration: AnyMeterWidgetDeclaration },
): () => Promise<FamilyViewModule> {
  return () => load().then(mod => {
    const { projection, declaration } = pick(mod);
    return { family: 'meters', kind, view, projection, declaration };
  });
}

export const METER_VIEWS: readonly FamilyViewEntry[] = [
  {
    kind: 'active-research',
    view: DEFAULT_VIEW,
    family: 'meters',
    loadSpec: meterSpec(
      'active-research',
      DEFAULT_VIEW,
      () => import('../active-research/activeResearchProjection.ts'),
      mod => ({ projection: mod.activeResearchProjection, declaration: mod.activeResearchDeclaration }),
    ),
  },
  {
    kind: 'doctrine-mastery',
    view: DEFAULT_VIEW,
    family: 'meters',
    loadSpec: meterSpec(
      'doctrine-mastery',
      DEFAULT_VIEW,
      () => import('../doctrine-mastery/doctrineMasteryProjection.ts'),
      mod => ({ projection: mod.doctrineMasteryProjection, declaration: mod.doctrineMasteryDeclaration }),
    ),
  },
  {
    kind: 'decryption',
    view: 'ciphers',
    label: 'Ciphers',
    family: 'meters',
    viewOption: 'view',
    loadSpec: meterSpec(
      'decryption',
      'ciphers',
      () => import('../decryption/decryptionProjection.ts'),
      mod => ({ projection: mod.decryptionProjection, declaration: mod.decryptionDeclaration }),
    ),
  },
];

export const metersFamilyManifest: FamilyManifestModule = {
  family: 'meters',
  laneView: familyMeterLaneView,
  views: METER_VIEWS,
  hostAdapter: meterHostAdapter,
};
