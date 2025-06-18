pub struct BinaryWriter<'a> {
    buffer: &'a mut Vec<u8>,
    bit_offset: usize,
}

impl<'a> BinaryWriter<'a> {
    pub fn new(buffer: &'a mut Vec<u8>) -> Self {
        Self {
            buffer,
            bit_offset: 0,
        }
    }

    pub fn write_u8(&mut self, value: u8) {
        self.round_up_byte_offset();

        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_f64(&mut self, value: f64) {
        self.round_up_byte_offset();

        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    fn round_up_byte_offset(&mut self) {
        if self.bit_offset > 0 {
            self.bit_offset = 0;
        }
    }
}
