use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Witnessed<T>(pub Option<T>);

impl<T> Witnessed<T> {
    pub fn get(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<T: Value> Value for Witnessed<T> {
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.0)
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;
    const HAS_STEPS: bool = T::HAS_STEPS;
    fn round_display(&mut self) {
        if let Some(value) = &mut self.0 {
            value.round_display();
        }
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        self.0.as_ref().map_or(Ok(0), Value::encoded_bytes)
    }
    fn recorded_bytes(&self) -> io::Result<usize> {
        self.0.as_ref().map_or(Ok(0), Value::recorded_bytes)
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.0.as_ref().map(Value::try_clone).transpose().map(Self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match &self.0 {
            Some(value) => value.encode_reserved(output),
            None => Ok(()),
        }
    }
    fn decode(input: &mut crate::codec::Decoder<'_>) -> io::Result<Self> {
        if input.remaining() == 0 {
            return Ok(Self(None));
        }
        T::decode(input).map(|value| Self(Some(value)))
    }
    fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
        if let Some(value) = &self.0 {
            value.visit_field_gaps(visit);
        }
    }
}

opaque_value!(Witnessed<u64>);
