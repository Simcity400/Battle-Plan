use super::*;

struct Numeric {
    source: Observed,
    values: [Result<crate::modifier_accumulator::Value, &'static str>; 2],
    queries: std::cell::RefCell<Vec<(usize, u32)>>,
}
impl Memory for Numeric {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.source.copy(address, output)
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
        self.queries.borrow_mut().push((address, key));
        [
            CANNOT_RETREAT_WHILE_DEFENDING,
            CANNOT_RETREAT_WHILE_ATTACKING,
        ]
        .iter()
        .position(|&expected| expected == key)
        .map(|index| self.values[index])
    }
}
fn numeric() -> Numeric {
    Numeric {
        source: Observed::new(Fixture::new()),
        values: [Ok(crate::modifier_accumulator::Value::default()); 2],
        queries: Default::default(),
    }
}
fn seed(group: Option<usize>) -> crate::carmy::DivisionSeed {
    crate::carmy::DivisionSeed {
        pointer: ARMY,
        orders_group: group,
        class: Some(1),
        ..Default::default()
    }
}
#[test]
fn null_commander_and_unobserved_context_remain_distinct_without_numeric_reads() {
    let source = numeric();
    assert_eq!(
        current_retreat_inputs(&source, 0, &seed(Some(0))),
        (Some(false), [Ok(0); 2])
    );
    let (context, values) = current_retreat_inputs(&source, 0, &seed(None));
    assert_eq!(context, None);
    assert!(values.into_iter().all(|value| value.is_err()));
    assert!(source.queries.borrow().is_empty());
    assert!(source.source.reads.borrow().is_empty());
    source
        .source
        .fixture
        .borrow_mut()
        .word(GROUP + group::LEADER, 0);
    assert_eq!(
        current_retreat_inputs(&source, 0, &seed(Some(GROUP))),
        (Some(false), [Ok(0); 2])
    );
    assert!(source.queries.borrow().is_empty());
}
#[test]
fn same_selector_feeds_independent_retreat_key_failures_without_retry() {
    for refused in 0..2 {
        let mut source = numeric();
        source.values[refused] = Err("independent selected retreat key fault");
        let (context, values) = current_retreat_inputs(&source, 0, &seed(Some(GROUP)));
        assert_eq!(context, Some(true));
        assert_eq!(
            values[refused],
            Err("independent selected retreat key fault")
        );
        assert_eq!(values[1 - refused], Ok(0));
        assert_eq!(
            *source.queries.borrow(),
            [
                (LEADER + 0x418, CANNOT_RETREAT_WHILE_DEFENDING),
                (LEADER + 0x418, CANNOT_RETREAT_WHILE_ATTACKING)
            ]
        );
        assert_eq!(source.source.count(GROUP + group::LEADER), 1);
        // Missing keys return native zero before modifier definition demand.
        assert_eq!(
            source
                .source
                .count(crate::modifier_definition::ELIGIBILITY_CALLBACK_RVA),
            0
        );
    }
}
