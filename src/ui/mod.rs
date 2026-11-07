//! Desktop interface (egui).

pub mod app;
pub mod dialogs;
pub mod icons;
pub mod text;
pub mod theme;
pub mod widgets;

pub fn run(
    urls: Vec<String>,
    from_browser: bool,
    screenshot: Option<String>,
    setup_browsers: bool,
) -> i32 {
    app::run(urls, from_browser, screenshot, setup_browsers)
}
