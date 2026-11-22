use collector_status::status::{DebugValue, DisplayValue, status};
use serde::Serialize;

#[cfg(test)]
pub(crate) fn allocation_failure_exercise() -> std::io::Result<()> {
    use collector_test_support::with_allocations_denied;
    use std::io::{self, Write};

    #[derive(Clone, Copy)]
    struct Label;
    impl std::fmt::Display for Label {
        fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(out, "borrowed \"label\"\np {}", 42)
        }
    }
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn prove(value: &impl Serialize) -> io::Result<()> {
        let mut count = Count(0);
        with_allocations_denied(|| serde_json::to_writer(&mut count, value))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let mut bytes = Vec::new();
        engine_allocation::reserve_exact(&mut bytes, count.0)?;
        with_allocations_denied(|| serde_json::to_writer(&mut bytes, value))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        assert_eq!(bytes.len(), count.0);
        let _: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        Ok(())
    }
    let value = with_allocations_denied(|| {
        status!({
            "debug": DebugValue(replay_data::rows::TableId::Clock),
            "display": DisplayValue(Label),
            "destruction": collector_equipment_destruction::equipment_destruction::status(),
        })
    });
    prove(&value)?;
    drop(value);
    #[cfg(feature = "advisor_profile")]
    prove(&with_allocations_denied(
        collector_characters::extended::characters::advisors::profile::status,
    ))?;
    #[cfg(feature = "callback_profile")]
    prove(&with_allocations_denied(
        collector_workers::callback_profile::status,
    ))?;
    #[cfg(feature = "worker_sampler")]
    prove(&with_allocations_denied(
        collector_workers::worker_sampler::status,
    ))?;
    crate::runtime::status_allocation_failure_exercise()?;
    Ok(())
}
