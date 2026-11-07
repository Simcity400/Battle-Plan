import { asFailure, type Failure } from './queryError.ts';
import { Popover, usePopover } from '../components/Popover';
import { Select } from '../components/Select';
import { newId } from '../shell/ids.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { desktopAvailable, errorMessage, recordingName, seekMap, mapAction } from '../shell/api';
import { configurePopout, popoutClockKey, popoutConfigKey, popoutProblem, popoutStopped, popoutUnloaded, seekPopout, setPopoutFlyAircraft, usePopout } from '../shell/popout';
import type { MapOptions, ReplayMetadata } from '../shell/api';
import MapControls from '../components/MapControls';
import CameraModeControl from '../components/CameraModeControl';
import AircraftPicker from '../components/AircraftPicker';
import type { CameraMode, FlyAircraftStatus } from '../generated/dashboard';
import { loadAircraftChoice, saveAircraftChoice } from './aircraftChoice';
import { Icon } from '../components/Icons';
import type { DashboardClient, DashboardSource } from './client';
import { addGridItem, addGridItemBelow, bindTracks, clockBounds, createWidget, defaultTemplate, MAX_WIDGETS, mergeViewPreferences, rebaseWidget, seekPositionFor, stableJson } from './model';
import type { DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { keptPageStates } from './pages';
import type { PageEntry, PageId } from './pages';
import { droppedTagWarning, layoutsUnsaved, loadLayouts, MAX_LAYOUTS, parseLayoutLibrary, parseTemplate, parseTemplateWithNotes, saveLayouts } from './storage';
import type { LayoutLibrary } from './storage';
import { PlaybackController } from './playback';
import { widgetDefinition } from './definitions';
import '../widgets';
import { CAMPAIGN_MAP_VIEW, campaignToggled, MapViewsStore, toggleMapSetting } from './geometry';
import { useSourceRoles } from './sources';
import { PlaybackCountryPicker, PlaybackStrip } from './controls';
import ReplayLibraryMenu from './ReplayLibraryMenu';
import type { ReplayLibraryState } from './replayLibrary';
import DashboardGrid from './DashboardGrid';
import './dashboard.css';
import WidgetPicker from './WidgetPicker';
import { campaignSlots, defaultAfterOpen, defaultAfterUnload, includeCampaigns, recordingKey, sameCampaignPath } from './campaigns';
import { hasMapOverlay, isMapOverlay } from '../shell/mapOverlays';
import type { Campaign, Relocation } from './campaigns';
import { recordingFilename } from './recordingNames';
import { followMapPlayback, mapCommands } from './mapPlayback';
import type { ParityHost } from './parityFixture';
import { LiveMotion } from '../charts/liveMotion.ts';
import { commitZone } from '../shell/commitCount';

interface Props { page: PageId; resetToken: number; library: ReplayLibraryState; path: string | null; client: DashboardClient; onPrimaryPath?: (path: string | null) => void; active?: boolean; mapActive?: boolean; mapSwitching?: boolean; mapReady?: boolean; cameraMode?: CameraMode;
  /** Whether flying and walking draw the map's interface, as the viewer last reported it. */
  cameraOverlays?: boolean; onMetadata: (metadata: ReplayMetadata | null) => void;
  /** The branch the shown campaign plays, and how to play another; null without one. */
  onBranch?: (branch: BranchChoice | null) => void;
  /** A campaign-map operation failed, or (null) succeeded or is no longer wanted. */
  onMapError?: (operation: string, error: string | null) => void;
  /** Bumped by Retry map: the campaign map is sent the clock's hour again, and each failed command. */
  mapRetry?: number;
  /** Whether any page has layout changes not yet saved, which resetting the pages discards. */
  onUnsaved?: (unsaved: boolean) => void;
  /** The pages that exist; what a deleted page kept while not shown is dropped. */
  pages?: readonly PageEntry[]; playbackHost?: HTMLElement | null; campaignHost?: HTMLElement | null; onOpenReady?: (open: (() => void) | null) => void;
  /** What the full map last answered for the aircraft Fly mode flies. */
  flyAircraft?: FlyAircraftStatus | null;
  /** Where panels dock beside the workspace, narrowing the map rather than covering it. */
  sideHost?: HTMLElement | null }

export interface BranchChoice { id: string; choose: (branchId: string) => void }

export default function Dashboard({ page, resetToken, path, client, library: replayLibrary, onPrimaryPath, active = true, mapActive = false, mapSwitching = false, mapReady = false, cameraMode = 'map', cameraOverlays = false, onMetadata, onBranch, onMapError, mapRetry = 0, onUnsaved, pages, playbackHost, campaignHost, onOpenReady, flyAircraft = null, sideHost = null }: Props) {
  const [restored] = useState(() => loadLayouts(undefined, page));
  const [library, setLibrary] = useState(restored.library);
  // The page whose layouts are shown; it trails `page` until the swap below runs.
  const [shownPage, setShownPage] = useState(page);
  const pageRef = useRef(shownPage);
  pageRef.current = shownPage;
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
  // Histories chosen by id: indexes shift as a recording grows or its joins change.
  const [selectedBranches, setSelectedBranches] = useState<Record<string, string>>({});
  const [loadErrors, setLoadErrors] = useState<Record<string, Failure>>({});
  const hasOpened = useRef(false);
  const [mapOptions, setMapOptions] = useState<MapOptions>({ country: null, fog: true });
  // The campaign map's picker choices, sent back with its options so a restarted viewer keeps them.
  const [campaignPicked, setCampaignPicked] = useState<Pick<MapOptions, 'mapMode' | 'mapModeSlots'>>({});
  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => { if (live) stops.push(stop); else stop(); };
    void client.onMapOptions?.((event) => {
      if (event.viewId === CAMPAIGN_MAP_VIEW) setCampaignPicked({ mapMode: event.options.mapMode ?? null, mapModeSlots: event.options.mapModeSlots ?? [] });
    }).then(keep);
    return () => { live = false; for (const stop of stops) stop(); };
  }, [client]);
  const template = useMemo(() => {
    const merged = includeCampaigns(draftTemplate, campaigns);
    return { ...merged, roles: merged.roles.map((role) => ({ ...role, selectedTag: selectedCountries[role.id] ?? null })) };
  }, [draftTemplate, campaigns, selectedCountries]);
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
  // Each page keeps its layouts, draft and edit state while another page is shown. A new
  // reset token drops them all, so every page reopens from storage.
  const pageStates = useRef(new Map<PageId, { library: LayoutLibrary; template: DashboardTemplate; editing: boolean }>());
  const resetSeen = useRef(resetToken);
  useLayoutEffect(() => {
    const reset = resetToken !== resetSeen.current;
    if (page === shownPage && !reset) return;
    resetSeen.current = resetToken;
    if (reset) pageStates.current.clear();
    else pageStates.current.set(shownPage, { library: libraryRef.current, template: templateRef.current, editing });
    let next = pageStates.current.get(page);
    if (!next) {
      const loaded = loadLayouts(undefined, page);
      next = { library: loaded.library, template: loaded.library.layouts.find(entry => entry.id === loaded.library.activeId)!.template, editing: false };
      setWarning(loaded.warning);
    }
    saveMenu.popup.current?.hidePopover();
    viewport.current?.scrollTo({ top: 0 });
    setLibrary(next.library); setTemplate(next.template); setEditing(next.editing); setSavedTemplate(null); setShownPage(page);
  }, [page, shownPage, editing, saveMenu.popup, resetToken]);
  const saving = saveMenu.open;
  const [layoutName, setLayoutName] = useState('');
  const [saveError, setSaveError] = useState<string | null>(null);
  const saveButton = saveMenu.trigger;
  const editButton = useRef<HTMLButtonElement>(null);
  const dirty = useMemo(() => stableJson(includeCampaigns(draftTemplate, campaigns)) !== stableJson(includeCampaigns(selectedLayout.template, campaigns)), [draftTemplate, selectedLayout.template, campaigns]);
  // A deleted page's kept layouts and draft go with it.
  const pageList = pages?.map(entry => entry.id).join('\n');
  useEffect(() => {
    if (pageList === undefined) return;
    const existing = pageList.split('\n');
    for (const id of [...pageStates.current.keys()]) if (!existing.includes(id)) pageStates.current.delete(id);
  }, [pageList]);
  // Unsaved changes on the shown page or any page kept aside, for Settings' page reset.
  const unsaved = useMemo(() => layoutsUnsaved([{ library, template: draftTemplate },
    ...keptPageStates(pageStates.current, shownPage, pageList?.split('\n') ?? [...pageStates.current.keys()])], template => includeCampaigns(template, campaigns)),
  [library, draftTemplate, shownPage, campaigns, pageList]);
  useEffect(() => { onUnsaved?.(unsaved); }, [unsaved, onUnsaved]);
  useEffect(() => () => onUnsaved?.(false), [onUnsaved]);
  // Resetting the arrangement would change nothing when the draft already is the page's default.
  const layoutAtDefault = useMemo(() => stableJson(includeCampaigns(draftTemplate, campaigns))
    === stableJson(includeCampaigns({ ...defaultTemplate(shownPage), name: draftTemplate.name }, campaigns)), [draftTemplate, campaigns, shownPage]);
  const [actionError, setActionError] = useState<string | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const mounted = useRef(false);
  const roles = useSourceRoles(client);
  const clock = useMemo(() => new PlaybackController(), []);
  const mapErrors = useRef(onMapError);
  mapErrors.current = onMapError;
  const commands = useMemo(() => mapCommands((operation, error) => mapErrors.current?.(operation, error)), []);
  const maps = useMemo(() => new MapViewsStore(client, clock), [client, clock]);
  useEffect(() => {
    // The native picker's day/night and allied-plan toggles flip the widget's own settings.
    maps.onToggle = (view, toggle) => { void toggleMapSetting(client, view, toggle).catch(() => undefined); };
    return () => { maps.onToggle = null; };
  }, [maps, client]);
  const sourceMap = useMemo(() => new Map(Object.entries(roles.bindings).flatMap(([roleId, binding]) => binding.source ? [[roleId, binding.source] as [string, DashboardSource]] : [])), [roles.bindings]);
  const tracks = useMemo(() => bindTracks(template, sourceMap, selectedBranches), [template.tracks, template.roles, template.alignment, sourceMap, selectedBranches]);
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

  const popout = usePopout();
  const popoutOpen = useRef(false);
  popoutOpen.current = popout.open;
  // The pop-out window shown now, which a request captures when it starts.
  const popoutGeneration = useRef(0);
  popoutGeneration.current = popout.generation;
  // Commands the pop-out's viewer `generation:viewer` refused, shown in its controls rather than over the main map.
  const [popoutFailures, setPopoutFailures] = useState<{ viewer: string; command: 'configure' | 'seek' | 'aircraft'; message: string }[]>([]);
  const reportPopout = useCallback((viewer: string, command: 'configure' | 'seek' | 'aircraft', message: string | null) => setPopoutFailures(previous => {
    const kept = previous.filter(failure => failure.viewer === viewer && failure.command !== command);
    return message == null ? (kept.length === previous.length ? previous : kept) : [...kept, { viewer, command, message }];
  }), []);
  const popoutViewer = popout.open ? `${popout.generation}:${popout.viewer}` : null;
  const popoutIssue = popoutProblem(popout) ?? popoutFailures.find(failure => failure.viewer === popoutViewer)?.message ?? null;
  // A pop-out map keeps following the clock from any page.
  useLayoutEffect(() => { if (!active && !mapActive && !popout.open) clock.stop(); }, [active, mapActive, popout.open, clock]);
  const mapCountry = defaultRoleId ? selectedCountries[defaultRoleId] ?? null : null;
  useEffect(() => {
    if (!mapActive || !mapReady || mapSwitching || defaultLoading || !desktopAvailable) return;
    const options = { ...mapOptions, ...campaignPicked, country: mapCountry };
    commands.send('configure', () => mapAction({ kind: 'configure', options }));
    return () => commands.forget('configure');
  }, [mapActive, mapReady, mapSwitching, defaultLoading, mapOptions, campaignPicked, mapCountry, commands]);
  // The camera mode and its overlay choice are the viewer's own: the control shows what it reports
  // and asks it to change.
  const chooseCameraMode = useCallback((mode: CameraMode) => {
    commands.send('camera', () => mapAction({ kind: 'cameraMode', mode, overlays: cameraOverlays }));
  }, [cameraOverlays, commands]);
  const chooseOverlays = useCallback((overlays: boolean) => {
    commands.send('camera', () => mapAction({ kind: 'cameraMode', mode: cameraMode, overlays }));
  }, [cameraMode, commands]);
  // A camera change is for the viewer shown when it was asked; a hidden or restarted map drops its failure.
  useEffect(() => mapActive && mapReady && desktopAvailable ? () => commands.forget('camera') : undefined, [mapActive, mapReady, commands]);
  const resendSeek = useRef<(() => void) | null>(null);
  useLayoutEffect(() => {
    const reference = tracks.find(track => track.roleId === defaultRoleId);
    if (!mapActive || mapSwitching || defaultLoading || !desktopAvailable || !reference) return;
    const stop = followMapPlayback(clock, reference, seekMap, error => {
      if (error != null) clock.stop();
      onMapError?.('seek', error == null ? null : errorMessage(error));
    });
    resendSeek.current = stop.resend;
    return () => { stop(); resendSeek.current = null; onMapError?.('seek', null); };
  }, [clock, tracks, defaultRoleId, defaultLoading, mapActive, mapSwitching, onMapError]);
  // Retry map sends the clock's hour again even without a failed seek: a viewer's own load error needs it too.
  const retried = useRef(mapRetry);
  useEffect(() => {
    if (mapRetry === retried.current) return;
    retried.current = mapRetry;
    resendSeek.current?.();
    commands.retry();
  }, [mapRetry, commands]);
  // The viewer runs its own clock between anchors, and a restarted one is restored with the last anchor
  // it was sent: a viewer that becomes ready is sent the clock as it is now.
  useEffect(() => {
    if (!mapReady) return;
    resendSeek.current?.();
    maps.resend();
  }, [mapReady, maps]);
  // Keyed on the window's generation, so every new pop-out window is sent the options and the
  // clock even when it opens in the same render that closed the previous one.
  const popoutConfig = popoutConfigKey(popout);
  useEffect(() => {
    if (popoutConfig == null || defaultLoading) return;
    let current = true;
    void configurePopout(popout.generation, { ...mapOptions, country: mapCountry })
      .then(() => { if (current) reportPopout(popoutConfig, 'configure', null); }, cause => { if (current) reportPopout(popoutConfig, 'configure', errorMessage(cause)); });
    return () => { current = false; };
  }, [popoutConfig, popout.generation, defaultLoading, mapOptions, mapCountry, reportPopout]);
  const popoutClock = popoutClockKey(popout);
  useLayoutEffect(() => {
    const reference = tracks.find(track => track.roleId === defaultRoleId);
    if (popoutClock == null || defaultLoading || !reference) return;
    const generation = popout.generation;
    const seek: Parameters<typeof followMapPlayback>[2] = (path, revision, branch, hour, playback) => seekPopout(generation, path, revision, branch, hour, playback);
    return followMapPlayback(clock, reference, seek, error => reportPopout(popoutClock, 'seek', error == null ? null : errorMessage(error)));
  }, [clock, tracks, defaultRoleId, defaultLoading, popoutClock, popout.generation, reportPopout]);
  useLayoutEffect(() => active && viewport.current ? maps.start(viewport.current) : undefined, [maps, active]);
  useLayoutEffect(() => { maps.setHidden(nativeHidden || hasMapOverlay()); maps.schedule(); }, [maps, nativeHidden, template.layouts]);
  useEffect(() => {
    mounted.current = true;
    // A minimised app keeps playing while the pop-out map shows the campaign.
    const visibility = () => { if (document.hidden && !popoutOpen.current) clock.setPlaying(false); };
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
    const error = saveLayouts(next, undefined, pageRef.current);
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
    const error = saveLayouts(next, undefined, shownPage);
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
    setWarning(saveLayouts(next, undefined, shownPage));
    setLibrary(next); setTemplate(entry.template); setSavedTemplate(null);
  };

  /** Opens a recording into a campaign slot; resolves to the campaign's id once it is loaded. */
  const openCampaign = useCallback(async (chosen: string, label = recordingName(chosen), slotId?: string, reconnect = false, campaignId?: string, onFailure?: (error: Failure) => void): Promise<string | null> => {
    // A rename reconnects its own campaign by id: another may briefly share the path.
    const known = campaignId === undefined
      ? campaignsRef.current.find((item) => sameCampaignPath(item.path, chosen))
      : campaignsRef.current.find((item) => item.id === campaignId);
    if (campaignId !== undefined && !known) return null;
    if (known && !reconnect) return known.loading ? null : known.id;
    if (known?.loading) return null;
    const slots = campaignSlots(draftTemplate, campaignsRef.current);
    const slot = slotId === 'new' ? undefined : slotId ? slots.empty.find(role => role.id === slotId) : slots.empty[0];
    if (slotId && slotId !== 'new' && !slot) { setActionError('That campaign slot is now occupied. Choose another slot.'); return null; }
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
          setLoadErrors(current => ({ ...current, [chosen]: asFailure(cause) }));
        }
        return null;
      }
      if (!mounted.current || !campaignsRef.current.includes(campaign)) return null;
    }
    const source = await roles.open(id, chosen, error => { setLoadErrors(current => ({ ...current, [chosen]: error })); onFailure?.(error); });
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
    let failure: Failure | undefined;
    const roleId = campaignsRef.current.find(item => item.id === chosen && !item.loading)?.id
      ?? await openCampaign(chosen, undefined, undefined, false, undefined, error => { failure = error; });
    if (!mounted.current) return;
    if (roleId) { pick(roleId); return; }
    if (failure) throw failure;
    // openCampaign reports its own failures beside the sidebar; the picker says why nothing changed.
    throw new Error(campaignsRef.current.some(item => item.loading && sameCampaignPath(item.path, chosen))
      ? 'That recording is still opening. Choose it again once it has loaded.'
      : 'That recording could not be opened. The notice above the dashboard or the sidebar shows why.');
  }, [update, openCampaign, persistView]);

  const bindingsRef = useRef(roles.bindings);
  bindingsRef.current = roles.bindings;
  const [relocations, setRelocations] = useState<Relocation[]>([]);
  /** A recording's file was renamed, by either app or in Explorer: an open campaign reconnects by its new name. */
  const relocate = useCallback((from: string, to: string) => {
    setLoadErrors(current => Object.fromEntries(Object.entries(current).map(([path, error]) => [sameCampaignPath(path, from) ? to : path, error])));
    const campaign = campaignsRef.current.find(item => sameCampaignPath(item.path, from));
    // One still opening keeps the path it opens; a rename waits for it.
    if (!campaign || campaign.loading) return;
    campaignsRef.current = campaignsRef.current.map(item => item === campaign ? { ...item, path: to } : item);
    setCampaigns(campaignsRef.current);
    // The open campaign names the recording through all its renames, whatever its header or folder.
    setRelocations(current => [...current, [from, to, campaign.id]]);
    void openCampaign(to, campaign.label, undefined, true, campaign.id);
  }, [openCampaign]);
  const shownRecordings = useCallback(() => campaignsRef.current.flatMap(campaign => {
    const source = campaign.loading ? null : bindingsRef.current[campaign.id]?.source;
    return source ? [{ path: campaign.path, campaignKey: source.metadata.campaignKey }] : [];
  }), []);
  // A look the host has not answered yet is never stacked with another, whichever effect started it.
  const locating = useRef(false);
  // Each open recording is found again by its folder and campaign, so a rename shows at once.
  useEffect(() => {
    if (!client.locateRecordings) return;
    let live = true;
    const timer = setInterval(() => {
      const shown = shownRecordings();
      if (!shown.length || locating.current) return;
      locating.current = true;
      void client.locateRecordings!(shown).finally(() => { locating.current = false; }).then(located => {
        if (!live) return;
        let moved = false;
        // A new name, or the same one with different capitals.
        located.forEach((path, at) => {
          if (path && (!sameCampaignPath(path, shown[at].path) || recordingFilename(path) !== recordingFilename(shown[at].path))) { relocate(shown[at].path, path); moved = true; }
        });
        if (moved) void replayLibrary.refresh();
      }, () => {});
    }, 2000);
    return () => { live = false; clearInterval(timer); };
  }, [client, relocate, shownRecordings, replayLibrary]);

  const unloadCampaign = async (id: string) => {
    const campaign = campaignsRef.current.find(campaign => campaign.id === id);
    if (!campaign || campaign.loading) return;
    if (id === defaultRoleId) clock.stop();
    const retiring = { ...campaign, loading: true };
    campaignsRef.current = campaignsRef.current.map(item => item === campaign ? retiring : item);
    setCampaigns(campaignsRef.current);
    const source = roles.bindings[id]?.source;
    if (desktopAvailable && source) {
      // The pop-out window shown when the unload started; a later window is not this unload's.
      const shownPopout = popoutGeneration.current;
      try { await mapAction({ kind: 'unload', path: source.path }); await popoutUnloaded(source.path, shownPopout); }
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
    // Other campaigns may unload while this unload is in flight; use the current roster.
    setDefaultRoleId(current => defaultAfterUnload(current, id, campaignsRef.current.filter(campaign => !campaign.loading).map(campaign => campaign.id)));
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
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    // Each click is its own flip, which may have saved before failing: Retry map clears it rather than flips again.
    const flips: string[] = [];
    void client.onMapToggle?.((event) => {
      if (!live) return;
      const flip = `toggle ${flips.length}`;
      flips.push(flip);
      commands.send(flip, () => campaignToggled(event, setMapOptions, client, defaultSource ?? null), false);
    }).then(unsubscribe => { if (live) stop = unsubscribe; else unsubscribe(); });
    return () => { live = false; stop?.(); commands.forget(...flips); };
  }, [client, defaultSource, commands]);
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
  // Fly mode's aircraft belongs to the campaign the map shows: a name of its content, kept on
  // this device, with none as the start. Sent again whenever a map becomes ready, as a restarted
  // viewer flies none.
  const aircraftRecording = defaultSource ? recordingKey(defaultSource.path, defaultSource.metadata.campaignKey) : null;
  const [aircraft, setAircraft] = useState<string | null>(null);
  useEffect(() => setAircraft(aircraftRecording ? loadAircraftChoice(aircraftRecording) : null), [aircraftRecording]);
  const chooseAircraft = useCallback((name: string | null) => {
    setAircraft(name);
    if (aircraftRecording) saveAircraftChoice(aircraftRecording, name);
  }, [aircraftRecording]);
  useEffect(() => {
    if (!mapActive || !mapReady || !desktopAvailable) return;
    commands.send('aircraft', () => mapAction({ kind: 'flyAircraft', aircraft }));
    return () => commands.forget('aircraft');
  }, [mapActive, mapReady, aircraft, commands]);
  // The pop-out's map flies the same aircraft; each viewer it starts is told when it is ready.
  useEffect(() => {
    if (popoutConfig == null) return;
    let current = true;
    void setPopoutFlyAircraft(popout.generation, aircraft)
      .then(() => { if (current) reportPopout(popoutConfig, 'aircraft', null); }, cause => { if (current) reportPopout(popoutConfig, 'aircraft', errorMessage(cause)); });
    return () => { current = false; };
  }, [popoutConfig, popout.generation, aircraft, reportPopout]);

  const onMapBranch = useCallback((trackId: string, branchId: string) => {
    const source = tracks.find(track => track.id === trackId)?.source;
    if (source?.metadata.branches.some(branch => branch.id === branchId)) { clock.stop(); setSelectedBranches(previous => ({ ...previous, [trackId]: branchId })); }
  }, [tracks, clock]);

  const shownTrack = tracks.find(track => track.roleId === defaultRoleId);
  useEffect(() => onBranch?.(shownTrack ? { id: shownTrack.branch.id, choose: branchId => onMapBranch(shownTrack.id, branchId) } : null),
    // The track object changes with every rebind; its identity and branch are what the choice names.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [shownTrack?.id, shownTrack?.branch.id, onMapBranch, onBranch]);

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
  if (import.meta.env.MODE === 'parity') {
    // The build mode is fixed for this component's lifetime and removed from a
    // production bundle together with its hooks and fixture import.
    const parityHost = useRef<ParityHost | null>(null);
    parityHost.current = { client, template, runtime, setTemplate, openCampaign };
    useEffect(() => {
      let current = true, cleanup: (() => void) | undefined;
      const ready = import('./parityFixture').then(module => { if (current) cleanup = module.installParityFixture(() => parityHost.current!); });
      window.__hoi4ParityReady = ready;
      return () => { current = false; cleanup?.(); if (window.__hoi4ParityReady === ready) delete window.__hoi4ParityReady; };
    }, [client]);
  }

  /** Opens a campaign's recording again, found by its folder and campaign first. */
  const reconnect = async (campaign: Campaign) => {
    const source = bindingsRef.current[campaign.id]?.source;
    const [located] = source && client.locateRecordings
      ? await client.locateRecordings([{ path: campaign.path, campaignKey: source.metadata.campaignKey }]).catch(() => [null]) : [null];
    // Renamed, the relocation reconnects it; either way only this campaign, by its id.
    if (located && (!sameCampaignPath(located, campaign.path) || recordingFilename(located) !== recordingFilename(campaign.path))) relocate(campaign.path, located);
    else void openCampaign(campaign.path, campaign.label, undefined, true, campaign.id);
  };

  return <div className="dashboard">
    {campaignHost && createPortal(<ReplayLibraryMenu library={replayLibrary} client={client} campaigns={campaigns} bindings={roles.bindings} defaultId={defaultRoleId} countryTags={countryTags}
      slots={campaignSlots(draftTemplate, campaigns)} errors={loadErrors} onLoad={(path, label, slotId) => { void openCampaign(path, label, slotId); }}
      onUnload={unloadCampaign} onDefault={setDefaultRoleId} onRenamed={relocate} relocations={relocations} onOpenReady={onOpenReady} />, campaignHost)}
    {(active || mapActive) && playbackHost && createPortal(<div className="header-workspace-bar">
    <PlaybackCountryPicker clock={clock} track={tracks.find(track => track.roleId === defaultRoleId)} source={defaultSource} client={client} onCountries={roles.observeCountries} onReconnect={() => {
      const campaign = campaignsRef.current.find(campaign => campaign.id === defaultRoleId);
      if (campaign) void reconnect(campaign);
    }} countries={defaultSource?.metadata.countries ?? []} playerCountryId={defaultSource?.metadata.playerCountryId ?? null}
      selectedTag={template.roles.find((role) => role.id === defaultRoleId)?.selectedTag ?? null}
      onChange={(tag) => { if (defaultRoleId) setSelectedCountries((previous) => ({ ...previous, [defaultRoleId]: tag })); }} />
    <PlaybackStrip client={client} clock={clock} tracks={tracks} alignment={template.alignment} defaultRoleId={defaultRoleId} />
    <div className="header-workspace-tools">
    {mapActive && mapReady && desktopAvailable && <CameraModeControl value={cameraMode} onChange={chooseCameraMode} overlays={cameraOverlays} onOverlaysChange={chooseOverlays} />}
    {mapActive && mapReady && desktopAvailable && cameraMode === 'fly' && <AircraftPicker client={client} source={defaultSource ?? null} chosen={aircraft} status={flyAircraft} onChoose={chooseAircraft} host={sideHost} prompt />}
    {mapActive && <MapControls source={defaultSource ?? null} client={client} metadata={defaultSource?.metadata ?? null} branchId={tracks.find(track => track.roleId === defaultRoleId)?.branch.id ?? null}
      onBranch={id => { const track = tracks.find(track => track.roleId === defaultRoleId); if (track) onMapBranch(track.id, id); }}
      onChooseHistory={client.chooseHistory && defaultSource ? async (segmentId, parentId) => {
        const campaign = campaignsRef.current.find(campaign => campaign.id === defaultRoleId);
        if (!campaign) return;
        await client.chooseHistory!(defaultSource.path, segmentId, parentId, new AbortController().signal);
        await openCampaign(campaign.path, campaign.label, undefined, true, campaign.id);
      } : undefined}
      options={mapOptions} onOptions={setMapOptions} poppedOut={popout.open} popoutGeneration={popout.generation} popoutProblem={popoutIssue} popoutStopped={popoutStopped(popout)}
      popoutCameraMode={popout.status?.running ? popout.status.cameraMode : null}
      popoutCameraOverlays={popout.status?.running ? popout.status.cameraOverlays : false}
      aircraft={aircraft} onAircraft={chooseAircraft} popoutFlyAircraft={popout.status?.flyAircraft ?? null} aircraftHost={sideHost} />}
    {active && <>
      {/* The layout picker and the editing tools share one slot, so the bar never shifts or grows. */}
      <div className="toolbar-swap" data-editing={editing || dirty || undefined}>
      <div className="header-layout"><Icon name="layers" />
        <Select label="Saved layout" value={library.activeId} disabled={editing || dirty || campaigns.some(campaign => campaign.loading)}
          onChange={selectLayout} options={library.layouts.map(entry => ({ value: entry.id, label: entry.template.name }))} /></div>
      <div className="dash-toolbar" role="toolbar" aria-label="Layout changes">
        <span className="toolbar-slot" data-shown={editing || undefined}><WidgetPicker runtime={runtime} placed={template.widgets} disabled={!editing || template.widgets.length >= MAX_WIDGETS} onOpen={() => saveMenu.popup.current?.hidePopover()} onAdd={(kind, initial) => {
          const widget = { ...createWidget(kind, template.tracks[0].id), ...initial };
          update((previous) => ({ ...previous, widgets: [...previous.widgets, widget], layouts: addGridItem(previous.layouts, widget) }));
        }} /></span>
        {!layoutAtDefault && <button type="button" className="icon-button header-reset toolbar-slot" data-shown={editing || undefined} aria-label="Reset layout" data-tooltip="Reset arrangement to default" onClick={() => {
          update(() => ({ ...defaultTemplate(shownPage), name: template.name })); saveMenu.popup.current?.hidePopover();
        }}><Icon name="refresh" /></button>}
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
      {active && <LiveMotion.Provider value={clock.getMotion}><DashboardGrid template={template} runtime={runtime} editing={editing} onLayouts={layoutsChanged} onApply={applyWidget} onRemove={removeWidget} /></LiveMotion.Provider>}
      {template.widgets.length === 0 && <div className="dash-empty dash-empty-dashboard"><span className="empty-icon"><Icon name="grid" /></span><strong>Make room for your campaign</strong><p>Choose Edit, then add widgets, or reset this page to its default arrangement.</p></div>}
    </div>
  </div>;
}

commitZone(Dashboard, 'dashboard');
