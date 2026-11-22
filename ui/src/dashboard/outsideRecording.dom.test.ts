import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

interface Shown { hour: number; early: (string | null)[]; late: (string | null)[]; renders: number }
const OUTSIDE = 'unavailable: This time is outside the recording.';

test('114 widgets playing at 1,440 h/s across their tracks’ ends: the host shows outside the recording on the frame that crosses, clears it on the frame that enters, and renders nothing on frames within', async () => {
  await withNativePage('outside-recording', '/ui/src/dashboard/outsideRecording.dom.fixture.tsx', page => page.stage('outside recording', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.outsideFixture.${expression}`);
    const start = await act<Shown>('start(95)');
    assert.deepEqual([start.early, start.late], [['loading'], [OUTSIDE]], `at 95 the early track is read and the late one is outside: ${JSON.stringify(start)}`);
    const [within, crossed, after] = await act<Shown[]>('play(1440, 3, 2.5)');
    assert.ok(within.hour <= 100 && within.renders === 0 && within.early[0] === 'loading' && within.late[0] === OUTSIDE,
      `a frame inside both tracks’ states renders nothing: ${JSON.stringify(within)}`);
    assert.ok(crossed.hour > 100, `one frame crosses both boundaries: ${JSON.stringify(crossed)}`);
    assert.deepEqual([crossed.early, crossed.late], [[OUTSIDE], ['loading']],
      `on the crossing frame every early widget is outside the recording and every late widget waits on its read: ${JSON.stringify(crossed)}`);
    assert.ok(after.renders === 0 && after.early[0] === OUTSIDE && after.late[0] === 'loading', `later frames render nothing: ${JSON.stringify(after)}`);
  }, 120_000));
});

const STUBS: Record<string, string> = {
  '/widgets/land-battles/CombatLogDetails.tsx': 'export default () => null;',
  '/widgets/equipment-losses/lossesSource.ts': `export const stretchWindow = (_reply, window) => ({ ...window, since: null }); export const exactLossHour = () => true;
    export const useLossHistory = () => ({ data: globalThis.__fixture.losses, retry() {}, onSummaryView() {}, onHover() {}, navigate() {}, exactAt: async () => undefined });`,
  '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: { countryIdentities: globalThis.__fixture.identities }, complete: true, retry() {} });',
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
