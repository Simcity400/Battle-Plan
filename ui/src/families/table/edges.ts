export interface TableEdgeViewport {
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
}

export function syncTableEdges(element: HTMLElement | null, viewport: TableEdgeViewport, extentHeight: number): void {
  if (!element) return;
  const width = `${Math.max(0, viewport.width)}px`;
  const height = `${Math.max(0, viewport.height)}px`;
  if (element.style.getPropertyValue('--tf-view-w') !== width) element.style.setProperty('--tf-view-w', width);
  if (element.style.getPropertyValue('--tf-view-h') !== height) element.style.setProperty('--tf-view-h', height);
  const below = extentHeight > 0 && viewport.scrollTop + viewport.height < extentHeight - 1;
  if ((element.dataset.bottom === 'true') !== below) {
    if (below) element.dataset.bottom = 'true';
    else delete element.dataset.bottom;
  }
}
