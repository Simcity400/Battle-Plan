use content_reader_parser::{
    crt::{Scanned, scan_f32, scan_x32},
    parser::Item,
};

pub const WHITE: [f32; 4] = [1.0; 4];

#[derive(Clone, Debug, PartialEq)]
pub struct ColorReads {
    color: [f32; 4],
}

impl Default for ColorReads {
    fn default() -> Self {
        Self { color: WHITE }
    }
}

impl ColorReads {
    pub fn read(&mut self, item: &Item) {
        let Some(components) = item.value.items() else {
            if item.number() {
                let text = item.value.scalar().unwrap_or_default();
                let value = match scan_x32(text) {
                    Scanned::Stored(value) => Some(value),
                    Scanned::End => Some(0),
                    Scanned::NoMatch => None,
                };
                if let Some(value) = value {
                    let [b, g, r, a] = value.to_le_bytes();
                    self.color = [r, g, b, a].map(|byte| f32::from(byte) / 255.0);
                }
            }
            return;
        };
        for (index, component) in components.iter().enumerate().take(4) {
            let text = component.value.scalar().unwrap_or_default();
            let scanned = scan_f32(text);
            if index == 3 {
                if let Some(alpha) = scanned {
                    self.color[3] = alpha;
                }
                continue;
            }
            if let Some(value) = scanned {
                self.color[index] = value;
            }
            if self.color[index] > 1.0 {
                self.color[index] /= 255.0;
            }
            if index == 2 {
                self.color[3] = 1.0;
            }
        }
    }

    pub fn color(&self) -> Result<[f32; 4], String> {
        Ok(self.color)
    }
}
