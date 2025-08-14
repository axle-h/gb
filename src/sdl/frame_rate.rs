use std::time::{Duration, SystemTime};

#[derive(Debug, Copy, Clone)]
pub struct FrameRate {
    t0: SystemTime,
}

impl Default for FrameRate {
    fn default() -> Self {
        Self {
            t0: SystemTime::now(),
        }
    }
}

impl FrameRate {
    /// Registers the start of a new frame, returns the time since the last frame
    pub fn update(&mut self) -> Result<Duration, String> {
        // TODO have option of limiting/recording the effective framerate
        let now = SystemTime::now();
        let delta = now.duration_since(self.t0).map_err(|e| e.to_string())?;
        self.t0 = now;
        Ok(delta)
    }
}
