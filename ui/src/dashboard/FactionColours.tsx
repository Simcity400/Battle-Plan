import { useCallback, useEffect, useMemo, useState } from 'react';
import { loadCountryGroups } from './products';
import { useContentRevision } from './contentRevision';
import { FactRequests, HourStore, nameKey, readFacts } from './countryFacts';
import { factionFounders, founderReads, leaderColours } from './factionLeaders';
import { STYLES } from './useCountriesAt';
import type { Rgb } from '../shell/appearance/color';
import { Icon } from '../components/Icons';
import { factionColours } from '../charts/factionColours';
import { readTokens, subscribeTokens } from '../charts/tokens';
import type { DashboardClient, DashboardSource } from './client';
import { chosenColour, factionIdentity, saveCampaignSettings, setFactionColour, setFactionStyle, useCampaignSettings } from './campaignSettings';
import type { ColourScope } from './campaignSettings';
import { Segmented } from '../components/Segmented';
import type { CountryStyle, FactionGroup } from '../generated/dashboard';
import { factionNameAt, latestFactionSummaries } from './factionNames';
import './factionColours.css';

/** How long the founders' map colours may take to read before a faction still unread
 * keeps its palette colour (reads retry with backoff while they run). */
const FOUNDER_WAIT_MS = 10_000;

interface CampaignFactions {
  key: string; factions?: FactionGroup[]; leaderColours?: ReadonlyMap<string, Rgb | null>;
  /** Each faction's first recorded leader's tag, for its identity across campaigns. */
  founders?: ReadonlyMap<string, string>;
  warnings: string[]; error?: string;
}

/** Every faction recorded on any branch of the campaign (colours are campaign-wide),
 * the map colour of the leader each had when first recorded (`factionFounders`), and
 * the recording's warnings about group history it could not read. A leader colour
 * that cannot be read leaves that faction on the palette, with a warning. */
function useCampaignFactions(client: DashboardClient, source: DashboardSource | undefined): CampaignFactions {
  const content = useContentRevision(client);
  const key = source ? `${source.id}:${source.revision}:${content}` : '';
  const [state, setState] = useState<CampaignFactions>({ key: '', warnings: [] });
  useEffect(() => {
    if (!source) return;
    const controller = new AbortController();
    const ids = { sourceId: source.id, revision: source.revision };
    // Campaign colours and founders follow each branch's complete history.
    Promise.all(source.metadata.branches.map(branch => loadCountryGroups(client, source, branch.id, controller.signal, false, () => {})))
      .then(async replies => {
        if (controller.signal.aborted) return;
        const factions = latestFactionSummaries(replies);
        const warnings = [...new Set(replies.flatMap(reply => reply.warnings))];
        const founders = factionFounders(replies);
        // Read through the shared bounded store (#146's FactRequests): at most two reads at
        // once per branch, and a failed read (such as one the desktop rejected) backs off and
        // is tried again. A founder still unread after FOUNDER_WAIT_MS keeps the palette, with
        // a warning, rather than leaving the colours loading.
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
        if (missing) warnings.push(`The leader's map colour of ${missing} ${missing === 1 ? 'faction' : 'factions'} was not recorded or could not be read in time; ${missing === 1 ? 'it takes' : 'they take'} a palette colour until the campaign is opened again.`);
        setState({ key, factions, leaderColours: leaderColoursRead, founders: new Map([...founders].map(([id, founder]) => [id, founder.tag])), warnings });
      }).catch(error => { if (!controller.signal.aborted) setState({ key, warnings: [], error: error instanceof Error ? error.message : String(error) }); });
    return () => controller.abort();
  }, [client, source, key, content]);
  return state.key === key ? state : { key, warnings: [] };
}

/** Each faction's colour for charts: the user's choice (this campaign's own, else the
 * one set for every campaign, found by the faction's identity), else the default from
 * `factionColours`: the map colour of the leader the faction had when first recorded,
 * exactly, grey included, nudged slightly only where it clashes with another faction's;
 * a faction whose leader colour is not known takes the categorical palette. `identity`
 * gives a faction's identity across campaigns (`factionIdentity`), and `setColour`
 * saves a colour for every campaign or this one only, returning an error message if the
 * device refused. */
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
  /** The tag of the leader a faction had when first recorded, if known. */
  const founder = (id: string) => founders?.get(id) ?? null;
  const setColour = (id: string, hex: string | null, scope: ColourScope) =>
    saveCampaignSettings(setFactionColour(settings, scope, campaignKey, id, identity(id), hex));
  return { factions, colours, names, displayNames, identity, founder, setColour, loading: !campaign.factions && !campaign.error, warnings: campaign.warnings, error: campaign.error };
}

/** Campaign-wide faction colours and display names, edited from a chart's settings. */
export function FactionColoursEditor({ client, source }: { client: DashboardClient; source: DashboardSource }) {
  const settings = useCampaignSettings();
  const campaign = source.metadata.campaignKey;
  const { factions, colours, identity, founder, setColour, loading, warnings, error } = useFactionColours(client, source);
  // An unnamed faction is told apart by its first recorded leader, named as the recording's catalogue names it.
  const unnamed = (id: string) => {
    const tag = founder(id);
    const name = tag ? source.metadata.countries.find(country => country.tag === tag)?.name ?? 'Country name unavailable' : null;
    return name ? `Unnamed faction (${name})` : 'Unnamed faction';
  };
  const chosen = settings.campaigns[campaign]?.factions ?? {};
  const [scope, setScope] = useState<ColourScope>('campaign');
  const [message, setMessage] = useState<string | null>(null);
  /** Saves and returns whether it worked; a failure leaves its message showing. */
  const save = (next: typeof settings) => {
    const failure = saveCampaignSettings(next);
    setMessage(failure);
    return failure == null;
  };
  if (error) return <p className="field-hint" role="alert">Couldn’t load factions.</p>;
  if (loading) return <p className="field-hint" role="status"><span className="spinner" /> Loading factions…</p>;
  // With warnings, an empty list may be history the recording could not read, not no factions.
  if (!factions.length) return <p className="field-hint">{warnings.length ? 'Factions unavailable.' : 'No factions were recorded in this campaign.'}</p>;
  return <div className="faction-colours-body">
    <p className="field-hint">Names apply to this campaign; colours can apply to all campaigns.</p>
    <Segmented label="Colour changes apply to" value={scope} options={SCOPES} onChange={setScope} />
    <ul className="faction-colours-list">{factions.map(faction => {
      const style = chosen[faction.id];
      const named = factionNameAt(faction, undefined, style?.name);
      const shared = identity(faction.id) != null ? settings.shared[identity(faction.id)!] : undefined;
      const set = scope === 'campaign' ? style?.colour : shared?.colour;
      return <li key={faction.id}>
        <label className="faction-swatch" data-tooltip="Change colour" style={{ background: colours.get(faction.id) }}>
          <input type="color" value={colours.get(faction.id) ?? '#888888'} aria-label={`Colour for ${named.name ?? unnamed(faction.id)}`}
            onChange={event => setMessage(setColour(faction.id, event.currentTarget.value, scope))} /></label>
        {/* Keyed on the stored name, so a reset replaces any old text. */}
        <input key={style?.name ?? ''} className="faction-name" defaultValue={style?.name ?? ''} placeholder={named.name ?? unnamed(faction.id)} maxLength={160} aria-label="Display name"
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
