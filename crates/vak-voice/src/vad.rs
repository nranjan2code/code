#[derive(Debug, Clone)]
pub struct EnergyVad {
    threshold: i16,
    hangover_frames: usize,
    silence: usize,
    speaking: bool,
}

impl EnergyVad {
    pub fn new(threshold: i16, hangover_frames: usize) -> Self {
        Self {
            threshold,
            hangover_frames,
            silence: 0,
            speaking: false,
        }
    }
    pub fn observe(&mut self, frame: &[i16]) -> VadEvent {
        let active = frame
            .iter()
            .any(|v| v.unsigned_abs() >= self.threshold.unsigned_abs());
        if active {
            self.silence = 0;
            if !self.speaking {
                self.speaking = true;
                return VadEvent::Started;
            }
        } else if self.speaking {
            self.silence += 1;
            if self.silence > self.hangover_frames {
                self.speaking = false;
                self.silence = 0;
                return VadEvent::Stopped;
            }
        }
        VadEvent::None
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    None,
    Started,
    Stopped,
}
