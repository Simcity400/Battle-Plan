import type { Pipeline, PointerHitMeta } from '../../dashboard/follow/pipeline.ts';
import type { InstalledAction, InstalledPicture, InstalledViewport } from '../plan.ts';
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

export function resolveInstalledAction(
  installed: InstalledPicture | null | undefined,
  key: string,
  toggle: boolean,
): InstalledAction | null {
  if (!installed) return null;
  const target = installed.actionKeys.get(key);
  return {
    installedId: installed.id,
    owner: installed.owner,
    subject: target?.subject ?? key,
    hour: target?.hour ?? installed.hour,
    ...(toggle || target?.toggle ? { toggle: true } : {}),
    ...(target?.label !== undefined ? { label: target.label } : {}),
    ...(target?.meta !== undefined ? { meta: target.meta } : {}),
  };
}

export function parseDefaultRowHit(hitKey: string | null): FamilyPointerHit | null {
  if (!hitKey) return null;
  const toggle = hitKey.startsWith('toggle:');
  if (!toggle && !hitKey.startsWith('row:')) return null;
  const key = hitKey.slice(hitKey.indexOf(':') + 1);
  return key ? { key, toggle } : null;
}

export interface FamilyPointerBindingOptions {
  readonly bodyNode: HTMLElement;
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
  readonly gestures?: FamilyPointerGestures;
  readonly onActivate: (key: string, toggle: boolean, action?: InstalledAction, hit?: ParsedFamilyHit | null) => void;
  readonly onContextMenu?: (key: string | null, clientX: number, clientY: number, hit?: ParsedFamilyHit | null) => boolean;
}

interface PointerDownState {
  key: string | null;
  hit: ParsedFamilyHit | null;
  installedId: number | undefined;
  owner: string | undefined;
  resolved: boolean;
  pendingUp: boolean;
  pointerId: number;
  x: number;
  y: number;
  startPoint: readonly [number, number];
  button: number;
  toggle: boolean;
  cancelled: boolean;
  dragging: boolean;
}

export interface FamilyPointerBinding {
  (): void;
  readonly clearHover: () => void;
}

