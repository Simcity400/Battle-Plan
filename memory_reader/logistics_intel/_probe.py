import sys, os
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import winmem, fastscan as fs
import _supply_cluster as sc

p = winmem.open_process("hoi4.exe")
arena = fs.discover_arena(p.pid)
pid = p.pid
p.close()
print("arena %.2f GB" % ((arena[1] - arena[0]) / (1 << 30)))
probes = [
    ("rem11.83", 11.83, "f32"), ("hub6.73", 6.73, "f32"),
    ("rem9.64", 9.64, "f32"), ("rem7.48", 7.48, "f32"),
    ("ssup6.60", 6.60, "f32"),
    ("rem11.83_fp16", 11.83, "fp16"), ("hub6.73_fp16", 6.73, "fp16"),
    ("rem9.64_fp16", 9.64, "fp16"), ("ssup6.60_fp16", 6.60, "fp16"),
    ("rem11.83_fx1e5", 11.83, "fx100000"), ("ssup6.60_fx1e5", 6.60, "fx100000"),
]
for label, val, enc in probes:
    a = sc.find_all(pid, arena, val, enc)
    print("%-18s n=%-7d sample=%s" % (label, len(a), [hex(x) for x in a[:4]]))
