pub mod cli;
pub mod lua;
pub mod model;
pub mod overlay;
pub mod platform;
pub mod recording;
pub mod runtime;
pub mod simulation;
pub mod store;
pub mod tray;
pub mod web;
pub mod workflow;

#[cfg(all(windows, not(target_arch = "x86_64")))]
compile_error!("RustyHotkey supports Windows x64 builds only.");

pub type Result<T> = std::result::Result<T, String>;

/// Operating-system boundary. Implementations must never inspect game memory.
pub trait Backend {
    /// Simulation backends may supply a bounded monotonic clock instead of wall time.
    fn virtual_time_ms(&self) -> Option<u64> {
        None
    }
    fn sleep_ms(&mut self, ms: u64) -> Result<()> {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        Ok(())
    }
    fn target_present(&mut self) -> Result<bool> {
        self.focused()
    }
    fn focused(&mut self) -> Result<bool>;
    fn key_down(&mut self, key: u16) -> Result<bool>;
    fn cursor(&mut self) -> Result<model::Point>;
    fn pixel(&mut self, point: model::Point) -> Result<u32>;
    fn clipboard(&mut self) -> Result<String>;
    fn emit(&mut self, event: &model::InputEvent) -> Result<()>;
    fn screenshot(&mut self, region: Option<model::Region>) -> Result<model::Capture>;
    fn ocr(&mut self, region: Option<model::Region>) -> Result<Vec<model::TextBox>>;
    fn notify(&mut self, message: &str, ttl_ms: u64) -> Result<()>;
    fn sound(&mut self, _cue: model::SoundCue) -> Result<()> {
        Err("sound is unavailable in this backend".into())
    }
}
