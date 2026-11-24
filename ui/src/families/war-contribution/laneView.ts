import type { LaneView, Painted, PaintInput, Scene } from '../../draw/lane.ts';
import type { WarContributionsReply, WarTimeline } from '../../generated/dashboard';
import { observationAt, warContributions } from '../../widgets/wars/warsData.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { createBoundedReadView } from '../boundedView.ts';
import type { FamilyTheme } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { ArtProvider, EntityKind, IdentityProvider } from '../plan.ts';
import { hitTestWarContribution, layoutWarContribution, type WarContributionLayout } from './layout.ts';
import {
  buildWCTooltip,
  deriveWCAudit,
  deriveWCNavAndMirror,
  projectWarContribution,
  type WCMeasureKey,
  type WCShowMode,
  type WCViewMode,
  type WarContributionModel,
} from './model.ts';
import { drawWarContribution } from './painter.ts';

export interface FamilyWarContributionBinding {
  readonly family: 'war-contribution';
  readonly view: WCViewMode;
  readonly show: WCShowMode;
  readonly sortKey: WCMeasureKey;
  readonly selectedMeasure: WCMeasureKey;
  readonly war: WarTimeline | null;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticReply?: WarContributionsReply | null;
  readonly extraWarnings?: readonly string[];
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly scrollLeftLeft: number;
  readonly scrollLeftRight: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly focusedKey?: string | null;
  readonly audit?: boolean;
  readonly auditId?: number;
}

export function isFamilyWarContributionBinding(value: unknown): value is FamilyWarContributionBinding {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'war-contribution';
}

type FieldEntry = { value: unknown; content: string };

interface HeldWarContribution {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  content: string;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artProvider: ArtProvider;
  kept: Map<string, { text: string; content: string }>;
  wake?: () => void;
  paintSig: string;
}

function cachedField(held: HeldWarContribution, name: string, text: string, content: () => string): string {
  const existing = held.kept.get(name);
  if (existing && existing.text === text) return existing.content;
  const next = { text, content: content() };
  held.kept.set(name, next);
  return next.content;
}

function replyFromScene(scene: Scene, bound: FamilyWarContributionBinding): WarContributionsReply | null {
  if (bound.staticReply) return bound.staticReply;
  const subId = bound.sources.contributions;
  if (subId === undefined) return null;
  const at = scene.subs.findIndex(s => s.id === subId);
  const head = at < 0 ? undefined : scene.pictures[at]?.parts[0]?.value;
  if (head?.kind === 'panel' && head.refused === null) {
    return head.data as WarContributionsReply;
  }
  return null;
}

