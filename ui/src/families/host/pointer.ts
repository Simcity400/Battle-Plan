import type { Pipeline, PointerHitMeta } from '../../dashboard/follow/pipeline.ts';
import type { InstalledPicture, InstalledViewport } from '../plan.ts';
import type { FamilyPointerMode, ParsedFamilyHit } from './adapter.ts';
import type { FamilyTooltipController, FamilyTooltipField } from './tooltips.ts';

export const CLICK_CANCEL_PX = 4;

export interface FamilyPointerHit {
  readonly key: string;
  readonly toggle: boolean;
}

export interface FamilyPointerGestures {
  readonly onWheel?: (event: WheelEvent, point: readonly [number, number]) => boolean | void;
  readonly onDragStart?: (point: readonly [number, number]) => void;
  readonly onDragMove?: (point: readonly [number, number], delta: readonly [number, number]) => void;
  readonly onDragEnd?: () => void;
  readonly onDoubleClick?: (point: readonly [number, number]) => boolean | void;
}

export function clientPointToInstalled(
  clientX: number,
  clientY: number,
  rect: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>,
  viewport?: Pick<InstalledViewport, 'width' | 'height'> | null,
): [number, number] {
  const localX = clientX - rect.left;
  const localY = clientY - rect.top;
  if (!viewport || rect.width <= 0 || rect.height <= 0 || viewport.width <= 0 || viewport.height <= 0) {
    return [localX, localY];
  }
  return [localX * (viewport.width / rect.width), localY * (viewport.height / rect.height)];
}

export function parseDefaultRowHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey) return null;
  const toggle = hitKey.startsWith('toggle:');
  if (!toggle && !hitKey.startsWith('row:')) return null;
  const key = hitKey.slice(hitKey.indexOf(':') + 1);
  return key ? { key, subject: key, toggle, kind: toggle ? 'toggle' : 'row', inspectToken: `row:${key}` } : null;
}

export interface FamilyRowDrag {
  readonly starts: (hit: ParsedFamilyHit) => boolean;
  readonly move: (point: readonly [number, number]) => void;
  readonly end: (point: readonly [number, number] | null) => void;
}

export interface FamilyPointerBindingOptions {
  readonly bodyNode: HTMLElement;
  readonly pictureNode: HTMLElement;
  readonly menuHost: HTMLElement;
  readonly pipeline: Pipeline;
  readonly pipe: number;
  readonly tooltips: FamilyTooltipController;
  readonly inspectedKeyRef: { current: string | null };
  readonly inspectedHitRef?: { current: ParsedFamilyHit | null };
  readonly lastPointerRef: { current: { x: number; y: number } | null };
  readonly focusInsideRef: { readonly current: boolean };
  readonly focusedKeyRef: { readonly current: string | null };
  readonly focusAnchorRef: { readonly current: { x: number; y: number } };
  readonly accessibleRef: { readonly current: HTMLElement | null };
  readonly pointerMode?: FamilyPointerMode;
  readonly isBlocked: () => boolean;
  readonly getViewportHeight: () => number;
  readonly getInstalled?: () => InstalledPicture | null;
  readonly parseHit?: (hitKey: string | null, installed: InstalledPicture | null) => ParsedFamilyHit | FamilyPointerHit | null;
  readonly inspectToken?: (key: string) => string;
  readonly onHoverChange?: (hit: ParsedFamilyHit | null) => void;
  readonly onHoverHit?: (hit: ParsedFamilyHit | null) => void;
  readonly gestures?: FamilyPointerGestures;
  readonly rowDrag?: FamilyRowDrag;
  readonly onActivate: (key: string, toggle: boolean, hit: ParsedFamilyHit | null, picture: InstalledPicture | null) => void;
  readonly onContextMenu?: (key: string | null, clientX: number, clientY: number, hit?: ParsedFamilyHit | null, picture?: InstalledPicture | null) => boolean;
}

interface PictureStamp {
  readonly installedId: number | undefined;
  readonly owner: string | undefined;
}

interface PointerDownState extends PictureStamp {
  picture: InstalledPicture | null;
  key: string | null;
  hit: ParsedFamilyHit | null;
  resolved: boolean;
  pointerId: number;
  x: number;
  y: number;
  startPoint: readonly [number, number];
  button: number;
  toggle: boolean;
  cancelled: boolean;
  dragging: boolean;
  rowDrag: boolean;
}

export interface FamilyPointerBinding {
  (): void;
  readonly clearHover: () => void;
}

