import type { Pipeline } from '../../dashboard/follow/pipeline.ts';
import type { FamilyTooltipController, FamilyTooltipField } from './tooltips.ts';

export const CLICK_CANCEL_PX = 4;

export interface FamilyPointerHit {
  readonly key: string;
  readonly toggle: boolean;
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
  readonly parseHit?: (hitKey: string | null) => FamilyPointerHit | null;
  readonly inspectToken?: (key: string) => string;
  readonly onActivate: (key: string, toggle: boolean) => void;
  readonly onContextMenu?: (key: string | null, clientX: number, clientY: number) => boolean;
}

interface PointerDownState {
  key: string | null;
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
    onActivate,
    onContextMenu,
  } = options;
  const parseHit = options.parseHit ?? parseDefaultRowHit;
  const inspectToken = options.inspectToken ?? ((key: string) => `row:${key}`);
  let pointerDown: PointerDownState | null = null;

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
    const box = bodyNode.getBoundingClientRect();
    pipeline.pointer(pipe, 0, [event.clientX - box.left, event.clientY - box.top], hitKey => {
      if (isBlocked()) return;
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
    const box = bodyNode.getBoundingClientRect();
    const x = event.clientX, y = event.clientY;
    if (inspectedKeyRef.current) {
      const down: PointerDownState = { key: inspectedKeyRef.current, resolved: true, pendingUp: false, x, y, button: event.button, toggle: false, cancelled: false };
      pointerDown = down;
      pipeline.pointer(pipe, 0, [x - box.left, y - box.top], hitKey => {
        if (pointerDown === down && parseHit(hitKey)?.toggle) down.toggle = true;
      });
    } else {
      const down: PointerDownState = { key: null, resolved: false, pendingUp: false, x, y, button: event.button, toggle: false, cancelled: false };
      pointerDown = down;
      pipeline.pointer(pipe, 0, [x - box.left, y - box.top], hitKey => {
        if (down.cancelled) return;
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
            onActivate(rowKey, isToggle);
          }
        }
      });
    }
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
    onActivate(down.key, down.toggle);
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
    bodyNode.removeEventListener('pointermove', onPointerMove);
    bodyNode.removeEventListener('pointerleave', onPointerLeave);
    bodyNode.removeEventListener('pointerdown', onPointerDown);
    bodyNode.removeEventListener('pointerup', onPointerUp);
    bodyNode.removeEventListener('pointercancel', onPointerCancel);
    menuHost.removeEventListener('contextmenu', handleContextMenu);
  };
}
