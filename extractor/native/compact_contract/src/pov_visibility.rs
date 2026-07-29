//! Pointer-free exact-a729 current-POV province visibility carrier (d94).

pub const DOMAIN_ID: u16 = 94;
pub const DOMAIN_NAME: &str = "pov_province_visibility_live";
pub const CARRIER_MAGIC: [u8; 4] = *b"FOW1";
pub const CARRIER_VERSION: u16 = 1;
pub const MAX_PROVINCE_SLOTS: usize = 65_535;
pub const MAX_POV_COUNTRY_ID: u32 = 2_048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PovVisibilitySnapshot {
    pub capture_sequence: u64,
    pub pov_country_id: u32,
    /// Complete engine byte vector. Index is the province database ID.
    pub province_levels: Vec<u8>,
}

impl PovVisibilitySnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.capture_sequence == 0 {
            return Err("visibility capture sequence is zero".into());
        }
        if self.pov_country_id == 0 || self.pov_country_id > MAX_POV_COUNTRY_ID {
            return Err("visibility POV country ID is outside the admitted range".into());
        }
        if self.province_levels.is_empty() || self.province_levels.len() > MAX_PROVINCE_SLOTS {
            return Err("visibility province vector length is outside the admitted range".into());
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let count = u32::try_from(self.province_levels.len())
            .map_err(|_| "visibility province vector length does not fit u32")?;
        let mut out = Vec::with_capacity(24 + self.province_levels.len());
        out.extend_from_slice(&CARRIER_MAGIC);
        out.extend_from_slice(&CARRIER_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&self.capture_sequence.to_le_bytes());
        out.extend_from_slice(&self.pov_country_id.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&self.province_levels);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 24 || bytes[..4] != CARRIER_MAGIC {
            return Err("visibility carrier magic/length mismatch".into());
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let reserved = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        if version != CARRIER_VERSION || reserved != 0 {
            return Err("visibility carrier version/reserved mismatch".into());
        }
        let capture_sequence = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let pov_country_id = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        if bytes.len()
            != 24usize
                .checked_add(count)
                .ok_or("visibility carrier length overflow")?
        {
            return Err("visibility carrier is truncated or has trailing bytes".into());
        }
        let snapshot = Self {
            capture_sequence,
            pov_country_id,
            province_levels: bytes[24..].to_vec(),
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}
