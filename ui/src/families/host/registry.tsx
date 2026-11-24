import type { ComponentType } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import type { AnyBarsWidgetDeclaration } from '../bars/declaration.ts';
import DeclaredBarsWidget from '../bars/DeclaredBarsWidget';
import type { AnyCardWidgetDeclaration } from '../card/declaration.ts';
import DeclaredCardWidget from '../card/DeclaredCardWidget';
import type { FamilyViewModule } from '../manifest.ts';
import type { FamilyId } from '../plan.ts';
import DeclaredTableWidget from '../table/DeclaredTableWidget';
import type { AnyTableWidgetDeclaration } from '../table/declaration.ts';
import type { TimeChartWidgetDeclaration } from '../time-chart/declaration.ts';
import DeclaredTimeChartWidget from '../time-chart/DeclaredTimeChartWidget';

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

function BarsFamilyHostAdapter({ spec, ...props }: FamilyHostProps) {
  return <DeclaredBarsWidget declaration={spec.declaration as AnyBarsWidgetDeclaration} {...props} />;
}

function CardFamilyHostAdapter({ spec, ...props }: FamilyHostProps) {
  return <DeclaredCardWidget declaration={spec.declaration as AnyCardWidgetDeclaration} {...props} />;
}

function TimeChartFamilyHostAdapter({ spec, ...props }: FamilyHostProps) {
  return <DeclaredTimeChartWidget declaration={spec.declaration as TimeChartWidgetDeclaration<any, any>} {...props} />;
}

registerFamilyHost('table', TableFamilyHostAdapter);
registerFamilyHost('bars', BarsFamilyHostAdapter);
registerFamilyHost('card', CardFamilyHostAdapter);
registerFamilyHost('time-chart', TimeChartFamilyHostAdapter);
registerFamilyHost('series', TimeChartFamilyHostAdapter);
