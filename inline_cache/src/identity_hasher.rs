#[derive(Default)]
pub struct IdentityHasher {
    state: u64,
}

impl IdentityHasher {}

impl std::hash::Hasher for IdentityHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.state
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        if let Some(chunk) = bytes.last_chunk::<8>() {
            self.state = u64::from_le_bytes(*chunk);
        } else {
            for &b in bytes {
                self.write_u8(b);
            }
        }
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.state = i;
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.state = (self.state >> 8) | ((i as u64) << (64 - 8));
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.state = (self.state >> 16) | ((i as u64) << (64 - 16));
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.state = (self.state >> 32) | ((i as u64) << (64 - 32));
    }
}
