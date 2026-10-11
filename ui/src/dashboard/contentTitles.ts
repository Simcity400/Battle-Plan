/** Display content titles without promoting an unresolved lookup key to a name.
 * Empty localized text is valid. Stored literal names use their own name paths. */
export function contentTitle(title: string | null | undefined, _key: string | number | null | undefined,
  unavailable = 'Name unavailable', pending = false): string {
  return title ?? (pending ? 'Loading name…' : unavailable);
}

export function contentTitleHint(key: string | number | null | undefined, reason?: string | null): string | undefined {
  return [key != null && key !== '' ? `Key: ${key}` : undefined, reason].filter(Boolean).join('\n') || undefined;
}
