use collector_memory::terminal::*;
#[cfg(test)]
use std::io;

#[cfg(test)]
pub(crate) fn concurrent_watermark_exercise() -> io::Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    install_hooks();
    for _ in 0..64 {
        reset();
        clear_handle();
        let started = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let writer_started = Arc::clone(&started);
        let writer_stop = Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            for segment in 1..100_000u32 {
                set_context(Context {
                    campaign_id: Some(*b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6"),
                    branch: Some(u64::from(segment)),
                    load_signal: Some(i64::from(segment)),
                    failed_hour: None,
                });
                committed(segment, u64::from(segment) * 37);
                set_context(Context {
                    campaign_id: Some(*b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6"),
                    ..Context::default()
                });
                record_start(Some((segment, u64::from(segment) * 17)));
                writer_started.store(true, Ordering::Release);
                if writer_stop.load(Ordering::Acquire) {
                    break;
                }
            }
        });
        while !started.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        fail(Phase::Status, Code::Reserve);
        let first = failure().ok_or(io::ErrorKind::Other)?;
        stop.store(true, Ordering::Release);
        writer
            .join()
            .map_err(|_| io::Error::from(io::ErrorKind::Other))?;
        let segment = first.committed_segment.ok_or(io::ErrorKind::Other)?;
        if first.committed_bytes != u64::from(segment) * 37
            || first.committed_campaign_id != Some(*b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6")
            || first.branch != Some(u64::from(segment))
            || first.load_signal != Some(i64::from(segment))
            || first.failed_hour.is_some()
        {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if let Some(pending) = first.pending_campaign_id {
            if pending != *b"22b7cc84-3028-4f65-8e0d-adf2f37ca1a6"
                || first.pending_segment != Some(segment)
                || first.pending_bytes != u64::from(segment) * 17
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
        } else if first.pending_segment.is_some() || first.pending_bytes != 0 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        fail(Phase::Teardown, Code::Thread);
        if failure() != Some(first) {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
    reset();
    Ok(())
}
