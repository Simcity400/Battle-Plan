import { Popover, usePopover } from '../components/Popover';
import { newId } from '../shell/ids.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { desktopAvailable, errorMessage, recordingName, seekMap, mapAction } from '../shell/api';
import type { MapOptions, ReplayMetadata } from '../shell/api';
import MapControls from '../components/MapControls';
import { Icon } from '../components/Icons';
import type { DashboardClient, DashboardSource } from './client';
import { addGridItem, addGridItemBelow, bindTracks, branchSelections, clockBounds, createWidget, defaultTemplate, MAX_WIDGETS, mergeViewPreferences, rebaseWidget, seekPositionFor, stableJson } from './model';
import type { DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { droppedTagWarning, loadLayouts, MAX_LAYOUTS, parseLayoutLibrary, parseTemplate, parseTemplateWithNotes, saveLayouts } from './storage';
import { PlaybackController } from './playback';
import { widgetDefinition } from './definitions';
import '../widgets';
import { MapViewsStore } from './geometry';
import { useSourceRoles } from './sources';
import { PlaybackCountryPicker, PlaybackStrip } from './controls';
import ReplayLibraryMenu from './ReplayLibraryMenu';
import type { ReplayLibraryState } from './replayLibrary';
import DashboardGrid from './DashboardGrid';
import './dashboard.css';
import WidgetPicker from './WidgetPicker';
import { campaignSlots, defaultAfterOpen, defaultAfterUnload, includeCampaigns, sameCampaignPath } from './campaigns';
import { hasMapOverlay, isMapOverlay } from '../shell/mapOverlays';
import type { Campaign } from './campaigns';
import { followMapPlayback } from './mapPlayback';

interface Props { library: ReplayLibraryState; path: string | null; client: DashboardClient; onPrimaryPath?: (path: string | null) => void; active?: boolean; mapActive?: boolean; mapSwitching?: boolean; mapReady?: boolean; onMetadata: (metadata: ReplayMetadata | null) => void; onMapError?: (error: string | null) => void; playbackHost?: HTMLElement | null; campaignHost?: HTMLElement | null; onOpenReady?: (open: (() => void) | null) => void }

export default function Dashboard({ path, client, library: replayLibrary, onPrimaryPath, active = true, mapActive = false, mapSwitching = false, mapReady = false, onMetadata, onMapError, playbackHost, campaignHost, onOpenReady }: Props) {
  const [restored] = useState(loadLayouts);
  const [library, setLibrary] = useState(restored.library);
  const libraryRef = useRef(library);
  libraryRef.current = library;
  const selectedLayout = library.layouts.find((entry) => entry.id === library.activeId)!;
  const [draftTemplate, setTemplate] = useState<DashboardTemplate>(selectedLayout.template);
  const templateRef = useRef(draftTemplate);
  templateRef.current = draftTemplate;
  const [campaigns, setCampaigns] = useState<Campaign[]>([]);
  const campaignsRef = useRef(campaigns);
  campaignsRef.current = campaigns;
  const [defaultRoleId, setDefaultRoleId] = useState<string | null>(null);
  const [selectedCountries, setSelectedCountries] = useState<Record<string, string | null>>({});
  const [selectedBranches, setSelectedBranches] = useState<Record<string, number>>({});
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({});
  const hasOpened = useRef(false);
  const [mapOptions, setMapOptions] = useState<MapOptions>({ country: null, fog: true });
  const template = useMemo(() => {
    const merged = includeCampaigns(draftTemplate, campaigns);
    return { ...merged, roles: merged.roles.map((role) => ({ ...role, selectedTag: selectedCountries[role.id] ?? null })),
      tracks: branchSelections(merged.tracks, selectedBranches) };
  }, [draftTemplate, campaigns, selectedCountries, selectedBranches]);
  const [warning, setWarning] = useState<string | null>(restored.warning);
  const [savedTemplate, setSavedTemplate] = useState<DashboardTemplate | null>(null);
  const saved = savedTemplate === draftTemplate ? 'Layout saved on this device' : null;
  const [savedVisible, setSavedVisible] = useState(false);
  useEffect(() => {
    if (!saved) return;
    setSavedVisible(true);
    const timer = setTimeout(() => setSavedVisible(false), 2800);
    return () => clearTimeout(timer);
  }, [saved, savedTemplate]);
  const [editing, setEditing] = useState(false);
  const saveMenu = usePopover();
  const saving = saveMenu.open;
  const [layoutName, setLayoutName] = useState('');
  const [saveError, setSaveError] = useState<string | null>(null);
  const saveButton = saveMenu.trigger;
  const editButton = useRef<HTMLButtonElement>(null);
  const dirty = useMemo(() => stableJson(includeCampaigns(draftTemplate, campaigns)) !== stableJson(includeCampaigns(selectedLayout.template, campaigns)), [draftTemplate, selectedLayout.template, campaigns]);
  const [actionError, setActionError] = useState<string | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const mounted = useRef(false);
  const roles = useSourceRoles(client);
  const clock = useMemo(() => new PlaybackController(), []);
  const maps = useMemo(() => new MapViewsStore(client, clock), [client, clock]);
  const sourceMap = useMemo(() => new Map(Object.entries(roles.bindings).flatMap(([roleId, binding]) => binding.source ? [[roleId, binding.source] as [string, DashboardSource]] : [])), [roles.bindings]);
  const tracks = useMemo(() => bindTracks(template, sourceMap), [template.tracks, template.roles, template.alignment, sourceMap]);
  const bounds = useMemo(() => clockBounds(tracks), [tracks]);
  const primaryIdentity = defaultRoleId && sourceMap.get(defaultRoleId) ? `${sourceMap.get(defaultRoleId)!.id}:${sourceMap.get(defaultRoleId)!.revision}` : '';
  const defaultLoading = campaigns.some(campaign => campaign.id === defaultRoleId && campaign.loading);
  const lastClockIdentity = useRef('');
  const nativeHidden = !active || editing || saving;
  const overlayBaseline = useRef(nativeHidden);
  overlayBaseline.current = nativeHidden;

  useLayoutEffect(() => {
    const identity = `${primaryIdentity}:${template.alignment}:${tracks.find(track => track.roleId === defaultRoleId)?.branch.id}`;
    const changed = identity !== lastClockIdentity.current;
    clock.setBounds(bounds, changed);
    const reference = tracks.find((track) => track.roleId === defaultRoleId);
    if (changed && reference) clock.seek(reference.branch.firstHour - reference.axisOffset);
    lastClockIdentity.current = identity;
    maps.schedule();
  }, [clock, maps, bounds, primaryIdentity, template.alignment, tracks, defaultRoleId]);

  useLayoutEffect(() => { if (!active && !mapActive) clock.stop(); }, [active, mapActive, clock]);
  const mapCountry = defaultRoleId ? selectedCountries[defaultRoleId] ?? null : null;
  useEffect(() => {
    if (!mapActive || !mapReady || mapSwitching || defaultLoading || !desktopAvailable) return;
    let current = true;
    void mapAction({ kind: 'configure', options: { ...mapOptions, country: mapCountry } }).catch(cause => {
      if (current) onMapError?.(errorMessage(cause));
    });
    return () => { current = false; };
  }, [mapActive, mapReady, mapSwitching, defaultLoading, mapOptions, mapCountry, onMapError]);
  useLayoutEffect(() => {
    const reference = tracks.find(track => track.roleId === defaultRoleId);
    if (!mapActive || mapSwitching || defaultLoading || !desktopAvailable || !reference) return;
    onMapError?.(null);
    return followMapPlayback(clock, reference, seekMap, error => {
      if (error != null) clock.stop();
      onMapError?.(error == null ? null : errorMessage(error));
    });
  }, [clock, tracks, defaultRoleId, defaultLoading, mapActive, mapSwitching, onMapError]);
  useLayoutEffect(() => active && viewport.current ? maps.start(viewport.current) : undefined, [maps, active]);
  useLayoutEffect(() => { maps.setHidden(nativeHidden || hasMapOverlay()); maps.schedule(); }, [maps, nativeHidden, template.layouts]);
  useEffect(() => {
    mounted.current = true;
    const visibility = () => { if (document.hidden) clock.setPlaying(false); };
    const beforeToggle = (event: Event) => {
      if ('newState' in event && event.newState === 'open' && isMapOverlay(event.target)) maps.setHidden(true);
    };
    const afterToggle = () => maps.setHidden(overlayBaseline.current || hasMapOverlay());
    document.addEventListener('visibilitychange', visibility);
    document.addEventListener('beforetoggle', beforeToggle, true);
    document.addEventListener('toggle', afterToggle, true);
    return () => {
      mounted.current = false;
      document.removeEventListener('visibilitychange', visibility);
      document.removeEventListener('beforetoggle', beforeToggle, true);
      document.removeEventListener('toggle', afterToggle, true);
      clock.stop();
    };
  }, [clock, maps]);

  const update = useCallback((change: (previous: DashboardTemplate) => DashboardTemplate) => {
    setTemplate((previous) => change(includeCampaigns(previous, campaignsRef.current)));
  }, []);
  /** Saves a widget's settings-panel choices to the active layout at once. A choice the
   * saved layout cannot hold yet (a newly opened campaign) stays a draft until Save. */
  const persistView = useCallback((previous: WidgetSpec, widget: WidgetSpec) => {
    const current = libraryRef.current;
    const next = { ...current, layouts: current.layouts.map(entry => {
      if (entry.id !== current.activeId) return entry;
      const tracks = new Set(entry.template.tracks.map(track => track.id));
      return { ...entry, template: { ...entry.template,
        widgets: entry.template.widgets.map(item => item.id === widget.id ? mergeViewPreferences(item, previous, widget, tracks) : item) } };
    }) };
    if (!parseLayoutLibrary(next)) { setWarning('This setting applies now and is saved with the layout when you press Save.'); return; }
    const error = saveLayouts(next);
    if (error) { setWarning(error); return; }
    libraryRef.current = next;
    setLibrary(next);
  }, []);
  const applyWidget = useCallback((widget: WidgetSpec, view = false) => {
    // The widget the change was made from, as last drawn: only what the change altered
    // is applied to the latest one, so a second change made before it is drawn again
    // (two options saved at once) never undoes the first.
    const before = templateRef.current.widgets.find(item => item.id === widget.id);
    update((previous) => ({ ...previous, widgets: previous.widgets.map((item) => item.id === widget.id ? (before ? rebaseWidget(item, before, widget) : widget) : item) }));
    if (view && before) persistView(before, widget);
  }, [update, persistView]);
  const removeWidget = useCallback((id: string) => update((previous) => ({ ...previous,
    widgets: previous.widgets.filter((widget) => widget.id !== id),
    layouts: Object.fromEntries(Object.entries(previous.layouts).map(([key, layout]) => [key, layout.filter((item) => item.i !== id)])),
  })), [update]);
  const layoutsChanged = useCallback((layouts: GridLayouts) => {
    if (!editing) return;
    setTemplate((previous) => {
      const merged = { ...previous.layouts, ...layouts };
      return JSON.stringify(merged) === JSON.stringify(previous.layouts) ? previous : { ...previous, layouts: merged };
    });
  }, [editing]);

  const closeSave = saveMenu.close;
  const persistLayout = (asNew: boolean) => {
    const name = layoutName.trim();
    if (!name || name.length > 160) { setSaveError('Enter a layout name (up to 160 characters).'); return; }
    if (library.layouts.some((entry) => (asNew || entry.id !== library.activeId) && entry.template.name.toLowerCase() === name.toLowerCase())) {
      setSaveError('A layout already has that name. Choose a different name.'); return;
    }
    if (asNew && library.layouts.length >= MAX_LAYOUTS) { setSaveError(`You can save up to ${MAX_LAYOUTS} layouts.`); return; }
    const parsed = parseTemplateWithNotes({ ...includeCampaigns(draftTemplate, campaigns), name });
    if (!parsed) { setSaveError('This layout could not be saved. Your changes are still available.'); return; }
    const { template: clean, droppedTags } = parsed;
    const id = asNew ? newId() : library.activeId;
    const entry = { id, template: clean };
    const next = { ...library, activeId: id, layouts: asNew ? [...library.layouts, entry] : library.layouts.map((item) => item.id === id ? entry : item) };
    const error = saveLayouts(next);
    setSaveError(error);
    if (error) return;
    setLibrary(next); setTemplate(clean); setSavedTemplate(clean); saveMenu.popup.current?.hidePopover(); setEditing(false); setWarning(droppedTagWarning(droppedTags));
    editButton.current?.focus();
  };

  const selectLayout = (id: string) => {
    if (editing || dirty || campaignsRef.current.some(campaign => campaign.loading)) return;
    const entry = library.layouts.find((item) => item.id === id);
    if (!entry) return;
    if (!parseTemplate(includeCampaigns(entry.template, campaigns))) { setWarning('This layout has no room for the open campaigns. Use a layout with fewer source slots or tracks.'); return; }
    const next = { ...library, activeId: id };
    setWarning(saveLayouts(next));
    setLibrary(next); setTemplate(entry.template); setSavedTemplate(null);
  };

  /** Opens a recording into a campaign slot; resolves to the campaign's id once it is loaded. */
  const openCampaign = useCallback(async (chosen: string, label = recordingName(chosen), slotId?: string, reconnect = false): Promise<string | null> => {
    const known = campaignsRef.current.find((item) => sameCampaignPath(item.path, chosen));
    if (known && !reconnect) return known.loading ? null : known.id;
    if (known?.loading) return null;
    const slots = campaignSlots(draftTemplate, campaignsRef.current);
    const slot = slotId === 'new' ? undefined : slotId ? slots.empty.find(role => role.id === slotId) : slots.empty[0];
    if (slotId && slotId !== 'new' && !slot) { setActionError('That campaign slot is now occupied. Choose another slot.'); return null; }
    if (!known && !slot && !slots.canAdd) { setActionError('Unload a campaign before loading another.'); return null; }
    if (reconnect) clock.stop();
    const id = known?.id ?? slot?.id ?? newId();
    const campaign = { id, path: chosen, label: label.slice(0, 160), loading: true };
    const next = [...campaignsRef.current.filter((item) => item.id !== id), campaign];
    campaignsRef.current = next;
    setCampaigns(next);
    setLoadErrors(current => { const next = { ...current }; delete next[chosen]; return next; });
    const previous = roles.bindings[id]?.source;
    if (reconnect && known && desktopAvailable && previous) {
      // Retire the standalone owner's snapshot before replacing this lease.
      // A rejected retirement leaves the old binding available for retry/unload.
      try { await mapAction({ kind: 'unload', path: previous.path }); }
      catch (cause) {
        if (mounted.current && campaignsRef.current.includes(campaign)) {
          campaignsRef.current = campaignsRef.current.map(item => item === campaign ? known : item);
          setCampaigns(campaignsRef.current);
          setLoadErrors(current => ({ ...current, [chosen]: errorMessage(cause) }));
        }
        return null;
      }
      if (!mounted.current || !campaignsRef.current.includes(campaign)) return null;
    }
    const source = await roles.open(id, chosen, message => setLoadErrors(current => ({ ...current, [chosen]: message })));
    if (!mounted.current || !campaignsRef.current.includes(campaign)) return null;
    campaignsRef.current = source ? campaignsRef.current.map(item => item === campaign ? { ...item, loading: false } : item)
      : campaignsRef.current.filter(item => item !== campaign);
    setCampaigns(campaignsRef.current);
    if (!source) {
      roles.remove(id);
      setDefaultRoleId(current => defaultAfterUnload(current, id, campaignsRef.current.filter(campaign => !campaign.loading).map(campaign => campaign.id)));
    }
    if (source) {
      hasOpened.current = true;
      if (!reconnect) {
        setSelectedCountries((current) => ({ ...current, [id]: null }));
        setSelectedBranches(current => Object.fromEntries(Object.entries(current).filter(([trackId]) => !template.tracks.some(track => track.id === trackId && track.roleId === id))));
      }
      setDefaultRoleId((current) => defaultAfterOpen(current, id));
    }
    return source ? id : null;
  }, [roles.open, roles.remove, roles.bindings, draftTemplate, template.tracks, clock]);

  /** Points a widget at the default campaign (null) or a recording. A recording that is not
   * loaded yet is opened once and shared, exactly as from the sidebar. */
  const assignCampaign = useCallback(async (widgetId: string, chosen: string | null) => {
    const assign = (previous: DashboardTemplate, widget: WidgetSpec, roleId: string | null): WidgetSpec => {
      const track = roleId === null ? null : previous.tracks.find(item => item.roleId === roleId);
      if (roleId !== null && !track) return widget;
      // Choosing the campaign it already follows changes nothing, not even a pinned moment.
      if (roleId === null ? widget.campaign === 'default' : widget.campaign === 'tracks' && widget.trackIds[0] === track!.id) return widget;
      // A pinned moment belongs to one recording's branch; a new campaign starts from the widget's default time.
      const time = widget.time.mode === 'pin' ? { mode: widgetDefinition(widget.kind)?.defaultTime ?? 'campaign' } as const : widget.time;
      return roleId === null ? { ...widget, campaign: 'default' as const, time } : { ...widget, campaign: 'tracks' as const, trackIds: [track!.id], time };
    };
    // The campaign is a settings-panel choice too, so it saves at once like the rest. The
    // new widget is worked out from the current layout first, so the save needs no
    // state updater to have run.
    const pick = (roleId: string | null) => {
      const current = includeCampaigns(templateRef.current, campaignsRef.current);
      const widget = current.widgets.find(item => item.id === widgetId);
      const changed = widget && assign(current, widget, roleId);
      if (!changed || changed === widget) return;
      update(previous => ({ ...previous, widgets: previous.widgets.map(item => item.id === widgetId ? changed : item) }));
      persistView(widget, changed);
    };
    if (chosen === null) { pick(null); return; }
    const roleId = campaignsRef.current.find(item => item.id === chosen && !item.loading)?.id ?? await openCampaign(chosen);
    if (!mounted.current) return;
    if (roleId) { pick(roleId); return; }
    // openCampaign reports its own failures beside the sidebar; the picker says why nothing changed.
    throw new Error(campaignsRef.current.some(item => item.loading && sameCampaignPath(item.path, chosen))
      ? 'That recording is still opening. Choose it again once it has loaded.'
      : 'That recording could not be opened. The notice above the dashboard or the sidebar shows why.');
  }, [update, openCampaign, persistView]);

  const unloadCampaign = async (id: string) => {
    const campaign = campaignsRef.current.find(campaign => campaign.id === id);
    if (!campaign || campaign.loading) return;
    if (id === defaultRoleId) clock.stop();
    const retiring = { ...campaign, loading: true };
    campaignsRef.current = campaignsRef.current.map(item => item === campaign ? retiring : item);
    setCampaigns(campaignsRef.current);
    const source = roles.bindings[id]?.source;
    if (desktopAvailable && source) {
      try { await mapAction({ kind: 'unload', path: source.path }); }
      catch (cause) {
        if (mounted.current && campaignsRef.current.includes(retiring)) {
          campaignsRef.current = campaignsRef.current.map(item => item === retiring ? campaign : item);
          setCampaigns(campaignsRef.current);
          setActionError(errorMessage(cause));
        }
        return;
      }
    }
    if (!mounted.current || !campaignsRef.current.includes(retiring)) return;
    setDefaultRoleId(current => defaultAfterUnload(current, id, campaigns.filter(campaign => roles.bindings[campaign.id]?.source).map(campaign => campaign.id)));
    roles.remove(id);
    campaignsRef.current = campaignsRef.current.filter(campaign => campaign.id !== id);
    setCampaigns(campaignsRef.current);
  };
  const adoptedPath = useRef<string | null>(null);
  useEffect(() => {
    if (!path || !client.available || adoptedPath.current === path) return;
    adoptedPath.current = path;
    if (!campaignsRef.current.some((item) => sameCampaignPath(item.path, path))) void openCampaign(path);
  }, [path, client.available, openCampaign]);
  const defaultSource = defaultRoleId ? sourceMap.get(defaultRoleId) : null;
  const countryTags = useMemo(() => Object.fromEntries(campaigns.map(campaign => {
    const metadata = roles.bindings[campaign.id]?.source?.metadata;
    return [campaign.id, selectedCountries[campaign.id] ?? metadata?.countries.find(country => country.id === metadata.playerCountryId)?.tag ?? null];
  })), [campaigns, roles.bindings, selectedCountries]);
  useEffect(() => onMetadata(defaultSource?.metadata ?? null), [defaultSource, onMetadata]);
  useEffect(() => {
    const firstReady = campaigns.find((campaign) => roles.bindings[campaign.id]?.source);
    if (firstReady) setDefaultRoleId((current) => defaultAfterOpen(current, firstReady.id));
  }, [campaigns, roles.bindings]);
  useEffect(() => { if (hasOpened.current && (defaultSource?.path ?? null) !== path) onPrimaryPath?.(defaultSource?.path ?? null); }, [defaultSource, path, onPrimaryPath, campaigns.length]);

  const onMapBranch = useCallback((trackId: string, branchId: string) => {
    const source = tracks.find(track => track.id === trackId)?.source;
    const index = source?.metadata.branches.findIndex(branch => branch.id === branchId) ?? -1;
    if (index >= 0) { clock.stop(); setSelectedBranches(previous => ({ ...previous, [trackId]: index })); }
  }, [tracks, clock]);

  const recordings = replayLibrary.files;
  const requestSeek = useCallback((trackId: string, rawHour: number) => {
    const track = tracks.find(item => item.id === trackId);
    const position = track ? seekPositionFor(track, rawHour) : null;
    if (position == null) return false;
    clock.seek(position);
    return true;
  }, [tracks, clock]);
  const addWidgetBelow = useCallback((sourceId: string, widget: WidgetSpec) => {
    if (templateRef.current.widgets.length >= MAX_WIDGETS) { setWarning(`A layout holds up to ${MAX_WIDGETS} widgets. Remove one to pin another.`); return; }
    update(previous => ({ ...previous, widgets: [...previous.widgets, widget], layouts: addGridItemBelow(previous.layouts, widget, sourceId) }));
  }, [update]);
  const runtime = useMemo(() => ({ onMapBranch, tracks, clock, maps, client, campaignTracks: template.tracks, defaultRoleId, alignment: template.alignment, nativeHidden,
    campaigns, recordings, assignCampaign, requestSeek, addWidgetBelow }),
    [onMapBranch, tracks, clock, maps, client, template.tracks, template.alignment, defaultRoleId, nativeHidden, campaigns, recordings, assignCampaign, requestSeek, addWidgetBelow]);


  return <div className="dashboard">
    {campaignHost && createPortal(<ReplayLibraryMenu library={replayLibrary} client={client} campaigns={campaigns} bindings={roles.bindings} defaultId={defaultRoleId} countryTags={countryTags}
      slots={campaignSlots(draftTemplate, campaigns)} errors={loadErrors} onLoad={(path, label, slotId) => { void openCampaign(path, label, slotId); }}
      onUnload={unloadCampaign} onDefault={setDefaultRoleId} onOpenReady={onOpenReady} />, campaignHost)}
    {(active || mapActive) && playbackHost && createPortal(<div className="header-workspace-bar">
    <PlaybackCountryPicker clock={clock} track={tracks.find(track => track.roleId === defaultRoleId)} source={defaultSource} client={client} onCountries={roles.observeCountries} onReconnect={() => {
      const campaign = campaignsRef.current.find(campaign => campaign.id === defaultRoleId);
      if (campaign) void openCampaign(campaign.path, campaign.label, undefined, true);
    }} countries={defaultSource?.metadata.countries ?? []} playerCountryId={defaultSource?.metadata.playerCountryId ?? null}
      selectedTag={template.roles.find((role) => role.id === defaultRoleId)?.selectedTag ?? null}
      onChange={(tag) => { if (defaultRoleId) setSelectedCountries((previous) => ({ ...previous, [defaultRoleId]: tag })); }} />
    <PlaybackStrip clock={clock} tracks={tracks} alignment={template.alignment} defaultRoleId={defaultRoleId} />
    <div className="header-workspace-tools">
    {mapActive && <MapControls source={defaultSource ?? null} client={client} metadata={defaultSource?.metadata ?? null} branchId={tracks.find(track => track.roleId === defaultRoleId)?.branch.id ?? null}
      onBranch={id => { const track = tracks.find(track => track.roleId === defaultRoleId); if (track) onMapBranch(track.id, id); }}
      options={mapOptions} onOptions={setMapOptions} />}
    {active && <>
      {/* The layout picker and the editing tools share one slot, so the bar never shifts or grows. */}
      <div className="toolbar-swap" data-editing={editing || dirty || undefined}>
      <label className="header-layout"><Icon name="layers" /><span className="sr-only">Layout</span>
        <select aria-label="Saved layout" value={library.activeId} disabled={editing || dirty || campaigns.some(campaign => campaign.loading)}
          onChange={(event) => selectLayout(event.currentTarget.value)}>
          {library.layouts.map((entry) => <option key={entry.id} value={entry.id}>{entry.template.name}</option>)}
        </select></label>
      <div className="dash-toolbar" role="toolbar" aria-label="Layout changes">
        <span className="toolbar-slot" data-shown={editing || undefined}><WidgetPicker runtime={runtime} disabled={!editing || template.widgets.length >= MAX_WIDGETS} onOpen={() => saveMenu.popup.current?.hidePopover()} onAdd={(kind) => {
          const widget = createWidget(kind, template.tracks[0].id);
          update((previous) => ({ ...previous, widgets: [...previous.widgets, widget], layouts: addGridItem(previous.layouts, widget) }));
        }} /></span>
        <button type="button" className="icon-button header-reset toolbar-slot" data-shown={editing || undefined} aria-label="Reset layout" data-tooltip="Reset arrangement to default" onClick={() => {
          update(() => ({ ...defaultTemplate(), name: template.name })); saveMenu.popup.current?.hidePopover();
        }}><Icon name="refresh" /></button>
        <button type="button" className="icon-button" aria-label="Cancel layout changes" data-tooltip="Discard layout changes" onClick={() => {
          setTemplate(selectedLayout.template); setEditing(false); saveMenu.popup.current?.hidePopover(); editButton.current?.focus();
        }}><Icon name="close" /></button>
        <button ref={saveButton} type="button" className={`icon-button toolbar-save${dirty ? ' is-dirty' : ''}`} aria-label="Save layout" data-tooltip="Save layout" aria-expanded={saving} aria-haspopup="dialog" popoverTarget="layout-save-menu" onClick={() => {
          setLayoutName(template.name); setSaveError(null);
        }}><Icon name="save" /></button>
      </div>
      </div>
      <button ref={editButton} type="button" className={`button toolbar-edit${editing ? ' primary' : ''}`} aria-label={editing ? 'Done arranging' : 'Edit layout'} data-tooltip={editing ? 'Done arranging' : 'Arrange widgets'} aria-pressed={editing} onClick={() => {
          saveMenu.popup.current?.hidePopover();
          setEditing(!editing);
        }}><Icon name={editing ? 'check' : 'edit'} /><span className="toolbar-label">{editing ? 'Done' : 'Edit'}</span></button>
      <Popover menu={saveMenu} id="layout-save-menu" className="layout-save-menu" aria-label="Save layout options">
      {saving && <form onSubmit={event => { event.preventDefault(); persistLayout(false); }}>
        <div className="ui-popover-heading"><strong>Save layout</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close save options" onClick={closeSave}><Icon name="close" /></button></div>
        <label className="field"><span>Layout name</span><input data-popover-autofocus aria-label="Layout name" value={layoutName} maxLength={160} onChange={(event) => { setLayoutName(event.currentTarget.value); setSaveError(null); }} /></label>
        <p className="field-hint">Update this layout, or keep it and save your changes as a new one.</p>
        {saveError && <p className="layout-save-error" role="alert">{saveError}</p>}
        <div className="layout-save-actions"><button type="submit" className="button" disabled={!layoutName.trim()}>Update layout</button>
          <button type="button" className="button primary" disabled={!layoutName.trim() || library.layouts.length >= MAX_LAYOUTS} onClick={() => persistLayout(true)}>Save as new</button></div>
      </form>}
      </Popover>
    </>}
    </div>
    </div>, playbackHost)}
    {(warning || actionError) && <div className="dash-banner" role="alert"><Icon name="alert" /><span>{actionError ?? warning}</span>
      <button type="button" className="icon-button small" aria-label="Dismiss notice" onClick={() => { setActionError(null); setWarning(null); }}><Icon name="close" /></button></div>}
    {saved && savedVisible && <div className="layout-saved" role="status"><Icon name="check" />{saved}</div>}
    <div className={`dash-grid-scroll${editing ? ' is-editing' : ''}`} ref={viewport}>
      {active && <DashboardGrid template={template} runtime={runtime} editing={editing} onLayouts={layoutsChanged} onApply={applyWidget} onRemove={removeWidget} />}
      {template.widgets.length === 0 && <div className="dash-empty dash-empty-dashboard"><span className="empty-icon"><Icon name="grid" /></span><strong>Make room for your campaign</strong><p>Choose Edit, then add a campaign map, a national focus or research panel, or an empty panel to plan around.</p></div>}
    </div>
  </div>;
}
