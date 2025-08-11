use crate::core::Core;

struct GameBoy {
    core: Core
}



impl GameBoy {
    pub fn dmg() -> Self {
        Self {
            core: Core::dmg_empty()
        }
    }

    pub fn run(&mut self) {
        loop {
            let opcode = self.core.fetch();
            self.core.execute(opcode);
        }
    }
}