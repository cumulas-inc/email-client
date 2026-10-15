//! Core of Cumulas Mail: accounts, sync, local storage and protocol providers.
//! Independent of Tauri so it can be tested headless.

pub mod auth;
pub mod error;
pub mod html;
pub mod model;
pub mod providers;
pub mod push;
pub mod reminders;
pub mod secrets;
pub mod service;
pub mod store;

pub use error::{Error, ErrorCode, Result};
pub use service::CoreService;
