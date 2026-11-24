import type { ComponentType } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { barsHostAdapter } from '../bars/hostAdapter.ts';
import { heatLanesHostAdapter, heatMatrixHostAdapter } from '../heat/hostAdapter.ts';
import type { FamilyViewModule } from '../manifest.ts';
import type { FamilyId } from '../plan.ts';
import DeclaredTableWidget from '../table/DeclaredTableWidget';
import type { AnyTableWidgetDeclaration } from '../table/declaration.ts';
import { timeChartHostAdapter } from '../time-chart/hostAdapter.ts';
import type { FamilyHostAdapter } from './adapter.ts';
import DeclaredFamilyWidget, { type DeclaredFamilyWidgetDeclaration } from './DeclaredFamilyWidget.tsx';

export interface FamilyHostProps extends WidgetProps {
  readonly spec: FamilyViewModule;
}

const hosts = new Map<FamilyId, ComponentType<FamilyHostProps>>();
const adapters = new Map<FamilyId, FamilyHostAdapter<any, any, any>>();

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

export function registerFamilyAdapter(family: FamilyId, adapter: FamilyHostAdapter<any, any, any>): void {
  adapters.set(family, adapter);
  hosts.set(family, AdaptedFamilyHost);
}

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
registerFamilyAdapter('bars', barsHostAdapter);
registerFamilyAdapter('time-chart', timeChartHostAdapter);
registerFamilyAdapter('series', timeChartHostAdapter);
registerFamilyAdapter('heat-matrix', heatMatrixHostAdapter);
registerFamilyAdapter('heat-lanes', heatLanesHostAdapter);
