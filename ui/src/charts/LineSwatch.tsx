export default function LineSwatch({ dash, color }: { dash: readonly number[]; color?: string }) {
  return <svg className="chart-line-swatch" width="16" height="2" viewBox="0 0 16 2" aria-hidden="true">
    <line x1="0" y1="1" x2="16" y2="1" stroke={color ?? 'currentColor'} strokeWidth="2" strokeDasharray={dash.length ? dash.join(' ') : undefined} />
  </svg>;
}
