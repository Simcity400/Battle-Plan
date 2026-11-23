use engine_native_memory::{Memory, read};

pub const HEADER: usize = 0x28;
pub const STRIDE: usize = 12;

#[derive(Clone, Copy)]
pub struct Map {
    data: usize,
    count: u32,
    mask: u32,
    probe: u8,
    stride: usize,
    marker: usize,
}
impl Map {
    pub(crate) fn hint(&self, key: u32) {
        if self.count == 0 {
            return;
        }
        let mix = |v: u32| (v >> 16 ^ v).wrapping_mul(0x045d_9f3b);
        let h = mix(mix(key));
        let first = (h >> 16 ^ h) & self.mask;
        if let Some(at) = self.data.checked_add(first as usize * self.stride) {
            engine_native_memory::prefetch::span(at, self.stride);
        }
    }

    pub(crate) fn read(
        memory: &impl Memory,
        at: usize,
        stride: usize,
        marker: usize,
    ) -> Result<Self, &'static str> {
        let header = read::<[u8; 17]>(memory, at).ok_or("supply map header unreadable")?;
        let data = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
        let count = u32::from_le_bytes(header[8..12].try_into().unwrap());
        let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
        let probe = header[16];
        let capacity = mask
            .checked_add(1)
            .filter(|v| v.is_power_of_two())
            .ok_or("supply map mask overflows or is invalid")?;
        if data == 0 || count > capacity || probe == 255 {
            return Err("supply map header invalid");
        }
        let terminal = data
            .checked_add((capacity as usize + probe as usize) * stride)
            .and_then(|v| v.checked_add(marker))
            .ok_or("supply map address overflow")?;
        if read::<u8>(memory, terminal) != Some(255) {
            return Err("supply map sentinel invalid");
        }
        Ok(Self {
            data,
            count,
            mask,
            probe,
            stride,
            marker,
        })
    }
    pub(crate) fn find(
        &self,
        memory: &impl Memory,
        key: u32,
    ) -> Result<Option<usize>, &'static str> {
        if self.count == 0 {
            return Ok(None);
        }
        let mix = |v: u32| (v >> 16 ^ v).wrapping_mul(0x045d_9f3b);
        let h = mix(mix(key));
        let first = (h >> 16 ^ h) & self.mask;
        for distance in 1..=u32::from(self.probe) + 1 {
            let index = first + distance - 1;
            if index >= self.mask + 1 + u32::from(self.probe) {
                break;
            }
            let at = self
                .data
                .checked_add(index as usize * self.stride)
                .ok_or("supply map probe overflow")?;
            let bytes =
                read::<[u8; 8]>(memory, at + self.marker).ok_or("supply map bucket unreadable")?;
            if bytes[0] == 255 || u32::from(bytes[0]) > u32::from(self.probe) + 1 {
                return Err("supply map marker invalid");
            }
            if u32::from(bytes[0]) < distance {
                return Ok(None);
            }
            if u32::from_le_bytes(bytes[4..].try_into().unwrap()) == key {
                return Ok(Some(at));
            }
        }
        Ok(None)
    }
    pub(crate) fn parent(&self, memory: &impl Memory, province: u32) -> Result<i32, &'static str> {
        self.find(memory, province)?.map_or(Ok(-1), |p| {
            read::<i32>(memory, p + 8).ok_or("supply parent unreadable")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_supply_map_mask_above_former_chosen_cap_uses_its_original_geometry() {
        struct Header;
        impl Memory for Header {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                if at == 0x1000 && output.len() == 17 {
                    output.fill(0);
                    output[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
                    output[12..16].copy_from_slice(&((1_u32 << 21) - 1).to_le_bytes());
                    true
                } else if at == 0x2000 + (1 << 21) * STRIDE && output.len() == 1 {
                    output[0] = 255;
                    true
                } else {
                    false
                }
            }
        }
        let map = Map::read(&Header, 0x1000, STRIDE, 0).unwrap();
        assert_eq!(map.parent(&Header, 1), Ok(-1));
    }
}
