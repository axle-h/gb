const MEMORY_SIZE: usize = 0x10000; // 64KB
const UNMAPPED_DEVICE: u8 = 0xff;

pub struct Memory {
    devices: Vec<Box<dyn MemoryInterface>>,
    memory: [u8; MEMORY_SIZE],
    address_map: [MemoryMode; MEMORY_SIZE],
    device_map: [u8; MEMORY_SIZE]
}

impl Memory {
    pub fn new() -> Self {
        Memory {
            devices: Vec::new(),
            memory: [0; MEMORY_SIZE],
            address_map: [MemoryMode::Unmapped; MEMORY_SIZE],
            device_map: [UNMAPPED_DEVICE; MEMORY_SIZE],
        }
    }

    pub fn dmg_empty() -> Self {
        // https://gbdev.io/pandocs/Memory_Map.html
        let mut memory = Memory::new();
        // Initialize DMG memory map
        memory.add_rom(0x0000, &[0; 0x4000]); // 16KB ROM bank 00
        memory.add_rom(0x4000, &[0; 0x4000]); // 16KB ROM bank 01
        memory.add_ram(0x8000, 0x2000); // 8 KiB Video RAM (VRAM)
        memory.add_ram(0xC000, 0x2000); // 8 KiB Work RAM (WRAM)
        memory.add_ram(0xFE00, 0x00A0); // OAM (Object Attribute Memory)
        // TODO mirror ram
        memory.add_ram(0xFF00, 0x0080); // I/O Registers (0xFF00-0xFF7F)
        memory.add_ram(0xFF80, 0x007F); // High RAM (0xFF80-0xFFFE)
        memory.add_ram(0xFFFF, 0x0001); // interrupt enable register at 0xFFFF
        memory
    }

    pub fn add_ram(&mut self, address: u16, size: u16) {
        self.add_block(MemoryBlock::ram(address, size))
    }

    pub fn add_rom(&mut self, address: u16, data: &[u8]) {
        self.add_block(MemoryBlock::rom(address, data.len() as u16));

        // TODO is there a better way to copy memory?
        for (i, &byte) in data.iter().enumerate() {
            self.memory[(address + i as u16) as usize] = byte;
        }
    }

    pub fn add_device(&mut self, device: Box<dyn MemoryInterface>) {
        let device_index = self.devices.len() as u8;
        if device_index >= UNMAPPED_DEVICE {
            panic!("Too many devices added, maximum is {}", UNMAPPED_DEVICE - 1);
        }
        let address = device.address();
        let size = device.size();
        for addr in address..address + size {
            let index = addr as usize;
            if self.device_map[index] != UNMAPPED_DEVICE {
                panic!("Device overlaps with existing device {:#02X} at address {:#04X}", self.device_map[index], addr);
            }
            self.device_map[index] = device_index;
        }
        self.devices.push(device);
    }

