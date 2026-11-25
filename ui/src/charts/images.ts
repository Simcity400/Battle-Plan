import { sharedDomArtCache } from '../families/art/domCache.ts';

export function canvasImageRead(url: string, loaded: () => void): { image: HTMLImageElement | null; pending: boolean; failed: boolean } {
  const result = sharedDomArtCache.readUrl(url, loaded);
  return { image: result.image, pending: result.pending, failed: result.failed };
}
