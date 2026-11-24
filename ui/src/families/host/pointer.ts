import type { Pipeline, PointerHitMeta } from '../../dashboard/follow/pipeline.ts';
import type { InstalledAction, InstalledPicture, InstalledViewport } from '../plan.ts';
import type { FamilyTooltipController, FamilyTooltipField } from './tooltips.ts';

export const CLICK_CANCEL_PX = 4;

export interface FamilyPointerHit {
  readonly key: string;
  readonly toggle: boolean;
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
  readonly lastPointerRef: { current: { x: number; y: number } | null };
  readonly focusInsideRef: { readonly current: boolean };
  readonly focusedKeyRef: { readonly current: string | null };
  readonly focusAnchorRef: { readonly current: { x: number; y: number } };
  readonly accessibleRef: { readonly current: HTMLElement | null };
  readonly isBlocked: () => boolean;
  readonly getViewportHeight: () => number;
  readonly getInstalled?: () => InstalledPicture | null;
  readonly parseHit?: (hitKey: string | null) => FamilyPointerHit | null;
  readonly inspectToken?: (key: string) => string;
  readonly onActivate: (key: string, toggle: boolean, action?: InstalledAction) => void;
  readonly onContextMenu?: (key: string | null, clientX: number, clientY: number) => boolean;
}

interface PointerDownState {
  key: string | null;
  installedId: number | undefined;
  owner: string | undefined;
  resolved: boolean;
  pendingUp: boolean;
  x: number;
  y: number;
  button: number;
  toggle: boolean;
  cancelled: boolean;
}

export function bindFamilyPointer(options: FamilyPointerBindingOptions): () => void {
  const {
    bodyNode,
    menuHost,
    pipeline,
    pipe,
    tooltips,
    inspectedKeyRef,
    lastPointerRef,
    focusInsideRef,
    focusedKeyRef,
    focusAnchorRef,
    accessibleRef,
    isBlocked,
    getViewportHeight,
    getInstalled,
    onActivate,
    onContextMenu,
  } = options;
  const parseHit = options.parseHit ?? parseDefaultRowHit;
  const inspectToken = options.inspectToken ?? ((key: string) => `row:${key}`);
  let pointerDown: PointerDownState | null = null;

  const installedNow = (): InstalledPicture | null => getInstalled?.() ?? null;

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
      const action = resolveInstalledAction(cur, key, toggle);
      if (!action) return;
      onActivate(action.subject, toggle, action);
      return;
    }
    onActivate(key, toggle);
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!event.isTrusted) return;
    if (pointerDown && !pointerDown.cancelled && Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) > CLICK_CANCEL_PX) {
      pointerDown.cancelled = true;
    }
    if (isBlocked()) return;
    const last = lastPointerRef.current;
    if (last && last.x === event.clientX && last.y === event.clientY) return;
    lastPointerRef.current = { x: event.clientX, y: event.clientY };
    tooltips.cancelHoverLeave();
    pipeline.pointer(pipe, 0, toPoint(event.clientX, event.clientY), (hitKey, hitMeta) => {
      if (isBlocked() || !matchesInstalled(hitMeta)) return;
      const parsed = parseHit(hitKey);
      const nextKey = parsed?.key ?? null;
      const changed = nextKey !== inspectedKeyRef.current;
      inspectedKeyRef.current = nextKey;
      pipeline.inspect(pipe, nextKey ? inspectToken(nextKey) : null);
      if (!nextKey) {
        tooltips.cancelHoverOpen();
        if (!focusInsideRef.current) tooltips.hideIfUnpinned();
        return;
      }
      if (changed) {
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
      key: inspectedKeyRef.current,
      installedId: snap?.id,
      owner: snap?.owner,
      resolved: !!inspectedKeyRef.current,
      pendingUp: false,
      x,
      y,
      button: event.button,
      toggle: false,
      cancelled: false,
    };
    pointerDown = down;
    pipeline.pointer(pipe, 0, point, (hitKey, hitMeta) => {
      if (down.cancelled) return;
      if (!matchesInstalled(hitMeta, down)) {
        down.key = null;
        down.resolved = true;
        if (pointerDown === down && down.pendingUp) pointerDown = null;
        return;
      }
      const parsed = parseHit(hitKey);
      const rowKey = parsed?.key ?? null;
      const isToggle = parsed?.toggle === true;
      down.key = rowKey;
      down.toggle = isToggle;
      down.resolved = true;
      if (rowKey) {
        inspectedKeyRef.current = rowKey;
        pipeline.inspect(pipe, inspectToken(rowKey));
      }
      if (down.pendingUp) {
        if (pointerDown === down) pointerDown = null;
        if (!isBlocked() && down.button === 0 && rowKey) {
          dispatchActivate(rowKey, isToggle, down);
        }
      }
    });
  };

  const onPointerUp = (event: PointerEvent) => {
    const down = pointerDown;
    if (!down || down.cancelled || down.button !== 0 || isBlocked()) {
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
    if (pointerDown) pointerDown.cancelled = true;
    pointerDown = null;
  };

  const handleContextMenu = (event: MouseEvent) => {
    if (isBlocked() || !onContextMenu) return;
    const key = inspectedKeyRef.current
      ?? (focusInsideRef.current && accessibleRef.current?.contains(event.target as Node) ? focusedKeyRef.current : null);
    if (onContextMenu(key, event.clientX, event.clientY)) {
      event.preventDefault();
    }
  };

  bodyNode.addEventListener('pointermove', onPointerMove);
  bodyNode.addEventListener('pointerleave', onPointerLeave);
  bodyNode.addEventListener('pointerdown', onPointerDown);
  bodyNode.addEventListener('pointerup', onPointerUp);
  bodyNode.addEventListener('pointercancel', onPointerCancel);
  menuHost.addEventListener('contextmenu', handleContextMenu);

  return () => {
    pointerDown = null;
    tooltips.cancelTimers();
    lastPointerRef.current = null;
    if (inspectedKeyRef.current !== null) {
      inspectedKeyRef.current = null;
      pipeline.pointer(pipe, 0, null);
      pipeline.inspect(pipe, null);
    }
    tooltips.hideIfUnpinned();
    bodyNode.removeEventListener('pointermove', onPointerMove);
    bodyNode.removeEventListener('pointerleave', onPointerLeave);
    bodyNode.removeEventListener('pointerdown', onPointerDown);
    bodyNode.removeEventListener('pointerup', onPointerUp);
    bodyNode.removeEventListener('pointercancel', onPointerCancel);
    menuHost.removeEventListener('contextmenu', handleContextMenu);
  };
}
