import { type Failure, asFailure, failureText } from './queryError.ts';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { loadCountryGroups } from './products';
import { useContentRevision } from './contentRevision';
import { FactRequests, HourStore, nameKey, readFacts } from './countryFacts';
import { factionFounders, founderReads, leaderColours } from './factionLeaders';
import { STYLES } from './useCountriesAt';
import type { Rgb } from '../shell/appearance/color';
import { Icon } from '../components/Icons';
import { factionColours } from '../charts/factionColours';
import { neutralColour } from '../charts/palette.ts';
import { readTokens, subscribeTokens } from '../charts/tokens';
import type { DashboardClient, DashboardSource } from './client';
import { chosenColour, factionIdentity, saveCampaignSettings, setFactionColour, setFactionStyle, useCampaignSettings } from './campaignSettings';
import type { ColourScope } from './campaignSettings';
import { Segmented } from '../components/Segmented';
import type { CountryStyle, FactionGroup } from '../generated/dashboard';
import { factionNameAt, latestFactionSummaries } from './factionNames';
import './factionColours.css';

const FOUNDER_WAIT_MS = 10_000;

interface CampaignFactions {
  key: string; factions?: FactionGroup[]; leaderColours?: ReadonlyMap<string, Rgb | null>;
  founders?: ReadonlyMap<string, string>;
  warnings: string[]; error?: Failure;
}

function useCampaignFactions(client: DashboardClient, source: DashboardSource | undefined): CampaignFactions {
  const content = useContentRevision(client);
  const key = source ? `${source.id}:${source.revision}:${content}` : '';
  const [state, setState] = useState<CampaignFactions>({ key: '', warnings: [] });
  useEffect(() => {
    if (!source) return;
    const controller = new AbortController();
    const ids = { sourceId: source.id, revision: source.revision };
    Promise.all(source.metadata.branches.map(branch => loadCountryGroups(client, source, branch.id, controller.signal, false, () => {})))
      .then(async replies => {
        if (controller.signal.aborted) return;
        const factions = latestFactionSummaries(replies);
        const warnings = [...new Set(replies.flatMap(reply => reply.warnings))];
        const founders = factionFounders(replies);
        const read = await Promise.all([...founderReads(founders)].map(([branchId, wanted]) => new Promise<ReadonlyMap<string, CountryStyle | null>>(resolve => {
          const store = new HourStore<CountryStyle>(0);
          const hours = new Set(wanted.map(item => item.hour)).size;
          let finished = false;
          let deadline: ReturnType<typeof setTimeout> | undefined;
          let requests: FactRequests<CountryStyle> | null = null;
          const finish = () => {
            if (finished) return;
            finished = true;
            clearTimeout(deadline);
            requests?.cancel();
            resolve(store.pickRead(wanted));
          };
          const done = () => { if (wanted.every(item => store.answered(item.hour, item.tag))) finish(); };
          requests = new FactRequests<CountryStyle>(store, (plan, signal, onChunk) => readFacts(client, { ...ids, branchId, content }, STYLES, plan, signal, onChunk), done);
          controller.signal.addEventListener('abort', finish, { once: true });
          deadline = setTimeout(finish, FOUNDER_WAIT_MS);
          requests.want(wanted, hours, STYLES.perRequest);
          done();
        })));
        if (controller.signal.aborted) return;
        const byBranch = new Map([...founderReads(founders).keys()].map((branchId, index) => [branchId, read[index]]));
        const leaderColoursRead = leaderColours(founders, founder => byBranch.get(founder.branchId)?.get(nameKey(founder.hour, founder.tag)));
        const missing = [...leaderColoursRead.values()].filter(colour => !colour).length;
        if (missing) warnings.push(`The leader's map colour of ${missing} ${missing === 1 ? 'faction' : 'factions'} wasn’t recorded or couldn’t be read in time; ${missing === 1 ? 'it takes' : 'they take'} a palette colour until the campaign is opened again.`);
        setState({ key, factions, leaderColours: leaderColoursRead, founders: new Map([...founders].map(([id, founder]) => [id, founder.tag])), warnings });
      }).catch(error => { if (!controller.signal.aborted) setState({ key, warnings: [], error: asFailure(error) }); });
    return () => controller.abort();
  }, [client, source, key, content]);
  return state.key === key ? state : { key, warnings: [] };
}

