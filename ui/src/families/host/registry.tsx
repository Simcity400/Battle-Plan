import type { ComponentType } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { FAMILY_MANIFESTS } from '../../widgets/familyManifest.ts';
import type { FamilyViewModule } from '../manifest.ts';
import type { FamilyId } from '../plan.ts';
import DeclaredTableWidget from '../table/DeclaredTableWidget';
import type { AnyTableWidgetDeclaration } from '../table/declaration.ts';
import type { FamilyHostAdapter } from './adapter.ts';
import DeclaredFamilyWidget, { type DeclaredFamilyWidgetDeclaration } from './DeclaredFamilyWidget.tsx';

export interface FamilyHostProps extends WidgetProps {
  readonly spec: FamilyViewModule;
}

const adapters = new Map<FamilyId, FamilyHostAdapter<any, any, any>>();
for (const manifest of FAMILY_MANIFESTS) {
  if (manifest.hostAdapter) adapters.set(manifest.family, manifest.hostAdapter);
}

function AdaptedFamilyHost({ spec, ...props }: FamilyHostProps) {
  const adapter = adapters.get(spec.family);
  if (!adapter) return null;
  return (
    <DeclaredFamilyWidget
      {...props}
      adapter={adapter}
      declaration={spec.declaration as DeclaredFamilyWidgetDeclaration<any, any, any>}
    />
  );
}

function TableFamilyHost({ spec, ...props }: FamilyHostProps) {
  return <DeclaredTableWidget declaration={spec.declaration as AnyTableWidgetDeclaration} {...props} />;
}

export function familyHost(family: FamilyId): ComponentType<FamilyHostProps> | undefined {
  if (adapters.has(family)) return AdaptedFamilyHost;
  return family === 'table' ? TableFamilyHost : undefined;
}
