let watched: number | null = null;
let sent = 0;

export function watchedDocument(document: number): void {
  watched = document;
}

export function nextInstallAttempt(): { document: number | null; attempt: number } {
  sent += 1;
  return { document: watched, attempt: sent };
}

export function lastInstallAttempt(): number {
  return sent;
}
