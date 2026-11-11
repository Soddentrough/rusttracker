#[cfg(not(target_os = "android"))]
pub(crate) use rt_core::state;

#[cfg(not(target_os = "android"))]
pub mod ui;
