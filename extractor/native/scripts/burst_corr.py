"""Round 3 (correct layout). frame_end payload:
  [0:8]  elapsed_wall_ms (u64)   <- wall clock since capture start
  [8:16] frame_time_us  (u64)    <- duration of THAT sweep
  [16:20] emitted_domains, [20:24] scheduled_domain_count, [24:28] status_flags

Rounds 1-2 misread elapsed_wall_ms as frame_time_us (monotone counter -> autocorr 1.0).
All numbers from those rounds are void.

Now we have real wall time, so the game's pace is measurable INDEPENDENTLY of our cost:
  wall_delta_i = wall_i - wall_{i-1}   (ms of real time that frame i's cycle took)
  gap_i        = game hours advanced across that cycle
  interval_i   = wall_delta_i / gap_i  (ms of real time per game-hour: the GAME's pace)
  ft_i         = frame_time_us_i/1000  (ms our sweep spent)

H1 (we cause bursts): a heavy frame i-1 should be followed by a FAST game window
   => corr(ft[i-1], interval[i]) < 0
H0/common-cause: our cost and the game's per-hour cost both track world workload
   => corr(ft[i], interval[i]) > 0 with no special role for the lag.

Also: what fraction of each cycle is OUR sweep?  ft_i / wall_delta_i
"""
import struct, sys

STREAM = r"extractor\native\runs\live_validation_20260709_221706\stage2_shortspeed5\collector_stream.bin"
HDR, FRAME_END = 32, 2
rec = []
with open(STREAM, "rb") as f:
    while True:
        h = f.read(HDR)
        if len(h) < HDR or h[0:4] != b"H4CR":
            break
        ghr = struct.unpack_from("<i", h, 16)[0]
        rt  = struct.unpack_from("<H", h, 22)[0]
        pl  = struct.unpack_from("<I", h, 24)[0]
        if rt == FRAME_END:
            p = f.read(pl)
            wall_ms = struct.unpack_from("<Q", p, 0)[0]
            ft_us   = struct.unpack_from("<Q", p, 8)[0]
            rec.append((ghr, wall_ms, ft_us))
        else:
            f.seek(pl, 1)

def mean(x):
    x=list(x); return sum(x)/len(x) if x else float('nan')
def pear(xs,ys):
    mx,my=mean(xs),mean(ys)
    num=sum((a-mx)*(b-my) for a,b in zip(xs,ys))
    dx=sum((a-mx)**2 for a in xs)**.5; dy=sum((b-my)**2 for b in ys)**.5
    return num/(dx*dy) if dx and dy else float('nan')
def pct(xs,q):
    s=sorted(xs); return s[min(len(s)-1,max(0,int(q*len(s))))]

rows=[]
for i in range(1,len(rec)):
    gh0,w0,_ = rec[i-1]; gh1,w1,ft1 = rec[i]
    gap = gh1-gh0; wd = w1-w0
    if gap<=0 or wd<=0: continue
    rows.append(dict(gap=gap, wall=wd, ft=ft1/1000.0, prev_ft=rec[i-1][2]/1000.0,
                     interval=wd/gap))

print(f"frame_end={len(rec)}  usable={len(rows)}  skipped_hours={sum(r['gap']-1 for r in rows)}")
ft=[r['ft'] for r in rows]; pf=[r['prev_ft'] for r in rows]
iv=[r['interval'] for r in rows]; wd=[r['wall'] for r in rows]; gp=[r['gap'] for r in rows]

print()
print("=== sanity (these must now look like real durations) ===")
print(f"  frame_time ms : mean={mean(ft):.1f} p50={pct(ft,.5):.1f} p95={pct(ft,.95):.1f} max={max(ft):.1f}")
print(f"  autocorr(ft,1)= {pear(ft,pf):+.4f}   (should be well under 1.0 now)")
print(f"  wall_delta ms : mean={mean(wd):.1f} p50={pct(wd,.5):.1f}")
print(f"  our share of the cycle (ft/wall): mean={mean([a/b for a,b in zip(ft,wd)]):.3f}")

print()
print("=== GAME PACE (independent of our frame cost) ===")
print(f"  interval ms/game-hour: min={min(iv):.1f} p1={pct(iv,.01):.1f} p5={pct(iv,.05):.1f} "
      f"p50={pct(iv,.5):.1f} mean={mean(iv):.1f}")

print()
print("=== H1: do WE cause the bursts? ===")
print(f"  corr(prev frame ms, interval of next cycle) = {pear(pf,iv):+.4f}")
print("     NEGATIVE supports H1 (heavy frame -> game then bursts)")
print(f"  corr(cur  frame ms, interval of same cycle) = {pear(ft,iv):+.4f}")
print("     POSITIVE supports common-cause (world workload drives both)")
print(f"  corr(gap, prev frame ms) = {pear(gp,pf):+.4f}   corr(gap, cur frame ms) = {pear(gp,ft):+.4f}")

print()
print("=== conditional means ===")
sk=[r for r in rows if r['gap']>1]; ok=[r for r in rows if r['gap']==1]
print(f"  SKIPPED (gap>1) n={len(sk)}: prev_ft={mean(r['prev_ft'] for r in sk):7.1f} ms  interval={mean(r['interval'] for r in sk):7.1f} ms")
print(f"  CLEAN   (gap==1) n={len(ok)}: prev_ft={mean(r['prev_ft'] for r in ok):7.1f} ms  interval={mean(r['interval'] for r in ok):7.1f} ms")

print()
print("=== ACQUISITION BUDGET: if the per-hour cycle cost D ms, how many hours are skipped? ===")
print("   (holds the game's observed pace fixed; each cycle of D ms lets D/interval hours pass)")
print(f"   {'D (ms)':>8} {'skipped':>10} {'skip rate':>10}")
total=len(rows)
for D in [2,5,10,15,20,25,28,40,60,100,151]:
    sk_h=sum(max(0.0, D/r['interval'] - 1.0) for r in rows)
    print(f"   {D:>8} {sk_h:>10.0f} {sk_h/(sk_h+total):>9.1%}")
print()
print(f"   observed sweep mean {mean(ft):.1f} ms produced {sum(r['gap']-1 for r in rows)} real skips "
      f"({sum(r['gap']-1 for r in rows)/sum(r['gap'] for r in rows):.1%} of advanced hours)")
