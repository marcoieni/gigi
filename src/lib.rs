#[cfg(feature = "ssr")]
pub mod args;
#[cfg(feature = "ssr")]
pub mod authors;
#[cfg(feature = "ssr")]
pub mod checkout;
#[cfg(feature = "ssr")]
pub mod cmd;
#[cfg(feature = "ssr")]
pub mod commit;
#[cfg(feature = "ssr")]
pub mod config;
#[cfg(feature = "ssr")]
pub mod db;
#[cfg(feature = "ssr")]
pub mod github;
#[cfg(feature = "ssr")]
pub mod init;
#[cfg(feature = "ssr")]
pub mod launcher;
#[cfg(feature = "ssr")]
pub mod review;
#[cfg(feature = "ssr")]
pub mod serve;
#[cfg(feature = "ssr")]
pub mod terminal;
#[cfg(feature = "ssr")]
pub mod web;

pub mod dashboard_app;
pub mod dashboard_types;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(dashboard_app::App);
}