export function useFactionColours(client: DashboardClient, source: DashboardSource | undefined) {
  const settings = useCampaignSettings();
  const campaign = useCampaignFactions(client, source);
  const [surface, setSurface] = useState(() => readTokens().surface);
  useEffect(() => subscribeTokens(tokens => setSurface(tokens.surface)), []);
  const campaignKey = source?.metadata.campaignKey ?? '';
  const own = useMemo(() => settings.campaigns[campaignKey]?.factions ?? {}, [settings, campaignKey]);
  const factions = useMemo(() => campaign.factions ?? [], [campaign.factions]);
  const leaders = campaign.leaderColours;
  const founders = campaign.founders;
  const identities = useMemo(() => new Map(factions.map(faction => [faction.id, factionIdentity(faction.recordedName, founders?.get(faction.id) ?? null)])), [factions, founders]);
  const colours = useMemo(() => factionColours(factions.map(faction => ({ id: faction.id, leaderColor: leaders?.get(faction.id) ?? null })),
    Object.fromEntries(factions.map(faction => [faction.id, chosenColour(settings, campaignKey, faction.id, identities.get(faction.id) ?? null)])), surface),
  [factions, leaders, settings, campaignKey, identities, surface]);
  const names = useCallback((faction: FactionGroup, hour?: number) => factionNameAt(faction, hour, own[faction.id]?.name), [own]);
  const displayNames = useMemo(() => new Map(Object.entries(own).flatMap(([id, style]) => style.name ? [[id, style.name] as const] : [])), [own]);
  const identity = (id: string) => identities.get(id) ?? null;
  const founder = (id: string) => founders?.get(id) ?? null;
  const setColour = (id: string, hex: string | null, scope: ColourScope) =>
    saveCampaignSettings(setFactionColour(settings, scope, campaignKey, id, identity(id), hex));
  return { factions, colours, names, displayNames, identity, founder, setColour, loading: !campaign.factions && !campaign.error, warnings: campaign.warnings, error: campaign.error };
}

export function FactionColoursEditor({ client, source }: { client: DashboardClient; source: DashboardSource }) {
  const settings = useCampaignSettings();
  const campaign = source.metadata.campaignKey;
  const { factions, colours, identity, founder, setColour, loading, warnings, error } = useFactionColours(client, source);
  const unnamed = (id: string) => {
    const tag = founder(id);
    const name = tag ? source.metadata.countries.find(country => country.tag === tag)?.name ?? 'Country name unavailable' : null;
    return name ? `Unnamed faction (${name})` : 'Unnamed faction';
  };
  const chosen = settings.campaigns[campaign]?.factions ?? {};
  const [scope, setScope] = useState<ColourScope>('campaign');
  const [message, setMessage] = useState<string | null>(null);
  const save = (next: typeof settings) => {
    const failure = saveCampaignSettings(next);
    setMessage(failure);
    return failure == null;
  };
  if (error) return <p className="field-hint" role="alert" data-detail={error.message}>{failureText(error, 'Couldn’t load factions.')}</p>;
  if (loading) return <p className="field-hint" role="status"><span className="spinner" /> Loading factions…</p>;
  if (!factions.length) return <p className="field-hint">{warnings.length ? 'Factions unavailable.' : 'No factions were recorded in this campaign.'}</p>;
  return <div className="faction-colours-body">
    <p className="field-hint">Names apply to this campaign; colours can apply to all campaigns.</p>
    <Segmented label="Colour changes apply to" value={scope} options={SCOPES} onChange={setScope} />
    <ul className="faction-colours-list">{factions.map(faction => {
      const style = chosen[faction.id];
      const named = factionNameAt(faction, undefined, style?.name);
      const recorded = factionNameAt(faction).name ?? unnamed(faction.id);
      const shared = identity(faction.id) != null ? settings.shared[identity(faction.id)!] : undefined;
      const set = scope === 'campaign' ? style?.colour : shared?.colour;
      return <li key={faction.id} data-faction={faction.id} data-faction-name={recorded}>
        <label className="faction-swatch" data-tooltip="Change colour" style={{ background: colours.get(faction.id) }}>
          <input type="color" value={colours.get(faction.id) ?? neutralColour('fallback', readTokens().surface)} aria-label={`Colour for ${recorded}`}
            onChange={event => setMessage(setColour(faction.id, event.currentTarget.value, scope))} /></label>
        { }
        <input key={style?.name ?? ''} className="faction-name" defaultValue={style?.name ?? ''} placeholder={named.name ?? unnamed(faction.id)} maxLength={160} aria-label={`Display name of ${recorded}`}
          onBlur={event => {
            const name = event.currentTarget.value.trim();
            if (name !== (style?.name ?? '')) save(setFactionStyle(settings, campaign, faction.id, name ? { name } : { name: undefined }));
          }} />
        {(set || scope === 'campaign' && style?.name) && <button type="button" className="icon-button small" aria-label={scope === 'campaign' ? 'Use the default colour and name in this campaign' : 'Use the default colour in every campaign'}
          data-tooltip="Reset" onClick={() => {
            const cleared = setFactionColour(settings, scope, campaign, faction.id, identity(faction.id), null);
            save(scope === 'campaign' ? setFactionStyle(cleared, campaign, faction.id, null) : cleared);
          }}><Icon name="refresh" /></button>}
      </li>;
    })}</ul>
    {message && <p className="field-hint" role="status">{message}</p>}
  </div>;
}

const SCOPES: readonly { value: ColourScope; label: string; tooltip: string }[] = [
  { value: 'all', label: 'All campaigns', tooltip: 'Use this colour in all campaigns.' },
  { value: 'campaign', label: 'This campaign only', tooltip: 'Override the colour for this campaign.' },
];