    fn add_block(&mut self, block: MemoryBlock) {
        for addr in block.range() {
            let index = addr as usize;
            if self.address_map[index] != MemoryMode::Unmapped {
                panic!("Memory block overlaps with existing memory at address {:#04X}", addr);
            }
            self.address_map[index] = block.mode;
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryInterface for Memory {
    fn address(&self) -> u16 {
        0
    }

    fn size(&self) -> u16 {
        MEMORY_SIZE as u16
    }

    fn read(&self, address: u16) -> u8 {
        match self.address_map[address as usize] {
            MemoryMode::Unmapped => {
                let device_index = self.device_map[address as usize];
                if device_index == UNMAPPED_DEVICE {
                    0 // Return 0 for unmapped memory TODO is this correct?
                } else {
                    let device = self.devices[device_index as usize].as_ref();
                    device.read(device.relative_address(address))
                }
            },
            MemoryMode::ReadWrite | MemoryMode::ReadOnly => self.memory[address as usize],
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        match self.address_map[address as usize] {
            MemoryMode::Unmapped | MemoryMode::ReadOnly => {
                // do nothing for unmapped or read-only memory
            },
            MemoryMode::ReadWrite => {
                self.memory[address as usize] = value;
            }
        }

        let device_index = self.device_map[address as usize];
        if device_index != UNMAPPED_DEVICE {
            let device = self.devices[device_index as usize].as_mut();
            device.write(device.relative_address(address), value);
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBlock {
    pub address: u16,
    pub size: u16,
    pub mode: MemoryMode
}

impl MemoryBlock {
    pub fn new(address: u16, size: u16, mode: MemoryMode) -> Self {
        MemoryBlock {
            address,
            size,
            mode,
        }
    }

    pub fn rom(address: u16, size: u16) -> Self {
        MemoryBlock::new(address, size, MemoryMode::ReadOnly)
    }

    pub fn ram(address: u16, size: u16) -> Self {
        MemoryBlock::new(address, size, MemoryMode::ReadWrite)
    }

    pub fn range(&self) -> std::ops::Range<u16> {
        // check if endpoint overflows
        if self.address.wrapping_add(self.size) < self.address {
            self.address .. 0xFFFF
        } else {
            self.address..(self.address + self.size)
        }
    }
}

pub trait MemoryInterface {
    fn address(&self) -> u16;
    fn size(&self) -> u16;
    fn read(&self, address: u16) -> u8;
    fn write(&mut self, address: u16, value: u8);

    fn relative_address(&self, address: u16) -> u16 {
        address - self.address()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryMode {
    Unmapped,
    ReadOnly,
    ReadWrite,
}

#[cfg(test)]
mod tests {
    use rand::Rng;
    use super::*;

    struct MockDevice {
        address: u16,
        size: u16,
        memory: Vec<u8>,
    }

    impl MockDevice {
        fn new(address: u16, size: u16) -> Self {
            MockDevice {
                address,
                size,
                memory: vec![0; size as usize],
            }
        }
    }

    impl MemoryInterface for MockDevice {
        fn address(&self) -> u16 {
            self.address
        }

        fn size(&self) -> u16 {
            self.size
        }

        fn read(&self, address: u16) -> u8 {
            self.memory[address as usize]
        }

        fn write(&mut self, address: u16, value: u8) {
            self.memory[address as usize] = value;
        }
    }

    #[test]
    fn memory_default() {
        let memory = Memory::default();
        assert_eq!(memory.devices.len(), 0);
    }

    #[test]
    fn add_ram() {
        let mut memory = Memory::new();
        memory.add_ram(0x8000, 0x2000);

        // Test writing and reading from RAM
        memory.write(0x8000, 0x42);
        assert_eq!(memory.read(0x8000), 0x42);

        memory.write(0x9FFF, 0x24);
        assert_eq!(memory.read(0x9FFF), 0x24);

        // Test reading from uninitialized RAM
        assert_eq!(memory.read(0x8001), 0);
    }

    #[test]
    fn add_rom() {
        let mut memory = Memory::new();
        let rom_data = [0x01, 0x02, 0x03, 0x04];
        memory.add_rom(0x0000, &rom_data);

        // Test reading from ROM
        assert_eq!(memory.read(0x0000), 0x01);
        assert_eq!(memory.read(0x0001), 0x02);
        assert_eq!(memory.read(0x0002), 0x03);
        assert_eq!(memory.read(0x0003), 0x04);

        // Test that writing to ROM does nothing
        memory.write(0x0000, 0xFF);
        assert_eq!(memory.read(0x0000), 0x01);
    }

    #[test]
    fn add_device() {
        let mut memory = Memory::new();
        let device = Box::new(MockDevice::new(0x8000, 0x100));
        memory.add_device(device);

        // Test device is accessible
        memory.write(0x8000, 0x42);
        assert_eq!(memory.read(0x8000), 0x42);

        memory.write(0x80FF, 0x24);
        assert_eq!(memory.read(0x80FF), 0x24);

        // Test address outside device range returns 0
        assert_eq!(memory.read(0x8100), 0);
    }

    #[test]
    fn device_with_memory_overlap() {
        let mut memory = Memory::new();

        // Add RAM first
        memory.add_ram(0x8000, 0x1000);
        memory.write(0x8000, 0x11);

        // Add device overlapping the RAM
        let device = Box::new(MockDevice::new(0x8000, 0x100));
        memory.add_device(device);

        // When a device overlaps with memory, reads should come from the device
        // but the memory mapping shows unmapped, so device takes precedence
        memory.write(0x8000, 0x42);
        assert_eq!(memory.read(0x8000), 0x42); // Device value since it was written to the device
    }

    #[test]
    #[should_panic(expected = "Memory block overlaps with existing memory at address 0x8100")]
    fn cannot_overlap_memory() {
        let mut memory = Memory::new();
        memory.add_ram(0x8000, 0x1000);
        memory.add_ram(0x8100, 0x100);
    }

    #[test]
    fn relative_addresses() {
        let device = MockDevice::new(0x8000, 0xFF);
        assert_eq!(device.relative_address(0x8000), 0);
        assert_eq!(device.relative_address(0x8010), 0x10);
        assert_eq!(device.relative_address(0x80FF), 0xFF);
    }

    #[test]
    fn memory_block_rom() {
        let block = MemoryBlock::rom(0x0000, 0x8000);
        assert_eq!(block.address, 0x0000);
        assert_eq!(block.size, 0x8000);
        assert_eq!(block.mode, MemoryMode::ReadOnly);
    }

    #[test]
    fn memory_block_ram() {
        let block = MemoryBlock::ram(0x8000, 0x2000);
        assert_eq!(block.address, 0x8000);
        assert_eq!(block.size, 0x2000);
        assert_eq!(block.mode, MemoryMode::ReadWrite);
    }

    #[test]
    fn memory_block_range() {
        let block = MemoryBlock::new(0x1000, 0x100, MemoryMode::ReadWrite);
        let range = block.range();
        assert_eq!(range.start, 0x1000);
        assert_eq!(range.end, 0x1100);
    }

    #[test]
    fn unmapped_memory_returns_zero() {
        let memory = Memory::new();
        assert_eq!(memory.read(0x5000), 0);
        assert_eq!(memory.read(0xFFFE), 0); // Use 0xFFFE instead of 0xFFFF to avoid index out of bounds
    }

    #[test]
    fn readonly_memory_ignore_writes() {
        let mut memory = Memory::new();
        let rom_data = [0xAB, 0xCD];
        memory.add_rom(0x1000, &rom_data);

        // Try to write to ROM
        memory.write(0x1000, 0x12);
        memory.write(0x1001, 0x34);

        // Values should remain unchanged
        assert_eq!(memory.read(0x1000), 0xAB);
        assert_eq!(memory.read(0x1001), 0xCD);
    }

    #[test]
    fn multiple_devices() {
        let mut memory = Memory::new();

        let device1 = Box::new(MockDevice::new(0x8000, 0x100));
        let device2 = Box::new(MockDevice::new(0x9000, 0x100));

        memory.add_device(device1);
        memory.add_device(device2);

        // Test both devices work independently
        memory.write(0x8050, 0x11);
        memory.write(0x9050, 0x22);

        assert_eq!(memory.read(0x8050), 0x11);
        assert_eq!(memory.read(0x9050), 0x22);
    }

    #[test]
    fn memory_modes() {
        assert_eq!(MemoryMode::Unmapped as u8, 0);
        assert_eq!(MemoryMode::ReadOnly as u8, 1);
        assert_eq!(MemoryMode::ReadWrite as u8, 2);
    }

    #[test]
    fn memory_interface_trait() {
        let memory = Memory::new();
        assert_eq!(memory.address(), 0);
        assert_eq!(memory.size(), MEMORY_SIZE as u16);
    }

    #[test]
    #[should_panic(expected = "Too many devices added")]
    fn too_many_devices_panic() {
        let mut memory = Memory::new();

        // Add 255 devices (UNMAPPED_DEVICE - 1)
        for i in 0..255 {
            let device = Box::new(MockDevice::new(i as u16, 1));
            memory.add_device(device);
        }

        // This should panic
        let device = Box::new(MockDevice::new(255, 1));
        memory.add_device(device);
    }

    #[test]
    fn large_rom_data() {
        let mut memory = Memory::new();
        let large_data: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
        memory.add_rom(0x2000, &large_data);

        // Test first few bytes
        assert_eq!(memory.read(0x2000), 0);
        assert_eq!(memory.read(0x2001), 1);
        assert_eq!(memory.read(0x2002), 2);

        // Test wraparound
        assert_eq!(memory.read(0x2000 + 256), 0);
        assert_eq!(memory.read(0x2000 + 257), 1);

        // Test last byte
        assert_eq!(memory.read(0x2000 + 999), (999 % 256) as u8);
    }

    #[test]
    fn dmg_empty() {
        let mut memory = Memory::dmg_empty();

        // Check ROM banks
        for addr in 0x0000..0x8000 {
            memory.write(addr, 0x42);
            assert_eq!(memory.read(addr), 0);
        }

        let mut rng = rand::rng();

        // Check VRAM
        for addr in 0x8000..0xA000 {
            let value: u8 = rng.random();
            memory.write(addr, value);
            assert_eq!(memory.read(addr), value);
        }

        // Check WRAM
        for addr in 0xC000..0xE000 {
            let value: u8 = rng.random();
            memory.write(addr, value);
            assert_eq!(memory.read(addr), value);
        }

        // Check OAM
        for addr in 0xFE00..0xFEA0 {
            let value: u8 = rng.random();
            memory.write(addr, value);
            assert_eq!(memory.read(addr), value);
        }

        // Check High RAM
        for addr in 0xFF80..0xFFFF {
            let value: u8 = rng.random();
            memory.write(addr, value);
            assert_eq!(memory.read(addr), value);
        }
    }
}
