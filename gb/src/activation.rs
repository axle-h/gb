pub trait Activation {

    fn is_activation_pending(&self) -> bool;

    fn clear_activation(&mut self);

    fn consume_pending_activation(&mut self) -> bool {
        if self.is_activation_pending() {
            self.clear_activation();
            true
        } else {
            false
        }
    }
}