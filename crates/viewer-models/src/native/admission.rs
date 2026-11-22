use crate::native::Queue;

#[derive(Default)]
pub(super) struct Admission {
    meshes: Vec<Option<[bool; 2]>>,
    #[cfg(test)]
    made: usize,
}

impl Admission {
    pub(super) fn of(
        &mut self,
        mesh: usize,
        nodes: impl IntoIterator<Item = usize>,
        flags: &[Option<[bool; 2]>],
    ) -> Option<[bool; 2]> {
        if let Some(Some(known)) = self.meshes.get(mesh) {
            return Some(*known);
        }
        let mut any = [false; 2];
        for node in nodes {
            let [regular, alternate] = (*flags.get(node)?)?;
            any[0] |= regular;
            any[1] |= alternate;
        }
        if self.meshes.len() <= mesh {
            self.meshes.resize(mesh + 1, None);
        }
        self.meshes[mesh] = Some(any);
        #[cfg(test)]
        {
            self.made += 1;
        }
        Some(any)
    }

    #[cfg(test)]
    pub(super) fn made(&self) -> usize {
        self.made
    }
}

pub(super) fn passes_over(queue: Queue, forced: bool, admits: [bool; 2]) -> bool {
    let regular = admits[0] && !forced;
    let alternate = admits[1] || forced;
    match queue {
        Queue::Regular => !regular && alternate,
        Queue::Alternate => !alternate && regular,
        Queue::Caster => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::native::admission::*;

    #[test]
    fn a_mesh_is_summed_once_its_nodes_are_known_and_never_again() {
        let mut admission = Admission::default();
        let mut flags = vec![Some([true, false]), None];
        assert_eq!(admission.of(3, [0, 1], &flags), None);
        assert_eq!(admission.of(3, [0, 1], &flags), None);
        assert_eq!(admission.made(), 0);
        flags[1] = Some([false, true]);
        assert_eq!(admission.of(3, [0, 1], &flags), Some([true, true]));
        assert_eq!(admission.made(), 1);
        for _ in 0..4 {
            assert_eq!(admission.of(3, [0, 1], &flags), Some([true, true]));
        }
        assert_eq!(admission.made(), 1, "a second frame sums nothing again");
        assert_eq!(admission.of(0, [0], &flags), Some([true, false]));
        assert_eq!(admission.made(), 2, "another mesh is summed on its own");
    }

    #[test]
    fn a_node_outside_the_known_flags_leaves_the_mesh_unknown() {
        let mut admission = Admission::default();
        assert_eq!(admission.of(0, [2], &[Some([true, false])]), None);
        assert_eq!(admission.made(), 0);
    }

    #[test]
    fn the_summary_equals_the_walk_of_each_node() {
        let all = [[false, false], [true, false], [false, true], [true, true]];
        for a in all {
            for b in all {
                let flags = [Some(a), Some(b)];
                let made = Admission::default().of(0, [0, 1], &flags).unwrap();
                for queue in [Queue::Regular, Queue::Alternate, Queue::Caster] {
                    for forced in [false, true] {
                        let node_draws = |f: [bool; 2]| match queue {
                            Queue::Regular => f[0] && !forced,
                            Queue::Alternate => f[1] || forced,
                            Queue::Caster => f[0],
                        };
                        let draws_here = node_draws(a) || node_draws(b);
                        if passes_over(queue, forced, made) {
                            assert!(
                                !draws_here,
                                "a walk passes over only what it draws nothing of"
                            );
                            let other = if queue == Queue::Regular {
                                Queue::Alternate
                            } else {
                                Queue::Regular
                            };
                            let other_draws = |f: [bool; 2]| match other {
                                Queue::Regular => f[0] && !forced,
                                _ => f[1] || forced,
                            };
                            assert!(
                                other_draws(a) || other_draws(b),
                                "and only what the other lit walk draws"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn casters_and_meshes_no_lit_walk_draws_are_never_passed_over() {
        for forced in [false, true] {
            for admits in [[false, false], [true, false], [false, true], [true, true]] {
                assert!(!passes_over(Queue::Caster, forced, admits));
            }
        }
        assert!(!passes_over(Queue::Regular, false, [false, false]));
        assert!(!passes_over(Queue::Alternate, false, [false, false]));
        assert!(
            passes_over(Queue::Regular, true, [true, false]),
            "forced trains draw only in the alternate walk"
        );
        assert!(!passes_over(Queue::Alternate, true, [true, false]));
    }
}
