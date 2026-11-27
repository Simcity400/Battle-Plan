import type { ComponentType } from 'react';
import type { FamilyViewProps } from '../../dashboard/definitions';
import type { FamilyViewModule } from '../manifest.ts';
import type { FamilyId } from '../plan.ts';
import TableChrome from '../table/TableChrome.tsx';
import { familyHostAdapters } from './adapters.ts';
import DeclaredFamilyWidget, { type DeclaredFamilyWidgetDeclaration, type DeclaredFamilyWidgetProps } from './DeclaredFamilyWidget.tsx';
import type { FamilyChromeProps } from './FamilyHost.tsx';

export type FamilyHostProps = FamilyViewProps & {
  readonly spec: FamilyViewModule;
};

const CHROME: Readonly<Record<string, ComponentType<FamilyChromeProps<any>>>> = {
  table: TableChrome,
};

export function familyChrome(family: FamilyId): ComponentType<FamilyChromeProps<any>> | undefined {
  return Object.hasOwn(CHROME, family) ? CHROME[family] : undefined;
}

export interface FamilyWidgetProps extends Omit<DeclaredFamilyWidgetProps<any, any, any, any, any>, 'adapter' | 'chrome' | 'declaration'> {
  readonly family: FamilyId;
  readonly declaration: unknown;
}

export function FamilyWidget({ family, declaration, ...props }: FamilyWidgetProps) {
  const adapter = familyHostAdapters.get(family);
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
  return familyHostAdapters.has(family) ? AdaptedFamilyHost : undefined;
}
