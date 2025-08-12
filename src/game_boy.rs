use std::time::Duration;
use crate::core::Core;

struct GameBoy {
    core: Core
}



impl GameBoy {
    pub fn dmg() -> Self {
        Self {
            core: Core::dmg_hello_world()
        }
    }

    pub fn run(&mut self) {
        loop {
            self.core.update(Duration::from_millis(16)); // TODO adjust based on actual frame time
            let opcode = self.core.fetch();
            self.core.execute(opcode);
            self.core.handle_interrupts();
        }
    }
}