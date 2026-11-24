import type { ComponentType } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { FAMILY_MANIFESTS } from '../../widgets/familyManifest.ts';
import type { FamilyViewModule } from '../manifest.ts';
import type { FamilyId } from '../plan.ts';
import TableChrome from '../table/TableChrome.tsx';
import type { FamilyHostAdapter } from './adapter.ts';
import DeclaredFamilyWidget, { type DeclaredFamilyWidgetDeclaration, type DeclaredFamilyWidgetProps } from './DeclaredFamilyWidget.tsx';
import type { FamilyChromeProps } from './FamilyHost.tsx';

export interface FamilyHostProps extends WidgetProps {
  readonly spec: FamilyViewModule;
}

const CHROME: Readonly<Record<string, ComponentType<FamilyChromeProps<any>>>> = {
  table: TableChrome,
};

const adapters = new Map<FamilyId, FamilyHostAdapter<any, any, any, any>>();
for (const manifest of FAMILY_MANIFESTS) {
  if (manifest.hostAdapter) adapters.set(manifest.family, manifest.hostAdapter);
}

export function familyChrome(family: FamilyId): ComponentType<FamilyChromeProps<any>> | undefined {
  return Object.hasOwn(CHROME, family) ? CHROME[family] : undefined;
}

export interface FamilyWidgetProps extends Omit<DeclaredFamilyWidgetProps<any, any, any, any, any>, 'adapter' | 'chrome' | 'declaration'> {
  readonly family: FamilyId;
  readonly declaration: unknown;
}

export function FamilyWidget({ family, declaration, ...props }: FamilyWidgetProps) {
  const adapter = adapters.get(family);
  if (!adapter) return null;
  const chrome = familyChrome(family);
  return (
    <DeclaredFamilyWidget
      {...props}
      adapter={adapter}
      {...(chrome ? { chrome } : {})}
      declaration={declaration as DeclaredFamilyWidgetDeclaration<any, any, any>}
    />
  );
}

function AdaptedFamilyHost({ spec, ...props }: FamilyHostProps) {
  return <FamilyWidget {...props} family={spec.family} view={spec.view} declaration={spec.declaration} />;
}

export function familyHost(family: FamilyId): ComponentType<FamilyHostProps> | undefined {
  return adapters.has(family) ? AdaptedFamilyHost : undefined;
}
