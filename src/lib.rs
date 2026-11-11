pub mod cli;
pub mod lua;
pub mod model;
pub mod overlay;
pub mod platform;
pub mod recording;
pub mod runtime;
pub mod store;
pub mod tray;
pub mod web;
pub mod workflow;

#[cfg(all(windows, not(target_arch = "x86_64")))]
compile_error!("RustyHotkey supports Windows x64 builds only.");

pub type Result<T> = std::result::Result<T, String>;

/// Operating-system boundary. Implementations must never inspect game memory.
pub trait Backend {
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
}
