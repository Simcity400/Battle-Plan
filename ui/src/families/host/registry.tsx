import type { ComponentType } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FamilyViewModule } from '../manifest.ts';
import type { FamilyId } from '../plan.ts';
import DeclaredTableWidget from '../table/DeclaredTableWidget';
import type { AnyTableWidgetDeclaration } from '../table/declaration.ts';

export interface FamilyHostProps extends WidgetProps {
  readonly spec: FamilyViewModule;
}

const hosts = new Map<FamilyId, ComponentType<FamilyHostProps>>();

export function registerFamilyHost(family: FamilyId, host: ComponentType<FamilyHostProps>): void {
  hosts.set(family, host);
}

export function familyHost(family: FamilyId): ComponentType<FamilyHostProps> | undefined {
  return hosts.get(family);
}

export function registeredFamilyHosts(): FamilyId[] {
  return [...hosts.keys()];
}

function TableFamilyHostAdapter({ spec, ...props }: FamilyHostProps) {
  return <DeclaredTableWidget declaration={spec.declaration as AnyTableWidgetDeclaration} {...props} />;
}

registerFamilyHost('table', TableFamilyHostAdapter);
