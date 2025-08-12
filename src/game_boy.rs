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
            let opcode = self.core.fetch();
            self.core.execute(opcode);
            self.core.handle_interrupts();
        }
    }
}