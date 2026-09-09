# Isolated Windows DLL evidence

This standalone tool is separate from the HOI4 broker, registry, index, and projects.
It pins only the two AMD64 Windows DLL files in MODULES, version 10.0.26100.9168,
observed on OS build 26200. These identities do not establish another OS's behavior.

Run python -m unittest discover -s replay_interface/tools/windows_dll_evidence -v.
The tests use the pinned installed KERNEL32 file and temporary C: files.

The default runtime root is the dedicated external-dll-capability/isolated scratch
directory declared in the wrapper. It contains its own ownership marker, exclusive
job lease, snapshots, per-module projects, JVM profile/temp files, and job bundles.
The wrapper reuses the broker's Windows child-process ownership helper; it does
not load the broker server or change its registration.

From the repository root, invoke external_module_evidence.py with:
- prepare kernel32
- prepare kernelbase
- probe kernel32
- functions kernel32 --rva 0x414f0 --rva 0x303b0 --rva 0x36f30
- verify kernel32 --job C:/absolute/owned/root/jobs/job-id
- imports kernel32 --job C:/absolute/owned/root/jobs/three-entry-function-job

Prepare performs an isolated import/analysis and metadata-only identity probe.
Other operations open that project with -readOnly -noanalysis. Every script run
checks disk snapshot name/hash/size, imported name/path/hash/base/language, and
original plus modified imported FileBytes before emitting anything. Exported
instruction/data bytes must also match file-backed PE ranges. A missing complete
manifest, missing/extra artifact, failed decompile, identity mismatch, or changed
byte refuses the bundle.

Function requests support one to three exact entry RVAs and depth zero only:
no caller/callee expansion. Full function bodies are bounded to 64 KiB and
8192 instructions; decompilation has a 120 second deadline. Data requests support
one to four explicit cells, each at most 512 bytes. Data scope must follow native
producer references; this command is not a table-census or broad dump tool.

Each job retains the JSON request, actual Java command, tool/script/wrapper hashes,
console and script logs, exit code, artifact manifest and independent byte-verification
receipt. Partial jobs stay visibly incomplete. The tool never launches/attaches HOI4
or invokes the three target APIs. Current-process import-slot metadata and native
entry-to-slot control flow are separate claims: a code export is not a PE forwarder.
The imports command checks both DLL paths/headers in the same helper process and
compares the exported KERNEL32 entry instruction bytes with loaded memory, then
records only the three named import slots and their exact KernelBase destinations.

Each Java process starts suspended, enters its own Windows Job, and resumes only
after ownership is established. The external root lease remains held until the
Job reports zero active processes, on success, failure, or the 900 second execution
timeout. Descendants surviving the launcher's bounded natural drain make the job
fail and are terminated. Cleanup/query errors retain the lease and retry, so the
execution timeout is not a hard wall-clock limit on cleanup. Only owned processes
are affected. The retained helper copy and process receipt are part of new-job
custody. Historical jobs remain verifiable with their original generating wrapper;
their byte evidence does not acquire a descendant-cleanup claim from this change.

Independent review of the final source and raw validation is required before relying
on generated semantics. Byte custody alone does not prove any ctype or locale policy.
