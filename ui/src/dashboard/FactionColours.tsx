import { useEffect, useId, useMemo, useState } from 'react';
import { loadProduct } from './products';
import { validateCountryGroupReply } from './groupsReply';
import { Icon } from '../components/Icons';
import { Popover, PopoverHeading, usePopover } from '../components/Popover';
import { factionColours } from '../charts/factionColours';
import { readTokens, subscribeTokens } from '../charts/tokens';
import { newId } from '../shell/ids';
import type { DashboardClient, DashboardSource } from './client';
import { applyColourSet, saveCampaignSettings, saveColourSet, setFactionStyle, useCampaignSettings } from './campaignSettings';
import type { CountryGroupReply, FactionGroup } from '../generated/dashboard';
import { MAX_FACTIONS } from './campaignSettings';
import './factionColours.css';

interface CampaignFactions { key: string; factions?: FactionGroup[]; warnings: string[]; error?: string }

/** Every faction recorded on any branch of the campaign (colours are campaign-wide),
 * with the recording's warnings about group history it could not read. */
function useCampaignFactions(client: DashboardClient, source: DashboardSource | undefined): CampaignFactions {
  const key = source ? `${source.id}:${source.revision}` : '';
  const [state, setState] = useState<CampaignFactions>({ key: '', warnings: [] });
  useEffect(() => {
    if (!source) return;
    const controller = new AbortController();
    Promise.all(source.metadata.branches.map(branch => loadProduct<CountryGroupReply>(client,
      { sourceId: source.id, revision: source.revision, product: 'country-groups', params: { branchId: branch.id } }, controller.signal, validateCountryGroupReply)))
      .then(replies => {
        if (controller.signal.aborted) return;
        const byId = new Map<string, FactionGroup>();
        for (const faction of replies.flatMap(reply => reply.factions)) {
          const known = byId.get(faction.id);
          byId.set(faction.id, known ? { ...known, name: known.name ?? faction.name } : faction);
        }
        setState({ key, factions: [...byId.values()], warnings: [...new Set(replies.flatMap(reply => reply.warnings))] });
      }, error => { if (!controller.signal.aborted) setState({ key, warnings: [], error: error instanceof Error ? error.message : String(error) }); });
    return () => controller.abort();
  }, [client, source, key]);
  return state.key === key ? state : { key, warnings: [] };
}

/** Each faction's campaign colour for charts: the user's choice, else the default
 * from `factionColours`. Faction leaders are not recorded yet, so defaults come
 * from the categorical palette. */
export function useFactionColours(client: DashboardClient, source: DashboardSource | undefined) {
  const settings = useCampaignSettings();
  const campaign = useCampaignFactions(client, source);
  const [surface, setSurface] = useState(() => readTokens().surface);
  useEffect(() => subscribeTokens(tokens => setSurface(tokens.surface)), []);
  const chosen = source ? settings.campaigns[source.metadata.campaignKey]?.factions ?? {} : {};
  const factions = useMemo(() => campaign.factions ?? [], [campaign.factions]);
  const colours = useMemo(() => factionColours(factions.map(faction => ({ id: faction.id })),
    Object.fromEntries(Object.entries(chosen).map(([id, style]) => [id, style.colour])), surface), [factions, chosen, surface]);
  const names = (faction: FactionGroup) => chosen[faction.id]?.name ?? faction.name;
  return { factions, colours, names, loading: !campaign.factions && !campaign.error, warnings: campaign.warnings, error: campaign.error };
}

/** A campaign's faction colours and display names, and colour sets that carry them
 * to other campaigns. Opened from the campaign's row in the sidebar. */
export default function FactionColoursButton({ client, source, label }: { client: DashboardClient; source: DashboardSource; label: string }) {
  const id = useId();
  const menu = usePopover('start');
  return <>
    <button ref={menu.trigger} type="button" className="icon-button small campaign-factions" popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}
      aria-label={`Faction colours for ${label}`} data-tooltip="Faction colours"><Icon name="palette" /></button>
    <Popover menu={menu} id={id} className="faction-colours" aria-label={`Faction colours for ${label}`}>
      <PopoverHeading title="Faction colours" menu={menu} />
      {menu.open && <FactionColoursEditor client={client} source={source} />}
    </Popover>
  </>;
}

