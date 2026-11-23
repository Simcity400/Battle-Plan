# Hand status off and finish writes against one encoder baseline

Decode publication, roster changes, and a stop request wake the spool. The spool prepares live clocks only between game ticks. The loader owns and joins the status writer. The spool hands over status and content snapshots without waiting on status I/O. A status snapshot borrows `Shared` only while the loader retains it and joins the spool.

Early callbacks without a readable attributable clock accumulate until the recorder can write them beside the corresponding session identity. While campaign setup retries, retain its frame and earlier witnesses. A refusal tied to immutable identity or history cannot recover by retrying. End the spool so the loader requests restoration and publishes the precise failure.

Conversion owns all inputs needed by encoding and diagnostics. Return the capture storage before potentially slow output when ownership allows it. If retry publication still needs the frame, retain it until that publication completes. A conversion failure before a write has a known unpublished batch. An interrupted write instead needs the spool’s uncertain-write handling. Complete prepared writes before starting the next frame or flush so comparison tokens still name the current encoder baseline. Bound the time spent consuming ready frames so a sustained backlog cannot starve sizing, flushing or status.

For a status-file replacement on Windows, prefer POSIX rename semantics so a reader with delete sharing may retain the old file while the new name takes effect. Use classic replacement when the status volume does not support POSIX rename. A classic replacement may be refused while a reader holds the old file. Keep the unsupported result for the attachment to avoid repeated probes.

The replacement regression uses canonical extended-length paths and checks both delete-sharing modes, including bounded retry when a reader denies deletion. Construct the temporary directory guard before opening readers so assertion failures clean up the test files. Status-snapshot tests change the source between measurement and serialization to confirm they use one owned observation.

[Spool runtime and status replacement](../../crates/collector-runtime/src/runtime.rs)
