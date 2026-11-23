import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Mounted { id: string; kind: string; track: 'early' | 'late'; follows: boolean; dated: boolean }
interface Frame { hour: number; widgets: Record<string, { state: string; renders: number }> }
const OUTSIDE = 'unavailable: This time is outside the recording.';
const NOT_PLAYING = "isn't playing back yet";

test('every registered widget kind through the production host, half on a track ending at 100 and half on one starting at 101, playing at 1,440 h/s: frames between boundary crossings render nothing, and on the frame that crosses every widget playing back dated says outside for the track it leaves and clears it for the track it enters', async () => {
  await withNativePage('outside-recording', '/ui/src/dashboard/outsideRecording.dom.fixture.tsx', page => page.stage('outside recording', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.outsideFixture.${expression}`);
    const { mounted } = await act<{ mounted: Mounted[] } & Frame>('start(95)');
    assert.equal(mounted.length, 114, 'one of each registered widget kind is mounted');
    const [within, crossed, after] = await act<Frame[]>('play(1440, 3, 2.5)');
    const rendered = (frame: Frame) => mounted.filter(item => frame.widgets[item.id].renders > 0).map(item => `${item.kind}: ${frame.widgets[item.id].renders}`);
    assert.ok(within.hour > 95 && within.hour <= 100 && crossed.hour > 101 && after.hour > crossed.hour, `one frame crosses both ends: ${within.hour}, ${crossed.hour}, ${after.hour}`);
    assert.deepEqual(rendered(within), [], `a frame inside every widget’s state renders nothing at ${within.hour}`);
    assert.deepEqual(rendered(after), [], `a frame after the crossing renders nothing at ${after.hour}`);
    const dated = mounted.filter(item => item.dated);
    assert.deepEqual(dated.filter(item => crossed.widgets[item.id].state.includes(NOT_PLAYING)).map(item => item.kind), [], 'every widget the registry plays back dated plays back');
    assert.ok(dated.some(item => item.track === 'early') && dated.some(item => item.track === 'late'), `widgets playing back dated on both tracks: ${dated.map(item => item.kind)}`);
    const wrong = dated.flatMap(item => {
      const before = within.widgets[item.id].state, at = crossed.widgets[item.id].state;
      const leaves = item.track === 'early';
      return before.includes(OUTSIDE) === leaves || at.includes(OUTSIDE) !== leaves ? [`${item.kind} on ${item.track}: ${before} → ${at}`] : [];
    });
    assert.deepEqual(wrong, [], 'the crossing frame says outside for the track left and clears it for the track entered');
  }, 240_000));
});

const STUBS: Record<string, string> = {
  '/widgets/land-battles/CombatLogDetails.tsx': 'export default () => null;',
  '/widgets/equipment-losses/lossesSource.ts': `export const stretchWindow = (_reply, window) => ({ ...window, since: null }); export const exactLossHour = () => true;
    export const useLossHistory = () => ({ data: globalThis.__fixture.losses, retry() {}, onSummaryView() {}, onHover() {}, navigate() {}, exactAt: async () => undefined });`,
  '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__fixture.groups, complete: true, retry() {} });',
  '/charts/sunburst/Sunburst.tsx': 'export default () => null;',
  '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  '/widgets/trade-flows/useTradeFlows.ts': `export const tradeFlowsRequest = () => null; export const useTradeWindows = () => ({ data: undefined, retry() {} });
    export const useTradeFlows = (_widget, runtime) => ({ track: runtime.tracks[0], all: true, resolved: null, groups: {}, scope: null, data: globalThis.__fixture.trade, error: undefined, retry() {}, extent: [100, 109] });`,
  '/charts/flow/FlowDiagram.tsx': `import { createElement } from 'react'; export default props => createElement('p', null, 'Diagram at ' + (props.live ? props.live.read().hour : props.hour));`,
};
const stubs = { name: 'outside-widgets', enforce: 'pre' as const, transform(_code: string, id: string) {
  return Object.entries(STUBS).find(([suffix]) => id.replaceAll('\\', '/').split('?')[0].endsWith(suffix))?.[1];
} };

const onPage = (run: (act: <T>(expression: string) => Promise<T>) => Promise<void>) => withNativePage('outside-widgets', '/ui/src/dashboard/outsideWidgets.dom.fixture.tsx',
  page => page.stage('outside widgets', () => run(expression => page.evaluate(`window.outsideWidgets.${expression}`)), 120_000), [stubs]);
const outside = OUTSIDE.split(': ')[1];

test('Trade flows kept mounted and stepped from 109 to 110, past its track, shows no diagram of 109 as current', () => onPage(async act => {
  const atEnd = await act<string>("mount('trade', 109)");
  assert.match(atEnd, /Diagram at 109/, `at 109 the current diagram is drawn: ${atEnd}`);
  const stepped = await act<{ hour: number; text: string }>("step('trade')");
  assert.ok(stepped.hour === 110 && stepped.text.includes(outside) && !/Diagram at/.test(stepped.text), `stepped to 110: ${JSON.stringify(stepped)}`);
}));

test('Equipment losses mounted at 99, before a country recorded 100–109, is outside the recording, and binds the country once the clock enters the track', () => onPage(async act => {
  const before = await act<string>("mount('losses', 99)");
  assert.ok(before.includes(outside) && !before.includes('Country unavailable'), `mounted at 99: ${before}`);
  const inside = await act<string>("seek('losses', 105)");
  assert.ok(/Losses: /.test(inside) && !inside.includes('Country unavailable'), `at 105 the country recorded since 100 is bound and its losses are drawn: ${inside}`);
}));
