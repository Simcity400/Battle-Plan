export default function ListSkeleton({ label, rows = 4, art = true }: { label: string; rows?: number; art?: boolean }) {
  return <div className="list-skeleton" role="status" aria-label={label}>
    {Array.from({ length: rows }, (_, index) => <div className="list-skeleton-row" key={index} aria-hidden="true">
      {art && <span className="skeleton list-skeleton-art" />}
      <span className="list-skeleton-text"><span className="skeleton" style={{ width: `${62 - index * 7}%` }} /><span className="skeleton" style={{ width: `${34 + index * 5}%` }} /></span>
    </div>)}
  </div>;
}
