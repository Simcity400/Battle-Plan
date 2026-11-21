export function isMapOverlay(target: EventTarget | null): boolean {
  return target instanceof Element && target.hasAttribute('popover') && target.getAttribute('role') !== 'tooltip';
}

export function hasMapOverlay(): boolean {
  return !!document.querySelector('[popover]:popover-open:not([role="tooltip"])');
}