export function bindFamilyPointer(options: FamilyPointerBindingOptions): FamilyPointerBinding {
  const {
    bodyNode,
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
    gestures,
    onActivate,
    onContextMenu,
  } = options;
  const parseHit = options.parseHit ?? parseDefaultRowHit;
  const inspectToken = options.inspectToken ?? ((key: string) => `row:${key}`);
  let pointerDown: PointerDownState | null = null;
  let lastHoverKey: string | null = null;
  let lastHoverHour: number | undefined;

  const installedNow = (): InstalledPicture | null => getInstalled?.() ?? null;

  const tokenFor = (hit: ParsedFamilyHit): string => hit.inspectToken ?? inspectToken(hit.key);

  const notifyHover = (hit: ParsedFamilyHit | null) => {
    if (inspectedHitRef) inspectedHitRef.current = hit;
    const nextKey = hit?.key ?? null;
    const nextHour = hit?.hour;
    if (nextKey !== lastHoverKey || nextHour !== lastHoverHour) {
      lastHoverKey = nextKey;
      lastHoverHour = nextHour;
      onHoverChange?.(hit);
    }
  };

  const matchesInstalled = (meta: PointerHitMeta | undefined, down?: PointerDownState): boolean => {
    const cur = installedNow();
    if (cur) {
      if (meta?.owner !== undefined && meta.owner !== cur.owner) return false;
      if (down?.owner !== undefined && down.owner !== cur.owner) return false;
      if (meta?.installedId !== undefined && meta.installedId !== cur.id) return false;
    } else if (getInstalled) {
      return false;
    }
    if (down?.installedId !== undefined && meta?.installedId !== undefined && meta.installedId !== down.installedId) {
      return false;
    }
    return true;
  };

  const toPoint = (clientX: number, clientY: number): [number, number] => {
    const box = bodyNode.getBoundingClientRect();
    return clientPointToInstalled(clientX, clientY, box, installedNow()?.viewport);
  };

  const dispatchActivate = (key: string, toggle: boolean, down: PointerDownState) => {
    const cur = installedNow();
    if (getInstalled) {
      if (!cur) return;
      if (down.owner !== undefined && cur.owner !== down.owner) return;
      const base = resolveInstalledAction(cur, key, toggle);
      if (!base) return;
      const hit = down.hit;
      const action: InstalledAction = {
        ...base,
        subject: hit?.subject ?? base.subject,
        hour: (hit?.hour ?? base.hour) as InstalledAction['hour'],
        ...(hit?.label !== undefined ? { label: hit.label } : {}),
        ...(hit?.meta !== undefined ? { meta: hit.meta } : {}),
      };
      onActivate(action.subject, toggle, action, hit);
      return;
    }
    onActivate(key, toggle, undefined, down.hit);
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!event.isTrusted) return;
    if (pointerDown) {
      const dx = event.clientX - pointerDown.x;
      const dy = event.clientY - pointerDown.y;
      if (!pointerDown.cancelled && Math.hypot(dx, dy) > CLICK_CANCEL_PX) {
        pointerDown.cancelled = true;
        if (pointerDown.button === 0 && !isBlocked() && gestures?.onDragMove) {
          pointerDown.dragging = true;
          try { bodyNode.setPointerCapture?.(pointerDown.pointerId); } catch {}
          gestures.onDragStart?.(pointerDown.startPoint);
        }
      }
      if (pointerDown.dragging) {
        gestures?.onDragMove?.(toPoint(event.clientX, event.clientY), [dx, dy]);
        return;
      }
    }
    if (isBlocked()) return;
    const last = lastPointerRef.current;
    if (last && last.x === event.clientX && last.y === event.clientY) return;
    lastPointerRef.current = { x: event.clientX, y: event.clientY };
    tooltips.cancelHoverLeave();
    pipeline.pointer(pipe, 0, toPoint(event.clientX, event.clientY), (hitKey, hitMeta) => {
      if (isBlocked() || !matchesInstalled(hitMeta)) return;
      const parsed = parseHit(hitKey, installedNow());
      const nextKey = parsed?.key ?? null;
      const changed = nextKey !== inspectedKeyRef.current;
      inspectedKeyRef.current = nextKey;
      notifyHover(parsed);
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

  const onPointerLeave = () => {
    lastPointerRef.current = null;
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
    const currentHit = inspectedHitRef?.current ?? null;
    const down: PointerDownState = {
      key: inspectedKeyRef.current,
      hit: currentHit,
      installedId: snap?.id,
      owner: snap?.owner,
      resolved: !!inspectedKeyRef.current,
      pendingUp: false,
      pointerId: event.pointerId,
      x,
      y,
      startPoint: point,
      button: event.button,
      toggle: currentHit?.toggle === true,
      cancelled: false,
      dragging: false,
    };
    pointerDown = down;
    pipeline.pointer(pipe, 0, point, (hitKey, hitMeta) => {
      if (down.cancelled) return;
      if (!matchesInstalled(hitMeta, down)) {
        down.key = null;
        down.hit = null;
        down.resolved = true;
        if (pointerDown === down && down.pendingUp) pointerDown = null;
        return;
      }
      const parsed = parseHit(hitKey, installedNow());
      const rowKey = parsed?.key ?? null;
      const isToggle = parsed?.toggle === true;
      down.key = rowKey;
      down.hit = parsed;
      down.toggle = isToggle;
      down.resolved = true;
      if (parsed && rowKey) {
        inspectedKeyRef.current = rowKey;
        notifyHover(parsed);
        pipeline.inspect(pipe, tokenFor(parsed));
      }
      if (down.pendingUp) {
        if (pointerDown === down) pointerDown = null;
        if (!isBlocked() && down.button === 0 && rowKey) {
          dispatchActivate(rowKey, isToggle, down);
        }
      }
    });
  };

  const endDrag = (down: PointerDownState) => {
    if (!down.dragging) return;
    down.dragging = false;
    try { bodyNode.releasePointerCapture?.(down.pointerId); } catch {}
    gestures?.onDragEnd?.();
  };

  const onPointerUp = (event: PointerEvent) => {
    const down = pointerDown;
    if (!down) return;
    if (down.dragging) {
      pointerDown = null;
      endDrag(down);
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
    if (!down.resolved) {
      down.pendingUp = true;
      return;
    }
    pointerDown = null;
    if (!down.key) return;
    dispatchActivate(down.key, down.toggle, down);
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
    const key = inspectedKeyRef.current
      ?? (focusInsideRef.current && accessibleRef.current?.contains(event.target as Node) ? focusedKeyRef.current : null);
    const hit = inspectedKeyRef.current !== null ? (inspectedHitRef?.current ?? null) : null;
    if (onContextMenu(key, event.clientX, event.clientY, hit)) {
      event.preventDefault();
    }
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
