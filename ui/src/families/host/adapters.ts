import { barsHostAdapter } from '../bars/hostAdapter.ts';
import { breakdownHostAdapter } from '../breakdown/hostAdapter.ts';
import { cardHostAdapter } from '../card/hostAdapter.ts';
import { flowHostAdapter } from '../flow/hostAdapter.ts';
import { heatLanesHostAdapter } from '../heat/hostAdapter.ts';
import { heatMatrixHostAdapter } from '../heat/hostAdapter.ts';
import { meterHostAdapter } from '../meter/hostAdapter.ts';
import { tableHostAdapter } from '../table/hostAdapter.ts';
import { timeChartHostAdapter } from '../time-chart/hostAdapter.ts';
import { timelineHostAdapter } from '../timeline/hostAdapter.ts';
import type { FamilyHostAdapter } from './adapter.ts';

export const familyHostAdapters = new Map<string, FamilyHostAdapter<any, any, any, any>>([
  ['bars', barsHostAdapter],
  ['breakdown', breakdownHostAdapter],
  ['card', cardHostAdapter],
  ['flow', flowHostAdapter],
  ['heat-lanes', heatLanesHostAdapter],
  ['heat-matrix', heatMatrixHostAdapter],
  ['meters', meterHostAdapter],
  ['table', tableHostAdapter],
  ['time-chart', timeChartHostAdapter],
  ['timeline', timelineHostAdapter],
]);
