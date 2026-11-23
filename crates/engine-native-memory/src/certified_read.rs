
pub struct Certified<T, C> {
    value: T,
    proof: Proof<C>,
}

enum Proof<C> {
    Vacant,
    Pending(C),
    Current(C),
}

pub enum Decision<'a, T> {
    Carried(&'a T),
    Read,
}

impl<T, C: PartialEq> Certified<T, C> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            proof: Proof::Vacant,
        }
    }

    pub fn begin(&mut self, signal: impl FnOnce(&T) -> Option<C>) -> Decision<'_, T> {
        let before = signal(&self.value);
        if let (Proof::Current(current), Some(before)) = (&self.proof, &before)
            && current == before
        {
            return Decision::Carried(&self.value);
        }
        self.proof = before.map_or(Proof::Vacant, Proof::Pending);
        Decision::Read
    }

    pub fn finish<E>(
        &mut self,
        signal: impl FnOnce(&T) -> Option<C>,
        read: impl FnOnce(&mut T) -> Result<(), E>,
    ) -> Result<&T, E> {
        let before = match std::mem::replace(&mut self.proof, Proof::Vacant) {
            Proof::Pending(before) => Some(before),
            _ => None,
        };
        read(&mut self.value)?;
        if before.is_some() && signal(&self.value) == before {
            self.proof = Proof::Current(before.unwrap());
        }
        Ok(&self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(value: &mut Certified<u32, u32>, before: Option<u32>, after: Option<u32>) {
        assert!(matches!(value.begin(|_| before), Decision::Read));
        value
            .finish(
                |_| after,
                |output| {
                    *output += 1;
                    Ok::<(), ()>(())
                },
            )
            .unwrap();
    }

    #[test]
    fn only_complete_matching_proof_carries_and_missing_signal_revokes_it() {
        let mut value = Certified::new(0);
        read(&mut value, None, Some(1));
        read(&mut value, Some(1), Some(2));
        read(&mut value, Some(2), Some(2));
        assert!(matches!(value.begin(|_| Some(2)), Decision::Carried(&3)));
        read(&mut value, Some(3), Some(3));
        assert!(matches!(value.begin(|_| Some(3)), Decision::Carried(&4)));
        read(&mut value, None, None);
        assert!(matches!(value.begin(|_| None), Decision::Read));
    }

    #[test]
    fn failed_or_abandoned_reads_never_certify_partially_written_values() {
        let mut value = Certified::new(0);
        read(&mut value, Some(1), Some(1));
        assert!(matches!(value.begin(|_| Some(2)), Decision::Read));
        assert_eq!(
            value.finish(
                |_| Some(2),
                |output| {
                    *output = 99;
                    Err::<(), _>("failed")
                }
            ),
            Err("failed")
        );
        assert!(matches!(value.begin(|_| Some(2)), Decision::Read));
        assert!(matches!(value.begin(|_| Some(2)), Decision::Read));
        value
            .finish(
                |_| Some(2),
                |output| {
                    *output = 3;
                    Ok::<(), ()>(())
                },
            )
            .unwrap();
        assert!(matches!(value.begin(|_| Some(2)), Decision::Carried(&3)));
    }
}
