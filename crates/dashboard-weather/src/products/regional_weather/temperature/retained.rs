use super::Observation;
use dashboard_keyed::{Delta, Values};
use replay_data::temperature::Input;
use std::io;

impl Observation {
    fn refresh_input(
        &mut self,
        branch: u64,
        hour: i32,
        input: Result<&Input, &str>,
        publish: impl FnMut(&str, Option<i128>),
    ) -> io::Result<bool> {
        self.refresh_with(
            branch,
            hour,
            |memo, content| {
                let input = input.map_err(str::to_owned)?;
                let states = memo.states_input(branch, hour, input, Ok(&content.pass))?;
                if states.len() != content.pass.provinces.len() {
                    return Err(format!(
                        "content has {} weather province slots, the recording {}",
                        content.pass.provinces.len(),
                        states.len()
                    ));
                }
                Ok(())
            },
            publish,
        )
    }

    pub(in super::super) fn observe_input(
        &mut self,
        branch: u64,
        hour: i32,
        input: Result<&Input, &str>,
        values: &mut Values,
    ) -> io::Result<bool> {
        let known = self.refresh_input(branch, hour, input, |_, _| {})?;
        if known {
            for region in self.aggregates.as_ref().unwrap().regions.values() {
                if let Some(mean) = region.mean {
                    values.push(region.name.clone(), mean);
                }
            }
        }
        Ok(known)
    }

    pub(in super::super) fn observe_input_changes(
        &mut self,
        branch: u64,
        hour: i32,
        input: Result<&Input, &str>,
        delta: &mut Delta,
    ) -> io::Result<bool> {
        self.refresh_input(branch, hour, input, |name, value| match value {
            Some(value) => delta.set(name.to_owned(), value),
            None => delta.remove(name.to_owned()),
        })
    }
}
