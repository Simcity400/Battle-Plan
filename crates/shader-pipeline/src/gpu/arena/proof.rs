use super::*;

impl Arena {
    pub fn proof_bytes_for_tests(&self) -> Vec<u8> {
        fn word(out: &mut Vec<u8>, value: u64) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        fn bytes(out: &mut Vec<u8>, value: &[u8]) {
            word(out, value.len() as u64);
            out.extend_from_slice(value);
        }
        fn recent(out: &mut Vec<u8>, value: &Recent) {
            for v in [value.offset, value.length, value.reserved] {
                word(out, v as u64);
            }
        }
        let inner = self.inner.borrow();
        let mut out = Vec::new();
        for v in [
            self.alignment,
            inner.frame,
            inner.current as u64,
            inner.chunks.len() as u64,
        ] {
            word(&mut out, v);
        }
        for chunk in &inner.chunks {
            word(&mut out, chunk.buffer.size());
            word(&mut out, u64::from(chunk.buffer.usage().bits()));
            bytes(&mut out, &chunk.staged);
            word(&mut out, chunk.flushed as u64);
        }
        word(&mut out, inner.recent.len() as u64);
        for r in &inner.recent {
            recent(&mut out, r);
        }
        word(&mut out, inner.last.len() as u64);
        for r in &inner.last {
            word(&mut out, u64::from(r.is_some()));
            if let Some(r) = r {
                recent(&mut out, r);
            }
        }
        word(&mut out, inner.vertices.len() as u64);
        for r in &inner.vertices {
            recent(&mut out, r);
        }
        let mut external = Vec::<&wgpu::Buffer>::new();
        word(&mut out, inner.writes.len() as u64);
        for (buffer, offset, data) in &inner.writes {
            let identity =
                if let Some(index) = inner.chunks.iter().position(|c| &c.buffer == buffer) {
                    index
                } else {
                    let index = external
                        .iter()
                        .position(|b| *b == buffer)
                        .unwrap_or_else(|| {
                            external.push(buffer);
                            external.len() - 1
                        });
                    inner.chunks.len() + index
                };
            word(&mut out, identity as u64);
            word(&mut out, buffer.size());
            word(&mut out, u64::from(buffer.usage().bits()));
            word(&mut out, *offset);
            bytes(&mut out, data);
        }
        word(&mut out, inner.refusals.len() as u64);
        for refusal in &inner.refusals {
            bytes(&mut out, refusal.as_bytes());
        }
        let packed = self.packed.borrow();
        word(&mut out, packed.len() as u64);
        for value in packed.iter() {
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        out
    }
}