export function familyWarContributionLaneView(
  wake: (widget: number) => void,
  laneOptions?: { readonly fetchBitmap?: LaneArtFetcher },
): LaneView {
  return {
    next(): number {
      return Infinity;
    },
    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('familyWarContribution') as HeldWarContribution | undefined;
      if (held) {
        held.art.dispose();
        state.delete('familyWarContribution');
      }
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyWarContributionBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };

      let held = state.get('familyWarContribution') as HeldWarContribution | undefined;
      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(bound.width * dpr));
      const heightPx = Math.max(1, Math.round(bound.height * dpr));

      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const ctx = canvas.getContext('2d')!;
        const art = new LaneArtCache({ fetchBitmap: laneOptions?.fetchBitmap });
        const artProvider = createLaneArtProvider(art);
        state.set('familyWarContribution', held = {
          canvas,
          ctx,
          content: content(),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artProvider,
          kept: new Map(),
          paintSig: '',
        });
        const holder = held;
        art.setOnSettle(() => {
          holder.paintSig = '';
          holder.wake?.();
        });
      }

      held.wake = () => wake(input.widget);
      if (held.identitySnapshot !== bound.identitySnapshot) {
        held.identitySnapshot = bound.identitySnapshot;
        held.identityIndex = bound.identitySnapshot ? IdentityIndex.fromSnapshot(bound.identitySnapshot) : null;
        held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        held.artProvider = createLaneArtProvider(held.art, { index: held.identityIndex });
      }
      if (held.canvas.width !== widthPx || held.canvas.height !== heightPx) {
        held.canvas.width = widthPx;
        held.canvas.height = heightPx;
        held.paintSig = '';
      }
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const pendingDemands = new Map<string, readonly [EntityKind, string, number]>();
      const demandedArt = new Set<string>();
      const reply = replyFromScene(input, bound);
      const effectiveHour = reply ? reply.hour : Math.floor(input.hour);
      const readView = createBoundedReadView({
        hour: effectiveHour,
        revisions: {},
        ensureNonLive: () => ({ panels: new Map(), answers: new Map(), refused: new Map() }),
        identity: held.identityProvider,
        onIdentityDemand: (kind, key, atHour) => {
          pendingDemands.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]);
        },
        art: held.artProvider,
        onArtDemand: handle => {
          const url = artUrlForHandle(handle, held!.identityIndex);
          if (url) demandedArt.add(url);
        },
      });

      const observation = reply && bound.war ? observationAt(bound.war, reply.hour) : null;
      const group = reply ? warContributions(reply.contributions, observation) : null;
      const model: WarContributionModel = projectWarContribution(
        readView,
        group,
        observation,
        bound.show,
        bound.theme?.surface ?? '#16181d',
        [...(bound.extraWarnings ?? []), ...(reply?.warnings ?? []), ...(group?.reason ? [group.reason] : [])],
      );

      for (const m of [...model.left.members, ...model.right.members]) {
        if (m.flagArt?.status === 'ready' && m.flagArt.url) demandedArt.add(m.flagArt.url);
      }
      if (demandedArt.size) held.art.requestVisible(demandedArt);

      const inspectedKey = typeof input.inspect === 'string' && (input.inspect.startsWith('row:') || input.inspect.startsWith('measure:'))
        ? (input.inspect.startsWith('row:') ? input.inspect.slice(4) : input.inspect)
        : null;
      const focusedKey = bound.focusedKey ?? null;

      let layout: WarContributionLayout | null = null;
      if (bound.theme && !model.empty) {
        layout = layoutWarContribution({
          model,
          view: bound.view,
          sortKey: bound.sortKey,
          selectedMeasure: bound.selectedMeasure,
          width: bound.width,
          height: bound.height,
        });
        const paintSig = JSON.stringify([
          bound.view,
          bound.show,
          bound.sortKey,
          bound.selectedMeasure,
          bound.width,
          bound.height,
          bound.scrollTop,
          bound.scrollLeftLeft,
          bound.scrollLeftRight,
          inspectedKey,
          focusedKey,
          model.hour,
          model.left.members.length,
          model.right.members.length,
          held.art.cachedCount,
        ]);
        if (paintSig !== held.paintSig) {
          drawWarContribution(held.ctx, {
            model,
            layout,
            theme: bound.theme,
            scrollTop: bound.scrollTop,
            scrollLeftLeft: bound.scrollLeftLeft,
            scrollLeftRight: bound.scrollLeftRight,
            inspectedKey,
            focusedKey,
          });
          held.paintSig = paintSig;
          held.content = content();
        }
      }

      const fields: Record<string, FieldEntry> = {};
      const demandsList = [...pendingDemands.values()];
      if (demandsList.length) {
        fields.identityDemands = { value: demandsList, content: cachedField(held, 'identityDemands', JSON.stringify(demandsList), content) };
      }

      const metaVal = {
        empty: model.empty ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        warnings: model.warnings ?? [],
        warTitle: model.warTitle,
        leftLeadTag: model.left.leadTag,
        rightLeadTag: model.right.leadTag,
        leftLeadName: model.left.leadName,
        rightLeadName: model.right.leadName,
        contentHeight: layout?.contentHeight ?? bound.height,
        maxScrollLeft: layout?.view === 'sides'
          ? Math.max(0, layout.panels[0].measureTotalWidth - layout.panels[0].measureViewportWidth)
          : 0,
      };
      fields.meta = { value: metaVal, content: cachedField(held, 'meta', JSON.stringify(metaVal), content) };

      const activeKey = bound.view === 'sides' ? bound.sortKey : bound.selectedMeasure;
      const { nav, mirror } = deriveWCNavAndMirror(model, activeKey);
      fields.nav = { value: nav, content: cachedField(held, 'nav', JSON.stringify(nav), content) };
      fields.mirror = { value: mirror, content: cachedField(held, 'mirror', JSON.stringify(mirror), content) };

      if (bound.theme) {
        if (inspectedKey) {
          const tip = buildWCTooltip(model, inspectedKey, activeKey, bound.theme);
          const payload = { key: `row:${inspectedKey}`, text: tip ? { hour: model.hour, rowKey: inspectedKey, content: tip } : null };
          fields.tooltip = { value: payload, content: cachedField(held, 'tooltip', JSON.stringify(payload), content) };
        }
        if (focusedKey) {
          const tip = buildWCTooltip(model, focusedKey, activeKey, bound.theme);
          const payload = { key: `focus:${focusedKey}`, text: tip ? { hour: model.hour, rowKey: focusedKey, content: tip } : null };
          fields.focusTooltip = { value: payload, content: cachedField(held, 'focusTooltip', JSON.stringify(payload), content) };
        }
      }

      if (bound.audit) {
        const auditRows = deriveWCAudit(model, activeKey);
        const auditVal = { id: bound.auditId ?? -1, rows: auditRows };
        fields.audit = { value: auditVal, content: cachedField(held, 'audit', JSON.stringify(auditVal), content) };
      }

      const activeLayout = layout;
      const sTop = bound.scrollTop;
      const sLeftL = bound.scrollLeftLeft;
      const sLeftR = bound.scrollLeftRight;

      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        hit: activeLayout ? pt => hitTestWarContribution(activeLayout, pt[0], pt[1], sTop, sLeftL, sLeftR) : undefined,
      };
    },
  };
}
