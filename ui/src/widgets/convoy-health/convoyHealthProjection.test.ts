import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { ConvoyHealthReply } from '../../generated/dashboard';
import { efficiencyColour, projectConvoyHealth } from './convoyHealthProjection.ts';

const SURFACE = '#161616';

function reply(lastHour: number): ConvoyHealthReply {
  return {
    branchId: '1', view: 'regions', tag: 'GER', firstHour: 0, lastHour, scale: 100000,
    known: [[{ firstHour: 0, lastHour }], [], []], unheld: [], gaps: [{ firstHour: 30, lastHour: 39 }],
    regions: [{
      region: 17, name: 'English Channel', missing: null, pinned: false, efficiency: [],
      cells: [
        { firstHour: 0, lastHour: 9, state: { kind: 'efficiency', value: 100000n } },
        { firstHour: 10, lastHour: 19, state: { kind: 'efficiency', value: 62000n } },
        { firstHour: 20, lastHour: 29, state: { kind: 'noTraffic' } },
        { firstHour: 30, lastHour: 39, state: { kind: 'notRecorded', reason: 'collector queue overflow' } },
        { firstHour: 40, lastHour, state: { kind: 'efficiency', value: 38000n } },
      ],
      readout: { kind: 'efficiency', value: 38000n },
    }],
    deals: [], leases: [], resources: [], warnings: [],
  };
}

const view = (hour: number, data: ConvoyHealthReply) => createBoundedReadView({ hour, panels: new Map([['convoys', data]]) });
const OPTIONS = { view: 'regions' as const, surface: SURFACE };

test('the efficiency scale runs from full at the first stop to empty at the last', () => {
  assert.equal(efficiencyColour(1), '#22364f');
  assert.equal(efficiencyColour(0), '#ff6b74');
  assert.notEqual(efficiencyColour(0.62), efficiencyColour(0.86));
});

test('no traffic is the empty lane, unrecorded hours keep their reason, and the readout is the latest efficiency', () => {
  const model = projectConvoyHealth(view(50, reply(50)), OPTIONS);
  const runs = model.rows[0]!.runs;
  assert.equal(runs[2]!.cell.kind, 'clear');
  assert.deepEqual(runs[3]!.cell, { kind: 'missing', reason: 'collector queue overflow' });
  const readout = model.rows[0]!.readout!;
  assert.equal(readout.kind === 'value' && readout.primary, '38%');
  assert.equal(readout.kind === 'value' && readout.secondary, undefined, 'the column shows the figure only');
  assert.equal(readout.kind === 'value' && readout.meaning, 'Efficiency', 'what it is goes to the tooltip, mirror and audit');
  assert.equal(readout.kind === 'value' && readout.tone, undefined, 'a damaged route is emphasised');
  const full = { ...reply(50), regions: [{ ...reply(50).regions[0]!, readout: { kind: 'efficiency' as const, value: 100000n } }] };
  const quiet = projectConvoyHealth(view(50, full), OPTIONS).rows[0]!.readout!;
  assert.equal(quiet.kind === 'value' && quiet.tone, 'muted', 'a full route is quiet');
  assert.equal(runs.at(-1)!.lastHour, 50);
});

test('a reply that reaches past the displayed hour, or for the other view, draws nothing', () => {
  assert.deepEqual(projectConvoyHealth(view(45, reply(50)), OPTIONS).rows, []);
  assert.deepEqual(projectConvoyHealth(view(50, reply(50)), { view: 'trade', surface: SURFACE }).rows, []);
});

test('a lend-lease route stays a status in the lane, the readout and the tooltip, never an efficiency', async () => {
  const { deriveLanesTooltip } = await import('../../families/heat/lanesModel.ts');
  const data: ConvoyHealthReply = {
    ...reply(50), view: 'trade', regions: [], gaps: [],
    known: [[], [], [{ firstHour: 0, lastHour: 50 }]],
    leases: [{ handle: '7', giver: [{ firstHour: 0, lastHour: 50, tag: 'USA', name: 'United States' }], bad: [[0, 19, 1], [20, 50, 0]], convoys: [] }],
  };
  const model = projectConvoyHealth(view(50, data), { view: 'trade', surface: SURFACE });
  const [bad, clear] = model.rows[0]!.runs;
  assert.equal(bad!.cell.kind === 'solid' && bad!.cell.label, 'Route bad');
  assert.deepEqual(clear!.cell, { kind: 'clear', label: 'Route clear' });
  assert.equal(model.rows[0]!.readout?.kind === 'value' && model.rows[0]!.readout.primary, 'Route clear');
  const tip = deriveLanesTooltip(model, 0, 10)!;
  assert.equal(tip.items?.[0]?.label, 'Route bad');
  assert.ok(!tip.items?.some(item => item.label === 'Efficiency'), 'no measured efficiency is claimed');
});
