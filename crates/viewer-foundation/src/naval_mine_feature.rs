use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

pub const FEATURE: u32 = 1 << 4;

pub fn enabled(frame: &DecodedFrame) -> Result<bool, String> {
    match frame.complete::<tables::SimulationFeatures>() {
        Ok([features]) => Ok(features.dlc_mask & FEATURE != 0),
        Ok(_) => Err("naval mines DLC feature: the DLC mask needs one SimulationFeatures row".into()),
        Err(why) => Err(why.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{SimulationFeatureRow, TableId, TableRows};
    use replay_frame::frame::{Metrics, TableView};

    fn frame(rows: Vec<SimulationFeatureRow>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 100,
            tables: std::array::from_fn(|i| TableView::Complete(TableRows::empty(TableId::ALL[i])).into()),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(rows));
        frame
    }
    fn mask(dlc_mask: u32) -> DecodedFrame {
        frame(vec![SimulationFeatureRow {
            dlc_mask,
            ..Default::default()
        }])
    }

    #[test]
    fn mines_follow_the_man_the_guns_runtime_bit_not_the_feature_id() {
        assert_eq!(enabled(&mask(0x10)), Ok(true));
        assert_eq!(enabled(&mask(0x1_ffef)), Ok(false));
        assert_eq!(enabled(&mask(1 << 19)), Ok(false));
        assert!(enabled(&frame(Vec::new())).is_err());
    }
}