function FactionColoursEditor({ client, source }: { client: DashboardClient; source: DashboardSource }) {
  const settings = useCampaignSettings();
  const campaign = source.metadata.campaignKey;
  const { factions, colours, loading, warnings, error } = useFactionColours(client, source);
  const chosen = settings.campaigns[campaign]?.factions ?? {};
  const [setName, setSetName] = useState('');
  const [setId, setSetId] = useState('');
  const [message, setMessage] = useState<string | null>(null);
  /** Saves and returns whether it worked; a failure leaves its message showing. */
  const save = (next: typeof settings) => {
    if (next === settings) { setMessage(`Up to ${MAX_FACTIONS} factions per campaign can have their own colour or name.`); return false; }
    const failure = saveCampaignSettings(next);
    setMessage(failure);
    return failure == null;
  };
  if (error) return <p className="field-hint" role="alert">Factions unavailable: {error}</p>;
  if (loading) return <p className="field-hint" role="status"><span className="spinner" /> Reading the campaign's factions…</p>;
  // With warnings, an empty list may be history the recording could not read, not no factions.
  if (!factions.length) return <p className="field-hint">{warnings.length
    ? `No factions could be read from this campaign. ${warnings.join(' ')}` : 'No factions were recorded in this campaign.'}</p>;
  return <div className="faction-colours-body">
    <p className="field-hint">Colours apply to this campaign in every chart. Faction names come from the game; a display name here replaces it.</p>
    <ul className="faction-colours-list">{factions.map(faction => {
      const style = chosen[faction.id];
      return <li key={faction.id}>
        <label className="faction-swatch" data-tooltip="Change colour" style={{ background: colours.get(faction.id) }}>
          <input type="color" value={colours.get(faction.id) ?? '#888888'} aria-label={`Colour for ${style?.name ?? faction.name ?? 'unnamed faction'}`}
            onChange={event => save(setFactionStyle(settings, campaign, faction.id, { colour: event.currentTarget.value }))} /></label>
        {/* Keyed on the stored name, so a reset or an applied set replaces any old text. */}
        <input key={style?.name ?? ''} className="faction-name" defaultValue={style?.name ?? ''} placeholder={faction.name ?? 'Unnamed faction'} maxLength={160} aria-label="Display name"
          onBlur={event => {
            const name = event.currentTarget.value.trim();
            if (name !== (style?.name ?? '')) save(setFactionStyle(settings, campaign, faction.id, name ? { name } : { name: undefined }));
          }} />
        {style && <button type="button" className="icon-button small" aria-label="Use the default colour and name" data-tooltip="Reset"
          onClick={() => save(setFactionStyle(settings, campaign, faction.id, null))}><Icon name="refresh" /></button>}
      </li>;
    })}</ul>
    <form className="faction-colours-sets" onSubmit={event => {
      event.preventDefault();
      if (!setName.trim()) return;
      if (!save(saveColourSet(settings, `set-${newId().replaceAll('-', '').slice(0, 20)}`, setName, campaign, factions, colours))) return;
      setSetName('');
      setMessage('Saved. Apply it to another campaign from its faction colours.');
    }}>
      <input value={setName} onChange={event => setSetName(event.currentTarget.value)} placeholder="Name this colour set" maxLength={160} aria-label="Colour set name" />
      <button type="submit" className="button small" disabled={!setName.trim()}><Icon name="save" />Save set</button>
    </form>
    {settings.sets.length > 0 && <div className="faction-colours-sets">
      <select value={setId} onChange={event => setSetId(event.currentTarget.value)} aria-label="Saved colour set">
        <option value="">Apply a saved set…</option>
        {settings.sets.map(set => <option key={set.id} value={set.id}>{set.name}</option>)}
      </select>
      <button type="button" className="button small" disabled={!setId} onClick={() => {
        const set = settings.sets.find(item => item.id === setId);
        if (!set) return;
        const result = applyColourSet(settings, set, campaign, factions);
        const error = saveCampaignSettings(result.settings);
        setMessage(error ?? `${result.matched} of ${factions.length} factions matched by name.`);
      }}>Apply</button>
    </div>}
    {message && <p className="field-hint" role="status">{message}</p>}
  </div>;
}
