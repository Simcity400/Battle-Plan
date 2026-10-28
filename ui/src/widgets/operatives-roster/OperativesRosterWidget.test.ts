import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, OperativeRosterReply } from '../../generated/dashboard';

test('registered roster identifies multiple native owners selected by one reused literal tag', async () => {
  const country = (tag: string): OperativeRosterReply['countries'][number] => ({ tag,
    identities: [{ firstHour: 10, lastHour: 20, slot: tag === 'X' ? 2 : 1, countryId: tag === 'X' ? 9 : 7 }],
    observed: [{ firstHour: 10, lastHour: 20, readHour: 10 }], operatives: [{ key: `operative:${tag}`, id: tag,
      joinedHour: 10, departedHour: null, departedReason: null, runs: [{ firstHour: 10, lastHour: 20, value: {
        name: `Operative ${tag}`, unnamed: false, nameMissing: null, portrait: null, portraitMissing: null,
        skill: 1, experienceRaw: '0', experienceText: '0', experienceRatio: null, experienceMissing: null,
        assignmentName: 'Idle', assignmentDuration: null, assignmentProgress: null, assignmentMissing: null,
        assignmentIcon: null, assignmentIconMissing: null, assignmentOperation: false, targetTag: null,
        captured: false, capturerTag: null, capturedHour: null, ownerTag: tag,
      } }] }],
  });
  const data: OperativeRosterReply = { branchId: '1', countries: [country('X'), country('Y')],
    operations: [], agencyAbsent: [], artwork: [], reasons: [], warnings: [] };
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [],
    capitulations: [], identityChanges: [], unobserved: { humans: [], factions: [], wars: [] }, warnings: [],
    countryIdentities: [
      { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 7 },
        { firstHour: 10, lastHour: 20, slot: 2, countryId: 9 }] },
      { tag: 'Y', identities: [{ firstHour: 10, lastHour: 20, slot: 1, countryId: 7 }] },
    ],
  };
  const state = { data, groups };
  const host = globalThis as typeof globalThis & { __rosterNativeOwners?: typeof state };
  host.__rosterNativeOwners = state;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, _request, _active, validate) =>
      ({data: validate(globalThis.__rosterNativeOwners.data), retry() {}});
      export const loadProduct = () => {throw new Error('Unexpected transient product request');};
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({data: globalThis.__rosterNativeOwners.groups});',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({labelAt: hour => tag =>
      ({name: tag, known: true, flag: 'flag-' + tag + '-at-' + hour})});`,
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 20;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({fontSize: {title: 16, small: 12}}); export const useViewportHeight = () => 1000;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'roster-native-owner-flags', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/charts/table/Table.tsx')) {
          // Render the actual operative column and shared CountryCell without viewport measurement.
          const declaration = 'export default function Table<T>';
          assert.ok(source.includes(declaration));
          return source.replace(declaration, 'function FullTable<T>') + `
            export default function MountedNames(props) {
              const column = props.columns.find(column => column.key === 'name');
              return <div>{props.rows.map(row => <div key={props.rowKey(row)}>{column.render(row)}</div>)}</div>;
            }`;
        }
      },
    }] });
  try {
    const { default: registered } = await vite.ssrLoadModule('/ui/src/widgets/operatives-roster/index.ts') as typeof import('./index.ts');
    const props = { widget: { id: 'roster', kind: 'operatives-roster', trackIds: ['track'],
      countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'campaign' }, options: {} }, runtime: {
      tracks: [{ id: 'track', selectedTag: null, axisOffset: 0, source: { id: 'source', revision: '1',
        metadata: { countries: [], playerCountryId: null } }, branch: { id: '1', firstHour: 0, lastHour: 20 } }],
      clock: { getHour: () => 20 }, client: {},
    }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(registered.render, props));
    const compared = render();
    assert.match(compared, /Operative X/);
    assert.match(compared, /Operative Y/);
    assert.match(compared, /src="flag-X-at-20"/);
    assert.match(compared, /src="flag-Y-at-20"/);
    assert.equal((compared.match(/class="art-fallback"/g) ?? []).length, 2);

    // A provisional holder selection shows known rows while its inventory grows.
    const provisional = { ...data, preparing: ['countries'],
      preparedCoverage: {firstHour:0,lastHour:9,complete:false}, pending:[{firstHour:10,lastHour:20}] };
    state.data = provisional;
    const partial = render();
    assert.match(partial, /Operative X/);
    assert.match(partial, /Operative Y/);
    assert.match(partial, /This roster is still being prepared/);
    // Identity selection can still be pending when numeric coverage reaches the day.
    const reached = {...provisional,preparedCoverage:{firstHour:0,lastHour:20,complete:false},pending:[]};
    state.data = reached;
    assert.match(render(), /Operative X/);
    assert.match(render(), /This roster is still being prepared/);
    const noneKnown = {...reached,countries:data.countries.map(country => ({...country,operatives:[]}))};
    state.data = noneKnown;
    assert.match(render(), /This roster is still being prepared/);
    assert.doesNotMatch(render(), /No operatives in the selected roster/);
    const completeEmpty = {...data,countries:noneKnown.countries};
    state.data = completeEmpty;
    assert.match(render(), /No operatives in the selected roster/);
    state.data = data;

    // Two aliases of one native owner must not manufacture a comparison.
    state.groups = { ...groups, countryIdentities: groups.countryIdentities.map(history => ({ ...history,
      identities: [{ firstHour: 0, lastHour: 20, slot: 1, countryId: 7 }] })) };
    props.widget.countries = { kind: 'tags', tags: ['X', 'Y'] };
    assert.doesNotMatch(render(), /table-country-flag/);

    // A departed owner's identity and flag use its last detail hour, before tag reuse.
    state.groups = groups;
    const departed = country('Y');
    departed.operatives[0].departedHour = 16;
    departed.operatives[0].runs[0].lastHour = 15;
    state.data = { ...data, countries: [country('X'), departed] };
    state.groups = { ...groups, countryIdentities: [groups.countryIdentities[0], { tag: 'Y', identities: [
      { firstHour: 10, lastHour: 15, slot: 1, countryId: 7 },
      { firstHour: 16, lastHour: 20, slot: 2, countryId: 9 },
    ] }] };
    props.widget.countries = { kind: 'tags', tags: ['X'] };
    const historical = render();
    assert.match(historical, /src="flag-X-at-20"/);
    assert.match(historical, /src="flag-Y-at-15"/);
    assert.doesNotMatch(historical, /src="flag-Y-at-20"/);
    departed.operatives[0].runs[0].value.ownerTag = null;
    assert.doesNotMatch(render(), /table-country-flag/);

    // Actual registered validation accepts one native actor's alias and return
    // while preserving the original supplying census and separate episode keys.
    const returning = country('X');
    returning.identities = [{ firstHour: 10, lastHour: 12, slot: 1, countryId: 7 },
      { firstHour: 15, lastHour: 20, slot: 1, countryId: 7 }];
    returning.observed = [{ firstHour: 10, lastHour: 12, readHour: 10 }, { firstHour: 15, lastHour: 20, readHour: 10 }];
    const first = returning.operatives[0];
    returning.operatives = [
      { ...first, key: 'X:first', runs: [{ ...first.runs[0], lastHour: 12 }] },
      { ...first, key: 'X:return', runs: [{ ...first.runs[0], firstHour: 15 }] },
    ];
    const alias = country('Y');
    alias.identities = [{ firstHour: 13, lastHour: 14, slot: 1, countryId: 7 }];
    alias.observed = [{ firstHour: 13, lastHour: 14, readHour: 10 }];
    alias.operatives[0].runs[0].firstHour = 13;
    alias.operatives[0].runs[0].lastHour = 14;
    state.data = { ...data, countries: [returning, alias] };
    state.groups = { ...groups, countryIdentities: state.data.countries.map(({ tag, identities }) => ({ tag, identities })) };
    assert.match(render(), /Operative X/);
  } finally {
    delete host.__rosterNativeOwners;
    await vite.close();
  }
});
