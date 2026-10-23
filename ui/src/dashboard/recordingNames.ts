/** A recording's name, everywhere in both apps, is its file name without the extension. */
export const recordingFilename = (path: string) => path.replaceAll('\\', '/').split('/').pop()!;

export const recordingDisplayName = (path: string) => recordingFilename(path).replace(/\.(?:battleplan|hoi4replay)$/i, '');

export function recordingNameMatches(path: string, query: string): boolean {
  return recordingFilename(path).toLowerCase().includes(query.trim().toLowerCase());
}
