import { Skeleton } from "@/components/ui/States";

/** The commit and run pages while their reads are in flight: the same blocks, in outline. */
export function DetailSkeleton({ what }: { what: string }) {
  return (
    <div role="status" aria-busy="true" aria-label={`Loading ${what}`}>
      <header className="page page-head">
        <Skeleton width={240} height={13} />
        <Skeleton width={160} height={12} />
        <Skeleton width="min(380px, 80%)" height={40} />
        <Skeleton width="min(620px, 95%)" height={16} />
        <div className="row" style={{ ["--gap" as string]: "6px" }}>
          <Skeleton width={84} height={22} />
          <Skeleton width={96} height={22} />
          <Skeleton width={110} height={22} />
        </div>
      </header>
      <div className="page">
        <div className="card stack" style={{ ["--gap" as string]: "12px" }}>
          <Skeleton width="45%" height={14} />
          <Skeleton width="70%" />
          <Skeleton width="60%" />
          <Skeleton width="80%" />
          <Skeleton width="50%" />
        </div>
      </div>
      <section className="page block">
        <div className="stack" style={{ ["--gap" as string]: "14px" }}>
          <Skeleton width={200} height={24} />
          {Array.from({ length: 5 }, (_, i) => (
            <div key={i} className="row" style={{ ["--gap" as string]: "12px", flexWrap: "nowrap" }}>
              <Skeleton width={56} height={14} />
              <Skeleton width={12} height={12} />
              <Skeleton width={`${70 - i * 6}%`} height={14} />
            </div>
          ))}
        </div>
      </section>
      <span className="visually-hidden">Loading {what}…</span>
    </div>
  );
}
