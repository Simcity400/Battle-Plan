import { useEffect, useMemo } from 'react';

const NO_WARNINGS: string[] = [];

export function useDiagnostics(onDiagnostics: (warnings: string[]) => void, warnings: readonly (string | Error)[] | null) {
  const text = useMemo(() => warnings && JSON.stringify(warnings.map(warning => warning instanceof Error ? warning.message : warning)), [warnings]);
  useEffect(() => {
    if (text === null) return;
    const sent: string[] = JSON.parse(text);
    onDiagnostics(sent.length ? sent : NO_WARNINGS);
  }, [text, onDiagnostics]);
  useEffect(() => () => onDiagnostics(NO_WARNINGS), [onDiagnostics]);
}
