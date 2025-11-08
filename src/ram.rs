
pub trait ROM {
    fn read(&self, address: u16) -> u8;

    fn read_u16_le(&self, address: u16) -> u16 {
        u16::from_le_bytes([self.read(address), self.read(address + 1)])
    }

    fn read_u16_be(&self, address: u16) -> u16 {
        u16::from_be_bytes([self.read(address), self.read(address + 1)])
    }

    fn read_u32_be(&self, address: u16) -> u32 {
        u32::from_be_bytes([
            self.read(address),
            self.read(address + 1),
            self.read(address + 2),
            self.read(address + 3)
        ])
    }

    fn read_slice(&self, address: u16, length: usize) -> Vec<u8> {
        let mut result = vec![0; length];
        for i in 0..length {
            result[i] = self.read(address + i as u16);
        }
        result
    }
}

pub trait RAM: ROM {
    fn write(&mut self, address: u16, value: u8);

    fn write_u16_le(&mut self, address: u16, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write(address, low);
        self.write(address + 1, high);
    }

    fn write_u16_be(&mut self, address: u16, value: u16) {
        let [low, high] = value.to_be_bytes();
        self.write(address, low);
        self.write(address + 1, high);
    }

    fn write_u32_be(&mut self, address: u16, value: u32) {
        let bytes = value.to_be_bytes();
        for i in 0..bytes.len() {
            self.write(address + i as u16, bytes[i]);
        }
    }

    fn write_slice(&mut self, address: u16, data: &[u8]) {
        for (i, &byte) in data.iter().enumerate() {
            self.write(address + i as u16, byte);
        }
    }
}

impl ROM for Vec<u8> {
    fn read(&self, address: u16) -> u8 {
        self[address as usize]
    }
}

impl RAM for Vec<u8> {
    fn write(&mut self, address: u16, value: u8) {
        self[address as usize] = value;
    }
}

impl ROM for &[u8] {
    fn read(&self, address: u16) -> u8 {
        self[address as usize]
    }
}
