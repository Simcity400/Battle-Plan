export const BROWSER_ARTWORK_BASE = '/api/artwork/';
export const artworkName = (value: string): boolean => /^[0-9a-f]{32}\.png$/.test(value);
export const artworkUrlValue = (value: unknown): value is string => typeof value === 'string'
  && /^(?:http:\/\/artwork\.localhost|\/api\/artwork)\/[0-9a-f]{32}\.png$/.test(value);

