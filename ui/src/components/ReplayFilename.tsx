export default function ReplayFilename({ path, className = '' }: { path: string; className?: string }) {
  return <span className={`replay-filename ${className}`} tabIndex={0} data-tooltip={path}>
    {path.replaceAll('\\', '/').split('/').pop()}
  </span>;
}
