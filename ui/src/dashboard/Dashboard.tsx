import { actionFailure, asFailure, type Failure, type PlainFailure } from './queryError.ts';
import { Popover, usePopover } from '../components/Popover';
import { ConfirmStep } from '../components/ContextMenu';
import { newId } from '../shell/ids.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal, flushSync } from 'react-dom';
import { desktopAvailable, errorMessage, recordingName, seekMap, mapAction } from '../shell/api';
import { configurePopout, popoutClockKey, popoutConfigKey, popoutProblem, popoutStopped, popoutUnloaded, seekPopout, setPopoutFlyAircraft, usePopout } from '../shell/popout';
import type { MapOptions, ReplayMetadata } from '../shell/api';
import MapControls from '../components/MapControls';
import CameraModeControl from '../components/CameraModeControl';
import AircraftPicker from '../components/AircraftPicker';
import type { CameraMode, FlyAircraftStatus } from '../generated/dashboard';
import { loadAircraftChoice, saveAircraftChoice } from './aircraftChoice';
import type { AircraftChoice } from './aircraftChoice';
import { Icon } from '../components/Icons';
import type { DashboardClient, DashboardSource } from './client';
import { addGridItem, addGridItemBelow, bindTracks, clockBounds, createWidget, defaultTemplate, mergeViewPreferences, rebaseWidget, seekPositionFor, stableJson } from './model';
import type { DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { persistedSpec, type PinnedSpec } from './regionTime.ts';
import type { PageEntry, PageId } from './pages';
import { doneEditing, droppedTagWarning, layoutSaveAllowed, layoutSaveCheck, loadLayouts, unsavedLayoutPages, parseLayoutLibrary, parseTemplate, parseTemplateWithNotes, saveLayouts } from './storage';
import type { LayoutLibrary, LayoutSaveAction } from './storage';
import { PlaybackController } from './playback';
import { widgetDefinition } from './definitions';
import '../widgets';
import { preloadAllWidgetRenderers, preloadWidgetRenderers } from '../widgets/rendererLoaders';
import { laneModules } from '../widgets/manifest';
import { usePipeline } from './follow/useFollowed.ts';
import { CAMPAIGN_MAP_VIEW, campaignToggled, MapViewsStore, toggleMapSetting } from './geometry';
import { DEFAULT_MAP_OPTIONS, observedRole, observedSwitches } from './mapPicker';
import type { CountryChoice } from './mapPicker';
import { useSourceRoles } from './sources';
import { LayoutMenu, PlaybackCountryPicker, PlaybackStrip } from './controls';
import EmptyState from '../components/EmptyState';
import { DismissButton, FailureLine } from '../components/FailureLine';
import { useSlotTags } from './countrySlots.ts';
import ReplayLibraryMenu from './ReplayLibraryMenu';
import type { ReplayLibraryState } from './replayLibrary';
import DashboardGrid, { bodiesWaiting } from './DashboardGrid';
import PreparedPages from './PreparedPages';
import type { PreparedPage } from './PreparedPages';
import { preparingClient } from './prepare.ts';
import './dashboard.css';
import WidgetPicker from './WidgetPicker';
import { campaignSlots, defaultAfterOpen, defaultAfterUnload, includeCampaigns, recordingKey, sameCampaignPath } from './campaigns';
import type { Campaign, Relocation } from './campaigns';
import { recordingDisplayName, recordingFilename } from './recordingNames';
import { followMapPlayback, mapCommands } from './mapPlayback';
import { LiveMotion } from '../charts/liveMotion.ts';
import { commitZone } from '../shell/commitCount';
import { keepLatest } from '../shell/latest.ts';
import type { Latest } from '../shell/latest.ts';
import { unsavedDrafts } from './layoutDrafts.ts';

interface Props { page: PageId; resetToken: number; library: ReplayLibraryState; path: string | null; client: DashboardClient; onPrimaryPath?: (path: string | null) => void; active?: boolean; mapActive?: boolean; mapSwitching?: boolean; mapReady?: boolean; cameraMode?: CameraMode;
  cameraOverlays?: boolean; onMetadata: (metadata: ReplayMetadata | null) => void;
  onBranch?: (branch: BranchChoice | null) => void;
  onMapError?: (operation: string, error: string | null) => void;
  onMapWarning?: (warning: string) => void;
  mapRetry?: number;
  onUnsaved?: (pages: readonly PageId[]) => void;
  unsavedReader?: Latest<(() => readonly PageId[]) | null>;
  saveRequest?: number;
  pages?: readonly PageEntry[]; playbackHost?: HTMLElement | null; campaignHost?: HTMLElement | null; onOpenReady?: (open: (() => void) | null) => void;
  flyAircraft?: FlyAircraftStatus | null;
  sideHost?: HTMLElement | null }

export interface BranchChoice { id: string; choose: (branchId: string) => void }

export default function Dashboard({ page, resetToken, path, client, library: replayLibrary, onPrimaryPath, active = true, mapActive = false, mapSwitching = false, mapReady = false, cameraMode = 'map', cameraOverlays = false, onMetadata, onBranch, onMapError, onMapWarning, mapRetry = 0, onUnsaved, unsavedReader, saveRequest = 0, pages, playbackHost, campaignHost, onOpenReady, flyAircraft = null, sideHost = null }: Props) {
  const [restored] = useState(() => loadLayouts(undefined, page));
  useEffect(() => {
    const shown = restored.library.layouts.find(entry => entry.id === restored.library.activeId);
    preloadWidgetRenderers(shown?.template.widgets.map(widget => widget.kind) ?? []);
    preloadAllWidgetRenderers();
  }, [restored]);
  const [library, showLibrary] = useState(restored.library);
  const libraryRef = useRef(library);
  const setLibrary = useMemo(() => keepLatest(libraryRef, showLibrary), []);
  const [shownPage, showPage] = useState(page);
  const pageRef = useRef(shownPage);
  const setShownPage = useMemo(() => keepLatest(pageRef, showPage), []);
  const selectedLayout = library.layouts.find((entry) => entry.id === library.activeId)!;
  const [draftTemplate, showTemplate] = useState<DashboardTemplate>(selectedLayout.template);
  const templateRef = useRef(draftTemplate);
  const setTemplate = useMemo(() => keepLatest(templateRef, showTemplate), []);
  const [campaigns, setCampaigns] = useState<Campaign[]>([]);
  const campaignsRef = useRef(campaigns);
  campaignsRef.current = campaigns;
  const pageIds = useMemo(() => pages?.map(entry => entry.id), [pages]);
  const pagesRef = useRef(pageIds);
  pagesRef.current = pageIds;
  const [defaultRoleId, setDefaultRoleId] = useState<string | null>(null);
  const [selectedCountries, setSelectedCountries] = useState<Record<string, CountryChoice | null>>({});
  const [observed, setObserved] = useState<Record<string, number>>({});
  const [selectedBranches, setSelectedBranches] = useState<Record<string, string>>({});
  const [loadErrors, setLoadErrors] = useState<Record<string, Failure>>({});
  const hasOpened = useRef(false);
  const [mapOptions, setMapOptions] = useState<MapOptions>(DEFAULT_MAP_OPTIONS);
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
  const merged = useMemo(() => includeCampaigns(draftTemplate, campaigns), [draftTemplate, campaigns]);
  const roleList = merged.roles.map((role) => ({ ...role, selectedTag: selectedCountries[role.id]?.tag ?? null, selectedSlot: selectedCountries[role.id]?.slot ?? null, observed }));
  const rolesKey = JSON.stringify(roleList), tracksKey = JSON.stringify(merged.tracks);
  const templateRoles = useMemo(() => roleList, [rolesKey]);
  const templateTracks = useMemo(() => merged.tracks, [tracksKey]);
  const template = useMemo(() => ({ ...merged, roles: templateRoles, tracks: templateTracks }), [merged, templateRoles, templateTracks]);
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
  const discardMenu = usePopover();
  const resetMenu = usePopover();
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
  const [saveError, setSaveError] = useState<{ text: string; action: LayoutSaveAction } | null>(null);
  const saveButton = saveMenu.trigger;
  const editButton = useRef<HTMLButtonElement>(null);
  const dirty = useMemo(() => doneEditing({ library, template: draftTemplate }, template => includeCampaigns(template, campaigns)) === 'save', [library, draftTemplate, campaigns]);
  const pageList = pages?.map(entry => entry.id).join('\n');
  useEffect(() => {
    if (pageList === undefined) return;
    const existing = pageList.split('\n');
    for (const id of [...pageStates.current.keys()]) if (!existing.includes(id)) pageStates.current.delete(id);
  }, [pageList]);
  const unsavedKey = useMemo(() => {
    const existing = pageList?.split('\n') ?? [...pageStates.current.keys()];
    const kept = [...pageStates.current].filter(([id]) => id !== shownPage && existing.includes(id));
    return unsavedLayoutPages([[shownPage, { library, template: draftTemplate }], ...kept], template => includeCampaigns(template, campaigns)).join('\n');
  }, [library, draftTemplate, shownPage, campaigns, pageList]);
  useEffect(() => { onUnsaved?.(unsavedKey ? unsavedKey.split('\n') as PageId[] : []); }, [unsavedKey, onUnsaved]);
  useEffect(() => () => onUnsaved?.([]), [onUnsaved]);
  useLayoutEffect(() => {
    if (!unsavedReader) return;
    const drafts = { page: pageRef, library: libraryRef, template: templateRef, kept: pageStates, pages: pagesRef, campaigns: campaignsRef };
    unsavedReader.current = () => unsavedDrafts(drafts);
    return () => { unsavedReader.current = null; };
  }, [unsavedReader]);
  const layoutAtDefault = useMemo(() => stableJson(includeCampaigns(draftTemplate, campaigns))
    === stableJson(includeCampaigns({ ...defaultTemplate(shownPage), name: draftTemplate.name }, campaigns)), [draftTemplate, campaigns, shownPage]);
  const [actionError, setActionError] = useState<PlainFailure | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  const mounted = useRef(false);
  const roles = useSourceRoles(client);
  const clock = useMemo(() => new PlaybackController(), []);
  const mapErrors = useRef(onMapError);
  const mapWarnings = useRef(onMapWarning);
  mapWarnings.current = onMapWarning;
  mapErrors.current = onMapError;
  const commands = useMemo(() => mapCommands((operation, error) => mapErrors.current?.(operation, error),
    warning => mapWarnings.current?.(`The map change was made, but Windows didn’t confirm it was saved: ${warning}`)), []);
  const maps = useMemo(() => new MapViewsStore(client, clock), [client, clock]);
  useEffect(() => {
    maps.onToggle = (view, toggle) => view.id === CAMPAIGN_MAP_VIEW
      ? commands.send(`setting:${view.id}:${toggle.setting}`, () => toggleMapSetting(client, view, toggle), false)
      : maps.saveToggle(view, toggle, () => toggleMapSetting(client, view, toggle));
    return () => { maps.onToggle = null; };
  }, [maps, client, commands]);
  const sourceMap = useMemo(() => new Map(Object.entries(roles.bindings).flatMap(([roleId, binding]) => binding.source ? [[roleId, binding.source] as [string, DashboardSource]] : [])), [roles.bindings]);
  const tracks = useMemo(() => bindTracks(template, sourceMap, selectedBranches), [template.tracks, template.roles, template.alignment, sourceMap, selectedBranches]);
  const bounds = useMemo(() => clockBounds(tracks), [tracks]);
  const primaryIdentity = defaultRoleId && sourceMap.get(defaultRoleId) ? `${sourceMap.get(defaultRoleId)!.id}:${sourceMap.get(defaultRoleId)!.revision}` : '';
  const defaultLoading = campaigns.some(campaign => campaign.id === defaultRoleId && campaign.loading);
  const lastClockIdentity = useRef('');
  const nativeHidden = !active || editing || saving;

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
  const popoutGeneration = useRef(0);
  popoutGeneration.current = popout.generation;
  const [popoutFailures, setPopoutFailures] = useState<{ viewer: string; command: 'configure' | 'seek' | 'aircraft'; message: string }[]>([]);
  const reportPopout = useCallback((viewer: string, command: 'configure' | 'seek' | 'aircraft', message: string | null) => setPopoutFailures(previous => {
    const kept = previous.filter(failure => failure.viewer === viewer && failure.command !== command);
    return message == null ? (kept.length === previous.length ? previous : kept) : [...kept, { viewer, command, message }];
  }), []);
  const popoutViewer = popout.open ? `${popout.generation}:${popout.viewer}` : null;
  const popoutIssue = popoutProblem(popout) ?? popoutFailures.find(failure => failure.viewer === popoutViewer)?.message ?? null;
  const [popoutRetry, setPopoutRetry] = useState(0);
  const retryPopout = popoutProblem(popout) == null && popoutIssue != null ? () => setPopoutRetry(count => count + 1) : undefined;
  useLayoutEffect(() => { if (!active && !mapActive && !popout.open) clock.stop(); }, [active, mapActive, popout.open, clock]);
  const mapChoice = defaultRoleId ? selectedCountries[defaultRoleId] ?? null : null;
  const mapCountry = mapChoice?.tag ?? null, mapCountrySlot = mapChoice?.slot ?? null, mapObserved = observed[CAMPAIGN_MAP_VIEW] ?? null;
  useEffect(() => {
    if (!mapActive || !mapReady || mapSwitching || defaultLoading || !desktopAvailable) return;
    const options = { ...mapOptions, ...campaignPicked, country: mapCountry, countrySlot: mapCountrySlot, observed: mapObserved };
    commands.send('configure', () => mapAction({ kind: 'configure', options }));
    return () => commands.forget('configure');
  }, [mapActive, mapReady, mapSwitching, defaultLoading, mapOptions, campaignPicked, mapCountry, mapCountrySlot, mapObserved, commands]);
  const chooseCameraMode = useCallback((mode: CameraMode) => {
    commands.send('camera', () => mapAction({ kind: 'cameraMode', mode, overlays: cameraOverlays }));
  }, [cameraOverlays, commands]);
  const chooseOverlays = useCallback((overlays: boolean) => {
    commands.send('camera', () => mapAction({ kind: 'cameraMode', mode: cameraMode, overlays }));
  }, [cameraMode, commands]);
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
  const retried = useRef(mapRetry);
  useEffect(() => {
    if (mapRetry === retried.current) return;
    retried.current = mapRetry;
    resendSeek.current?.();
    commands.retry();
  }, [mapRetry, commands]);
  useEffect(() => {
    if (!mapReady) return;
    resendSeek.current?.();
    maps.resend();
  }, [mapReady, maps]);
  const popoutConfig = popoutConfigKey(popout);
  useEffect(() => {
    if (popoutConfig == null || defaultLoading) return;
    let current = true;
    void configurePopout(popout.generation, { ...mapOptions, country: mapCountry, countrySlot: mapCountrySlot, observed: mapObserved })
      .then(() => { if (current) reportPopout(popoutConfig, 'configure', null); }, cause => { if (current) reportPopout(popoutConfig, 'configure', errorMessage(cause)); });
    return () => { current = false; };
  }, [popoutConfig, popout.generation, defaultLoading, mapOptions, mapCountry, mapCountrySlot, mapObserved, reportPopout, popoutRetry]);
  const popoutClock = popoutClockKey(popout);
  useLayoutEffect(() => {
    const reference = tracks.find(track => track.roleId === defaultRoleId);
    if (popoutClock == null || defaultLoading || !reference) return;
    const generation = popout.generation;
    const seek: Parameters<typeof followMapPlayback>[2] = (path, revision, branch, hour, playback) => seekPopout(generation, path, revision, branch, hour, playback);
    return followMapPlayback(clock, reference, seek, error => reportPopout(popoutClock, 'seek', error == null ? null : errorMessage(error)));
  }, [clock, tracks, defaultRoleId, defaultLoading, popoutClock, popout.generation, reportPopout, popoutRetry]);
  useLayoutEffect(() => active && viewport.current ? maps.start(viewport.current) : undefined, [maps, active]);
  useLayoutEffect(() => { maps.setHidden(nativeHidden); maps.schedule(); }, [maps, nativeHidden, template.layouts]);
  useEffect(() => {
    mounted.current = true;
    const visibility = () => { if (document.hidden && !popoutOpen.current) clock.setPlaying(false); };
    document.addEventListener('visibilitychange', visibility);
    return () => {
      mounted.current = false;
      document.removeEventListener('visibilitychange', visibility);
      clock.stop();
    };
  }, [clock]);

  const update = useCallback((change: (previous: DashboardTemplate) => DashboardTemplate) => {
    setTemplate((previous) => change(includeCampaigns(previous, campaignsRef.current)));
  }, []);
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
    setLibrary(next);
  }, [setLibrary]);
  const applyWidget = useCallback((widget: WidgetSpec, view = false) => {
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
  const saveCheck = layoutSaveCheck(library, layoutName);
  const persistLayout = (action: LayoutSaveAction | null) => {
    const check = layoutSaveCheck(library, layoutName);
    if (!action || !layoutSaveAllowed(check, action)) return;
    const asNew = action === 'new';
    const name = layoutName.trim();
    const parsed = parseTemplateWithNotes({ ...includeCampaigns(draftTemplate, campaigns), name });
    if (!parsed) { setSaveError({ text: 'Couldn’t save this layout. Your changes are still here.', action }); return; }
    const { template: clean, droppedTags } = parsed;
    const id = asNew ? newId() : library.activeId;
    const entry = { id, template: clean };
    const next = { ...library, activeId: id, layouts: asNew ? [...library.layouts, entry] : library.layouts.map((item) => item.id === id ? entry : item) };
    const error = saveLayouts(next, undefined, shownPage);
    setSaveError(error ? { text: error, action } : null);
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

  const openCampaign = useCallback(async (chosen: string, label = recordingName(chosen), slotId?: string, reconnect = false, campaignId?: string, onFailure?: (error: Failure) => void): Promise<string | null> => {
    const known = campaignId === undefined
      ? campaignsRef.current.find((item) => sameCampaignPath(item.path, chosen))
      : campaignsRef.current.find((item) => item.id === campaignId);
    if (campaignId !== undefined && !known) return null;
    if (known && !reconnect) return known.loading ? null : known.id;
    if (known?.loading) return null;
    const slots = campaignSlots(templateRef.current, campaignsRef.current);
    const slot = slotId === 'new' ? undefined : slotId ? slots.empty.find(role => role.id === slotId) : slots.empty[0];
    if (slotId && slotId !== 'new' && !slot) { setActionError({ text: 'That campaign slot is now occupied. Choose another slot.' }); return null; }
    if (reconnect) clock.stop();
    const id = known?.id ?? slot?.id ?? newId();
    const campaign = { id, path: chosen, label: label.slice(0, 160), loading: true };
    const next = [...campaignsRef.current.filter((item) => item.id !== id), campaign];
    campaignsRef.current = next;
    setCampaigns(next);
    setLoadErrors(current => { const next = { ...current }; delete next[chosen]; return next; });
    const previous = roles.bindings[id]?.source;
    if (reconnect && known && desktopAvailable && previous) {
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
  }, [roles.open, roles.remove, roles.bindings, template.tracks, clock]);

  const assignCampaign = useCallback(async (widgetId: string, chosen: string | null) => {
    const assign = (previous: DashboardTemplate, widget: WidgetSpec, roleId: string | null): WidgetSpec => {
      const track = roleId === null ? null : previous.tracks.find(item => item.roleId === roleId);
      if (roleId !== null && !track) return widget;
      if (roleId === null ? widget.campaign === 'default' : widget.campaign === 'tracks' && widget.trackIds[0] === track!.id) return widget;
      const time = widget.time.mode === 'pin' ? { mode: widgetDefinition(widget.kind)?.defaultTime ?? 'campaign' } as const : widget.time;
      return roleId === null ? { ...widget, campaign: 'default' as const, time } : { ...widget, campaign: 'tracks' as const, trackIds: [track!.id], time };
    };
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
    throw new Error(campaignsRef.current.some(item => item.loading && sameCampaignPath(item.path, chosen))
      ? 'That recording is still opening. Choose it again once it has loaded.'
      : 'That recording couldn’t be opened. The notice above the dashboard or the sidebar shows why.');
  }, [update, openCampaign, persistView]);

  const bindingsRef = useRef(roles.bindings);
  bindingsRef.current = roles.bindings;
  const [relocations, setRelocations] = useState<Relocation[]>([]);
  const relocate = useCallback((from: string, to: string) => {
    setLoadErrors(current => Object.fromEntries(Object.entries(current).map(([path, error]) => [sameCampaignPath(path, from) ? to : path, error])));
    const campaign = campaignsRef.current.find(item => sameCampaignPath(item.path, from));
    if (!campaign || campaign.loading) return;
    campaignsRef.current = campaignsRef.current.map(item => item === campaign ? { ...item, path: to } : item);
    setCampaigns(campaignsRef.current);
    setRelocations(current => [...current, [from, to, campaign.id]]);
    void openCampaign(to, campaign.label, undefined, true, campaign.id);
  }, [openCampaign]);
  const shownRecordings = useCallback(() => campaignsRef.current.flatMap(campaign => {
    const source = campaign.loading ? null : bindingsRef.current[campaign.id]?.source;
    return source ? [{ path: campaign.path, campaignKey: source.metadata.campaignKey }] : [];
  }), []);
  const locating = useRef(false);
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
      const shownPopout = popoutGeneration.current;
      try { await mapAction({ kind: 'unload', path: source.path }); await popoutUnloaded(source.path, shownPopout); }
      catch (cause) {
        if (mounted.current && campaignsRef.current.includes(retiring)) {
          campaignsRef.current = campaignsRef.current.map(item => item === retiring ? campaign : item);
          setCampaigns(campaignsRef.current);
          setActionError(actionFailure(`Couldn’t unload ${recordingDisplayName(campaign.path)}.`, errorMessage(cause)));
        }
        return;
      }
    }
    if (!mounted.current || !campaignsRef.current.includes(retiring)) return;
    setActionError(null);
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
    const flips: string[] = [];
    void client.onMapToggle?.((event) => {
      if (!live) return;
      const flip = `toggle ${flips.length}`;
      flips.push(flip);
      commands.send(flip, () => campaignToggled(event, setMapOptions, client, defaultSource ?? null), false);
    }).then(unsubscribe => { if (live) stop = unsubscribe; else unsubscribe(); });
    return () => { live = false; stop?.(); commands.forget(...flips); };
  }, [client, defaultSource, commands]);
  const observeTarget = useRef({ template, defaultRoleId });
  observeTarget.current = { template, defaultRoleId };
  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    void client.onMapObserve?.((event) => {
      const role = live ? observedRole(event.viewId, observeTarget.current.template, observeTarget.current.defaultRoleId) : null;
      if (role) setSelectedCountries(previous => ({ ...previous, [role]: { tag: event.country.tag, slot: event.country.slot } }));
      if (live) setObserved(previous => observedSwitches(previous, event));
    }).then(unsubscribe => { if (live) stop = unsubscribe; else unsubscribe(); });
    return () => { live = false; stop?.(); };
  }, [client]);
  const defaultTrack = tracks.find(track => track.roleId === defaultRoleId);
  const defaultSlots = useSlotTags(client, defaultTrack, !!defaultTrack);
  const pickCountry = (tag: string | null, slot: number | null) => {
    if (!defaultRoleId) return;
    const hour = defaultTrack ? clock.getHour() + defaultTrack.axisOffset : null;
    const held = tag == null || hour == null ? null : defaultSlots?.slotOf(tag, hour) ?? null;
    const chosen = held ?? slot;
    if (tag != null && chosen == null) return;
    setSelectedCountries((previous) => ({ ...previous, [defaultRoleId]: tag == null || chosen == null ? null : { tag, slot: chosen } }));
  };
  const countryTags = useMemo(() => Object.fromEntries(campaigns.map(campaign => {
    const metadata = roles.bindings[campaign.id]?.source?.metadata;
    return [campaign.id, selectedCountries[campaign.id]?.tag ?? metadata?.countries.find(country => country.id === metadata.playerCountryId)?.tag ?? null];
  })), [campaigns, roles.bindings, selectedCountries]);
  useEffect(() => onMetadata(defaultSource?.metadata ?? null), [defaultSource, onMetadata]);
  useEffect(() => {
    const firstReady = campaigns.find((campaign) => roles.bindings[campaign.id]?.source);
    if (firstReady) setDefaultRoleId((current) => defaultAfterOpen(current, firstReady.id));
  }, [campaigns, roles.bindings]);
  useEffect(() => { if (hasOpened.current && (defaultSource?.path ?? null) !== path) onPrimaryPath?.(defaultSource?.path ?? null); }, [defaultSource, path, onPrimaryPath, campaigns.length]);
  const aircraftRecording = defaultSource ? recordingKey(defaultSource.path, defaultSource.metadata.campaignKey) : null;
  const [aircraft, setAircraft] = useState<AircraftChoice | null>(null);
  const aircraftName = aircraft?.name ?? null;
  useEffect(() => setAircraft(aircraftRecording ? loadAircraftChoice(aircraftRecording) : null), [aircraftRecording]);
  const chooseAircraft = useCallback((choice: AircraftChoice | null) => {
    setAircraft(choice);
    if (aircraftRecording) saveAircraftChoice(aircraftRecording, choice);
  }, [aircraftRecording]);
  useEffect(() => {
    if (!mapActive || !mapReady || !desktopAvailable) return;
    commands.send('aircraft', () => mapAction({ kind: 'flyAircraft', aircraft: aircraftName }));
    return () => commands.forget('aircraft');
  }, [mapActive, mapReady, aircraftName, commands]);
  useEffect(() => {
    if (popoutConfig == null) return;
    let current = true;
    void setPopoutFlyAircraft(popout.generation, aircraftName)
      .then(() => { if (current) reportPopout(popoutConfig, 'aircraft', null); }, cause => { if (current) reportPopout(popoutConfig, 'aircraft', errorMessage(cause)); });
    return () => { current = false; };
  }, [popoutConfig, popout.generation, aircraftName, reportPopout, popoutRetry]);

  const onMapBranch = useCallback((trackId: string, branchId: string) => {
    const source = tracks.find(track => track.id === trackId)?.source;
    if (source?.metadata.branches.some(branch => branch.id === branchId)) { clock.stop(); setSelectedBranches(previous => ({ ...previous, [trackId]: branchId })); }
  }, [tracks, clock]);

  const shownTrack = tracks.find(track => track.roleId === defaultRoleId);
  useEffect(() => onBranch?.(shownTrack ? { id: shownTrack.branch.id, choose: branchId => onMapBranch(shownTrack.id, branchId) } : null),
    [shownTrack?.id, shownTrack?.branch.id, onMapBranch, onBranch]);

  const recordings = replayLibrary.files;
  const requestSeek = useCallback((trackId: string, rawHour: number) => {
    const track = tracks.find(item => item.id === trackId);
    const position = track ? seekPositionFor(track, rawHour) : null;
    if (position == null) return false;
    clock.seek(position);
    return true;
  }, [tracks, clock]);
  const addWidgetBelow = useCallback((sourceId: string, pinned: PinnedSpec) => {
    const widget = persistedSpec(pinned);
    update(previous => ({ ...previous, widgets: [...previous.widgets, widget], layouts: addGridItemBelow(previous.layouts, widget, sourceId) }));
  }, [update]);
  const runtime = useMemo(() => ({ onMapBranch, tracks, clock, maps, client, campaignTracks: template.tracks, defaultRoleId, alignment: template.alignment, nativeHidden,
    campaigns, recordings, assignCampaign, requestSeek, addWidgetBelow }),
    [onMapBranch, tracks, clock, maps, client, template.tracks, template.alignment, defaultRoleId, nativeHidden, campaigns, recordings, assignCampaign, requestSeek, addWidgetBelow]);

  const laneClient = useMemo(() => preparingClient(client), [client]);
  const [viewportBox, setViewportBox] = useState<HTMLDivElement | null>(null);
  useLayoutEffect(() => setViewportBox(viewport.current), []);
  const pageOrder = useMemo(() => {
    const ids = pageList?.split('\n') ?? [shownPage];
    const at = Math.max(0, ids.indexOf(shownPage));
    return [...ids.slice(at), ...ids.slice(0, at)].filter((id, index) => index === 0 || id !== shownPage) as PageId[];
  }, [pageList, shownPage]);
  const preparedPages = useMemo<PreparedPage[]>(() => pageOrder.map(id => {
    const kept = id === shownPage ? draftTemplate : pageStates.current.get(id)?.template ?? (() => {
      const { library: loaded } = loadLayouts(undefined, id);
      return loaded.layouts.find(entry => entry.id === loaded.activeId)!.template;
    })();
    const merged = includeCampaigns(kept, campaigns);
    const paged = { ...merged, roles: merged.roles.map(role => ({ ...role, selectedTag: selectedCountries[role.id]?.tag ?? null, selectedSlot: selectedCountries[role.id]?.slot ?? null, observed })) };
    const bound = bindTracks(paged, sourceMap, selectedBranches);
    return { id, template: paged, runtime: { ...runtime, client: laneClient, tracks: bound, campaignTracks: paged.tracks, alignment: paged.alignment } };
  }), [pageOrder, shownPage, draftTemplate, campaigns, selectedCountries, observed, sourceMap, selectedBranches, runtime, laneClient]);

  const laneKinds = [...new Set(preparedPages.flatMap(entry => entry.template.widgets.map(widget => widget.kind)))].sort().join(' ');
  const laneNeeds = useMemo(() => laneModules(laneKinds.split(' ')), [laneKinds]);
  const pipeline = usePipeline(laneNeeds.families.length ? clock : null, client);
  useEffect(() => { pipeline?.prepare(laneNeeds.families, laneNeeds.projections); }, [pipeline, laneNeeds]);

  const [discarding, setDiscarding] = useState(false);
  const openSave = () => {
    setLayoutName(selectedLayout.template.name); setSaveError(null); setDiscarding(false);
    const popup = saveMenu.popup.current;
    if (!popup || popup.matches(':popover-open')) return;
    saveMenu.opener.current = document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null;
    popup.showPopover();
  };
  const keepChanges = () => {
    flushSync(() => setDiscarding(false));
    saveMenu.popup.current?.querySelector<HTMLElement>('.layout-save-discard')?.focus();
  };
  const askedSave = useRef(saveRequest);
  useEffect(() => {
    if (saveRequest === askedSave.current || !active || page !== shownPage) return;
    askedSave.current = saveRequest;
    if (dirty) openSave();
  });
  const discardLayout = () => {
    setTemplate(selectedLayout.template); setEditing(false); saveMenu.popup.current?.hidePopover(); editButton.current?.focus();
  };
  const resetLayout = () => {
    update(() => ({ ...defaultTemplate(shownPage), name: template.name })); saveMenu.popup.current?.hidePopover(); editButton.current?.focus();
  };
  const reconnect = async (campaign: Campaign) => {
    const source = bindingsRef.current[campaign.id]?.source;
    const [located] = source && client.locateRecordings
      ? await client.locateRecordings([{ path: campaign.path, campaignKey: source.metadata.campaignKey }]).catch(() => [null]) : [null];
    if (located && (!sameCampaignPath(located, campaign.path) || recordingFilename(located) !== recordingFilename(campaign.path))) relocate(campaign.path, located);
    else void openCampaign(campaign.path, campaign.label, undefined, true, campaign.id);
  };

  return <div className="dashboard">
    {campaignHost && createPortal(<ReplayLibraryMenu library={replayLibrary} client={client} campaigns={campaigns} bindings={roles.bindings} defaultId={defaultRoleId} countryTags={countryTags}
      slots={campaignSlots(draftTemplate, campaigns)} errors={loadErrors} onLoad={(path, label, slotId) => { void openCampaign(path, label, slotId); }}
      onUnload={unloadCampaign} onDefault={setDefaultRoleId} onRenamed={relocate} relocations={relocations} onOpenReady={onOpenReady} />, campaignHost)}
    {(active || mapActive) && playbackHost && createPortal(<div className="header-workspace-bar">
    <PlaybackCountryPicker clock={clock} track={defaultTrack} source={defaultSource} client={client} onCountries={roles.observeCountries} onReconnect={() => {
      const campaign = campaignsRef.current.find(campaign => campaign.id === defaultRoleId);
      if (campaign) void reconnect(campaign);
    }} countries={defaultSource?.metadata.countries ?? []} playerCountryId={defaultSource?.metadata.playerCountryId ?? null}
      selectedTag={template.roles.find((role) => role.id === defaultRoleId)?.selectedTag ?? null}
      selectedSlot={template.roles.find((role) => role.id === defaultRoleId)?.selectedSlot ?? null}
      onChange={pickCountry} />
    <PlaybackStrip clock={clock} tracks={tracks} alignment={template.alignment} defaultRoleId={defaultRoleId} />
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
      options={mapOptions} onOptions={setMapOptions} poppedOut={popout.open} popoutGeneration={popout.generation} popoutProblem={popoutIssue} onPopoutRetry={retryPopout} popoutStopped={popoutStopped(popout)}
      popoutCameraMode={popout.status?.running ? popout.status.cameraMode : null}
      popoutCameraOverlays={popout.status?.running ? popout.status.cameraOverlays : false}
      aircraft={aircraft} onAircraft={chooseAircraft} popoutFlyAircraft={popout.status?.flyAircraft ?? null} aircraftHost={sideHost} />}
    {active && <>
      { }
      <div className="toolbar-swap" data-editing={editing || dirty || undefined}>
      <LayoutMenu layouts={library.layouts.map(entry => ({ id: entry.id, name: entry.template.name }))} activeId={library.activeId}
        disabled={editing || dirty || campaigns.some(campaign => campaign.loading)} onChoose={selectLayout} onEdit={() => {
          saveMenu.popup.current?.hidePopover();
          flushSync(() => setEditing(true));
          editButton.current?.focus();
        }} />
      <div className="dash-toolbar" role="toolbar" aria-label="Layout changes">
        <span className="toolbar-slot" data-shown={editing || undefined}><WidgetPicker runtime={runtime} placed={template.widgets} disabled={!editing} onOpen={() => saveMenu.popup.current?.hidePopover()} onAdd={(kind, initial) => {
          const widget = { ...createWidget(kind, template.tracks[0].id), ...initial };
          update((previous) => ({ ...previous, widgets: [...previous.widgets, widget], layouts: addGridItem(previous.layouts, widget) }));
        }} /></span>
        {!layoutAtDefault && <button ref={resetMenu.trigger} type="button" className="icon-button header-reset toolbar-slot" data-shown={editing || undefined} aria-label="Reset layout" data-tooltip="Reset arrangement to default"
          aria-haspopup={dirty ? 'dialog' : undefined} aria-expanded={dirty ? resetMenu.open : undefined} popoverTarget={dirty ? 'layout-reset-menu' : undefined}
          onClick={() => { if (!dirty) resetLayout(); else saveMenu.popup.current?.hidePopover(); }}><Icon name="refresh" /></button>}
        <Popover menu={resetMenu} id="layout-reset-menu" className="layout-reset-menu" aria-label="Reset to the default arrangement?">
          {resetMenu.open && <ConfirmStep confirmation={{ title: 'Reset to the default arrangement?', detail: 'Your unsaved changes to this layout are lost.', action: 'Reset' }}
            onCancel={resetMenu.close} onConfirm={() => { resetMenu.popup.current?.hidePopover(); resetLayout(); }} />}
        </Popover>
        <button ref={discardMenu.trigger} type="button" className="icon-button" aria-label="Discard layout changes" data-tooltip="Discard layout changes"
          aria-haspopup={dirty ? 'dialog' : undefined} aria-expanded={dirty ? discardMenu.open : undefined} popoverTarget={dirty ? 'layout-discard-menu' : undefined}
          onClick={() => { if (!dirty) discardLayout(); else saveMenu.popup.current?.hidePopover(); }}><Icon name="close" /></button>
        <Popover menu={discardMenu} id="layout-discard-menu" className="layout-discard-menu" aria-label="Discard layout changes?">
          {discardMenu.open && <ConfirmStep confirmation={{ title: 'Discard layout changes?', detail: 'Your unsaved arrangement is lost.', action: 'Discard' }}
            onCancel={discardMenu.close} onConfirm={() => { discardMenu.popup.current?.hidePopover(); discardLayout(); }} />}
        </Popover>
        <button ref={saveButton} type="button" className={`icon-button toolbar-save${dirty ? ' is-dirty' : ''}`} aria-label="Save layout" data-tooltip="Save layout" aria-expanded={saving} aria-haspopup="dialog" popoverTarget="layout-save-menu" onClick={() => {
          setLayoutName(selectedLayout.template.name); setSaveError(null); setDiscarding(false);
        }}><Icon name="save" /></button>
      </div>
      </div>
      <button ref={editButton} type="button" className={`button toolbar-edit${editing ? ' primary' : ''}`} aria-label={editing ? 'Done editing' : 'Edit layout'} data-tooltip={editing ? 'Done editing' : 'Edit layout'} aria-pressed={editing}
        aria-haspopup={editing && dirty ? 'dialog' : undefined} aria-expanded={editing && dirty ? saving : undefined} onClick={() => {
          if (editing && dirty) { openSave(); return; }
          saveMenu.popup.current?.hidePopover();
          flushSync(() => setEditing(!editing));
          if (!editButton.current?.checkVisibility()) editButton.current?.parentElement?.querySelector<HTMLElement>('.header-layout-trigger')?.focus();
        }}><Icon name={editing ? 'check' : 'edit'} /><span className="toolbar-label">{editing ? 'Done' : 'Edit'}</span></button>
      <Popover menu={saveMenu} id="layout-save-menu" className="layout-save-menu" aria-label="Save layout options">
      {saving && discarding && <div onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); keepChanges(); } }}>
        <ConfirmStep confirmation={{ title: 'Discard layout changes?', detail: 'Your unsaved arrangement is lost.', action: 'Discard' }}
          onCancel={keepChanges} onConfirm={() => { setDiscarding(false); discardLayout(); }} /></div>}
      {saving && !discarding && <form onSubmit={event => { event.preventDefault(); persistLayout(saveCheck.submit); }}>
        <div className="ui-popover-heading"><strong>Save layout</strong><button type="button" className="icon-button ui-popover-action" aria-label="Close save options" onClick={closeSave}><Icon name="close" /></button></div>
        <label className="field"><span>Layout name</span><input data-popover-autofocus aria-label="Layout name" value={layoutName} maxLength={160}
          aria-invalid={saveCheck.problem ? true : undefined} aria-describedby="layout-save-hint"
          onChange={(event) => { setLayoutName(event.currentTarget.value); setSaveError(null); }} /></label>
        {saveError ? <FailureLine className="is-top" id="layout-save-hint" message={saveError.text} onRetry={() => persistLayout(saveError.action)} retryDisabled={!layoutSaveAllowed(saveCheck, saveError.action)} />
          : saveCheck.problem ? <FailureLine className="is-top" id="layout-save-hint" message={saveCheck.problem} alert={false} />
          : <p className="field-hint" id="layout-save-hint">{saveCheck.submit === 'new' ? `Saves a new layout and keeps “${selectedLayout.template.name}” as it was.`
            : 'Saves your changes to this layout. Enter a new name to keep it and save a new layout instead.'}</p>}
        <div className="layout-save-actions">
          {dirty && <button type="button" className="button layout-save-discard" onClick={() => setDiscarding(true)}>Discard changes</button>}
          <button type={saveCheck.submit === 'update' ? 'submit' : 'button'} className={`button${saveCheck.submit === 'update' ? ' primary' : ''}`} disabled={!saveCheck.update}
            onClick={saveCheck.submit === 'update' ? undefined : () => persistLayout('update')}>{saveCheck.renames && saveCheck.submit !== 'update' ? 'Rename and update' : 'Update layout'}</button>
          <button type={saveCheck.submit === 'new' ? 'submit' : 'button'} className={`button${saveCheck.submit === 'new' ? ' primary' : ''}`} disabled={!saveCheck.asNew}
            onClick={saveCheck.submit === 'new' ? undefined : () => persistLayout('new')}>Save as new</button></div>
      </form>}
      </Popover>
    </>}
    </div>
    </div>, playbackHost)}
    {(warning || actionError) && <div className={`notice dash-banner ${actionError ? 'error' : 'warning'}`} role="alert" data-detail={actionError?.detail}>
      <Icon name="alert" /><span>{actionError ? actionError.text : warning}</span>
      <DismissButton onDismiss={() => { setActionError(null); setWarning(null); }} /></div>}
    {saved && savedVisible && <div className="layout-saved" role="status"><Icon name="check" />{saved}</div>}
    <div className={`dash-grid-scroll${editing ? ' is-editing' : ''}`} ref={viewport}>
      {template.widgets.length === 0 ? <div className="dash-empty-dashboard"><EmptyState icon="grid" title="This page has no widgets yet">
        <p>{defaultTemplate(shownPage).widgets.length ? 'Add widgets, or reset the page to its default arrangement from Edit.' : 'Add widgets to build your own view of the campaign.'}</p>
        <button type="button" className="button primary" onClick={() => {
          saveMenu.popup.current?.hidePopover();
          flushSync(() => setEditing(true));
          playbackHost?.querySelector<HTMLButtonElement>('.header-add-widget')?.click();
        }}><Icon name="plus" />Add widgets</button></EmptyState></div>
        : active && <LiveMotion.Provider value={clock.getMotion}><DashboardGrid template={template} runtime={runtime} editing={editing} onLayouts={layoutsChanged} onApply={applyWidget} onRemove={removeWidget} /></LiveMotion.Provider>}
      <PreparedPages pages={preparedPages} clock={clock} client={client} viewport={viewportBox}
        enabled={active && !editing && !saving && client.available && campaigns.some(campaign => !campaign.loading)} draw={drawPrepared} mounting={bodiesWaiting} />
    </div>
  </div>;
}

commitZone(Dashboard, 'dashboard');

const noop = () => {};
const drawPrepared = (page: PreparedPage) =>
  <DashboardGrid prepared template={page.template} runtime={page.runtime} editing={false} onLayouts={noop} onApply={noop} onRemove={noop} />;