export function bindFamilyPointer(options: FamilyPointerBindingOptions): FamilyPointerBinding {
  const {
    bodyNode,
    pictureNode,
    menuHost,
    pipeline,
    pipe,
    tooltips,
    inspectedKeyRef,
    inspectedHitRef,
    lastPointerRef,
    focusInsideRef,
    focusedKeyRef,
    focusAnchorRef,
    accessibleRef,
    pointerMode = 'discrete',
    isBlocked,
    getViewportHeight,
    getInstalled,
    onHoverChange,
    onHoverHit,
    gestures,
    rowDrag,
    onActivate,
    onContextMenu,
  } = options;
  const parseHit = options.parseHit ?? parseDefaultRowHit;
  const inspectToken = options.inspectToken ?? ((key: string) => `row:${key}`);
  let pointerDown: PointerDownState | null = null;
  let lastHoverKey: string | null = null;
  let lastHoverHour: number | undefined;
  let lastHoverPicture: number | undefined;
  let hoverVisit = 0;

  const installedNow = (): InstalledPicture | null => getInstalled?.() ?? null;

  const tokenFor = (hit: ParsedFamilyHit): string => hit.inspectToken ?? inspectToken(hit.key);

  const notifyHover = (hit: ParsedFamilyHit | null, picture?: number) => {
    if (inspectedHitRef) inspectedHitRef.current = hit;
    const nextKey = hit?.key ?? null;
    const nextHour = hit?.hour;
    const moved = nextKey !== lastHoverKey || nextHour !== lastHoverHour;
    const repainted = nextKey !== null && picture !== lastHoverPicture;
    lastHoverPicture = picture;
    if (moved || repainted) onHoverHit?.(hit);
    if (moved) {
      lastHoverKey = nextKey;
      lastHoverHour = nextHour;
      onHoverChange?.(hit);
    }
  };

  const acceptsReply = (meta: PointerHitMeta | undefined, stamp: PictureStamp): boolean => {
    if (meta?.installedId !== undefined && meta.installedId !== stamp.installedId) return false;
    if (meta?.owner !== undefined && meta.owner !== stamp.owner) return false;
    if (!getInstalled) return true;
    const cur = installedNow();
    return cur !== null && cur.owner === stamp.owner;
  };

  const toPoint = (clientX: number, clientY: number): [number, number] => {
    const box = pictureNode.getBoundingClientRect();
    return clientPointToInstalled(clientX, clientY, box, installedNow()?.viewport);
  };

  const sameOwner = (stamp: PictureStamp): boolean => {
    if (!getInstalled) return true;
    const cur = installedNow();
    return cur !== null && cur.owner === stamp.owner;
  };

  const dragLive = (down: PointerDownState): boolean => !isBlocked() && sameOwner(down);

  const hover = (clientX: number, clientY: number) => {
    const moved = installedNow();
    const stamp: PictureStamp = { installedId: moved?.id, owner: moved?.owner };
    const visit = hoverVisit;
    pipeline.pointer(pipe, 0, toPoint(clientX, clientY), (hitKey, hitMeta) => {
      if (isBlocked() || visit !== hoverVisit || !acceptsReply(hitMeta, stamp)) return;
      if (getInstalled && installedNow()?.id !== stamp.installedId) {
        const last = lastPointerRef.current;
        if (last) hover(last.x, last.y);
        return;
      }
      const parsed = parseHit(hitKey, moved);
      const nextKey = parsed?.key ?? null;
      const changed = nextKey !== inspectedKeyRef.current;
      inspectedKeyRef.current = nextKey;
      notifyHover(parsed, moved?.id);
      pipeline.inspect(pipe, parsed ? tokenFor(parsed) : null);
      if (!nextKey) {
        tooltips.cancelHoverOpen();
        if (!focusInsideRef.current) tooltips.hideIfUnpinned();
        return;
      }
      if (changed || (pointerMode === 'continuous' && tooltips.isHoverShown())) {
        tooltips.scheduleHoverOpen(() => {
          const tipField = pipeline.view(pipe)?.fields?.tooltip as FamilyTooltipField | undefined;
          const tip = tipField?.text && tipField.text.rowKey === inspectedKeyRef.current ? tipField.text : null;
          return {
            tip,
            pos: lastPointerRef.current,
            viewportHeight: getViewportHeight(),
            blocked: isBlocked(),
          };
        });
      }
    });
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!event.isTrusted) return;
    if (pointerDown) {
      const dx = event.clientX - pointerDown.x;
      const dy = event.clientY - pointerDown.y;
      if (!pointerDown.cancelled && Math.hypot(dx, dy) > CLICK_CANCEL_PX) {
        pointerDown.cancelled = true;
        if (pointerDown.button === 0 && dragLive(pointerDown) && pointerDown.hit && rowDrag?.starts(pointerDown.hit)) {
          pointerDown.rowDrag = true;
          try { bodyNode.setPointerCapture?.(pointerDown.pointerId); } catch {}
        } else if (pointerDown.button === 0 && dragLive(pointerDown) && gestures?.onDragMove) {
          pointerDown.dragging = true;
          try { bodyNode.setPointerCapture?.(pointerDown.pointerId); } catch {}
          gestures.onDragStart?.(pointerDown.startPoint);
        }
      }
      if (pointerDown.rowDrag || pointerDown.dragging) {
        if (!dragLive(pointerDown)) {
          const down = pointerDown;
          pointerDown = null;
          endDrag(down);
          return;
        }
        if (pointerDown.rowDrag) rowDrag?.move(toPoint(event.clientX, event.clientY));
        else gestures?.onDragMove?.(toPoint(event.clientX, event.clientY), [dx, dy]);
        return;
      }
    }
    if (isBlocked()) return;
    const last = lastPointerRef.current;
    if (last && last.x === event.clientX && last.y === event.clientY) return;
    lastPointerRef.current = { x: event.clientX, y: event.clientY };
    tooltips.cancelHoverLeave();
    hover(event.clientX, event.clientY);
  };

  const onPointerLeave = () => {
    lastPointerRef.current = null;
    hoverVisit++;
    tooltips.scheduleHoverLeave(() => {
      inspectedKeyRef.current = null;
      notifyHover(null);
      pipeline.pointer(pipe, 0, null);
      pipeline.inspect(pipe, null);
      if (!tooltips.isPinned()) {
        if (focusInsideRef.current && focusedKeyRef.current && !isBlocked()) {
          const focusTipField = pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined;
          if (focusTipField?.text && focusTipField.text.rowKey === focusedKeyRef.current) {
            tooltips.show('focus', focusTipField.text, focusAnchorRef.current, getViewportHeight());
            return;
          }
        }
        tooltips.hideIfUnpinned();
      }
    });
  };

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0 && event.button !== 2) return;
    if (isBlocked()) return;
    const snap = installedNow();
    if (getInstalled && !snap) return;
    const x = event.clientX, y = event.clientY;
    const point = toPoint(x, y);
    const down: PointerDownState = {
      picture: snap,
      key: null,
      hit: null,
      installedId: snap?.id,
      owner: snap?.owner,
      resolved: false,
      pointerId: event.pointerId,
      x,
      y,
      startPoint: point,
      button: event.button,
      toggle: false,
      cancelled: false,
      dragging: false,
      rowDrag: false,
    };
    pointerDown = down;
    pipeline.pointer(pipe, 0, point, (hitKey, hitMeta) => {
      if (down.cancelled) return;
      if (!acceptsReply(hitMeta, down)) {
        down.key = null;
        down.hit = null;
        down.resolved = true;
        return;
      }
      const parsed = parseHit(hitKey, down.picture);
      const rowKey = parsed?.key ?? null;
      down.key = rowKey;
      down.hit = parsed;
      down.toggle = parsed?.toggle === true;
      down.resolved = true;
    });
  };

  const endDrag = (down: PointerDownState, point: readonly [number, number] | null = null) => {
    if (down.rowDrag) {
      down.rowDrag = false;
      try { bodyNode.releasePointerCapture?.(down.pointerId); } catch {}
      rowDrag?.end(point);
      return;
    }
    if (!down.dragging) return;
    down.dragging = false;
    try { bodyNode.releasePointerCapture?.(down.pointerId); } catch {}
    gestures?.onDragEnd?.();
  };

  const onPointerUp = (event: PointerEvent) => {
    const down = pointerDown;
    if (!down) return;
    if (down.dragging || down.rowDrag) {
      pointerDown = null;
      endDrag(down, dragLive(down) ? toPoint(event.clientX, event.clientY) : null);
      return;
    }
    if (down.cancelled || down.button !== 0 || isBlocked()) {
      pointerDown = null;
      return;
    }
    if (Math.hypot(event.clientX - down.x, event.clientY - down.y) > CLICK_CANCEL_PX) {
      down.cancelled = true;
      pointerDown = null;
      return;
    }
    pointerDown = null;
    const picture = installedNow();
    if (getInstalled && !picture) return;
    const stamp: PictureStamp = { installedId: picture?.id, owner: picture?.owner };
    if (stamp.owner !== down.owner) return;
    pipeline.pointer(pipe, 0, toPoint(event.clientX, event.clientY), (hitKey, hitMeta) => {
      if (isBlocked() || !acceptsReply(hitMeta, stamp)) return;
      const released = parseHit(hitKey, picture);
      if (!released || !down.resolved || released.key !== down.key) return;
      onActivate(released.key, released.toggle === true, released, picture);
    });
  };

  const onPointerCancel = () => {
    const down = pointerDown;
    if (down) {
      down.cancelled = true;
      endDrag(down);
    }
    pointerDown = null;
  };

  const handleContextMenu = (event: MouseEvent) => {
    if (isBlocked() || !onContextMenu) return;
    const target = event.target as Node | null;
    if (focusInsideRef.current && !bodyNode.contains(target) && accessibleRef.current?.contains(target)) {
      if (onContextMenu(focusedKeyRef.current, event.clientX, event.clientY, null, installedNow())) event.preventDefault();
      return;
    }
    const picture = installedNow();
    if (getInstalled && !picture) return;
    event.preventDefault();
    const { clientX, clientY } = event;
    const stamp: PictureStamp = { installedId: picture?.id, owner: picture?.owner };
    pipeline.pointer(pipe, 0, toPoint(clientX, clientY), (hitKey, hitMeta) => {
      if (isBlocked() || !acceptsReply(hitMeta, stamp)) return;
      const parsed = parseHit(hitKey, picture);
      onContextMenu(parsed?.key ?? null, clientX, clientY, parsed, picture);
    });
  };

  const onWheel = gestures?.onWheel
    ? (event: WheelEvent) => {
        if (isBlocked()) return;
        if (gestures.onWheel?.(event, toPoint(event.clientX, event.clientY)) === true) {
          event.preventDefault();
        }
      }
    : null;

  const onDoubleClick = gestures?.onDoubleClick
    ? (event: MouseEvent) => {
        if (isBlocked()) return;
        if (gestures.onDoubleClick?.(toPoint(event.clientX, event.clientY)) === true) {
          event.preventDefault();
        }
      }
    : null;

  bodyNode.addEventListener('pointermove', onPointerMove);
  bodyNode.addEventListener('pointerleave', onPointerLeave);
  bodyNode.addEventListener('pointerdown', onPointerDown);
  bodyNode.addEventListener('pointerup', onPointerUp);
  bodyNode.addEventListener('pointercancel', onPointerCancel);
  if (onWheel) bodyNode.addEventListener('wheel', onWheel, { passive: false });
  if (onDoubleClick) bodyNode.addEventListener('dblclick', onDoubleClick);
  menuHost.addEventListener('contextmenu', handleContextMenu);

  const clearHover = () => {
    tooltips.cancelHoverOpen();
    lastPointerRef.current = null;
    hoverVisit++;
    if (inspectedKeyRef.current === null && lastHoverKey === null && lastHoverHour === undefined) return;
    inspectedKeyRef.current = null;
    notifyHover(null);
    pipeline.inspect(pipe, null);
    if (!focusInsideRef.current) tooltips.hideIfUnpinned();
  };

  return Object.assign(() => {
    if (pointerDown) endDrag(pointerDown);
    pointerDown = null;
    tooltips.cancelTimers();
    lastPointerRef.current = null;
    hoverVisit++;
    if (inspectedKeyRef.current !== null || lastHoverKey !== null || lastHoverHour !== undefined) {
      inspectedKeyRef.current = null;
      notifyHover(null);
      pipeline.pointer(pipe, 0, null);
      pipeline.inspect(pipe, null);
    }
    tooltips.hideIfUnpinned();
    bodyNode.removeEventListener('pointermove', onPointerMove);
    bodyNode.removeEventListener('pointerleave', onPointerLeave);
    bodyNode.removeEventListener('pointerdown', onPointerDown);
    bodyNode.removeEventListener('pointerup', onPointerUp);
    bodyNode.removeEventListener('pointercancel', onPointerCancel);
    if (onWheel) bodyNode.removeEventListener('wheel', onWheel);
    if (onDoubleClick) bodyNode.removeEventListener('dblclick', onDoubleClick);
    menuHost.removeEventListener('contextmenu', handleContextMenu);
  }, { clearHover });
}
