use super::{DecodedFrame, TABLE_COUNT, TableView};
use crate::{Hour, codec::invalid_static as invalid};
use std::io;

impl DecodedFrame {
    pub fn restore_samples(&mut self, samples: [Option<Hour>; TABLE_COUNT]) -> io::Result<()> {
        for (view, sample) in self.tables.iter().zip(samples) {
            if sample
                .is_some_and(|hour| hour > self.hour || !matches!(&**view, TableView::Complete(_)))
            {
                return Err(invalid("invalid checkpoint sample hour"));
            }
        }
        for (view, sample) in self.tables.iter_mut().zip(samples) {
            view.1 = sample;
        }
        Ok(())
    }
}
