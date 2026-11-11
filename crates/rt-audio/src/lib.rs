#[cfg(target_os = "android")]
pub(crate) use rt_core::android_log as android;
pub(crate) use rt_core::state;
#[cfg(target_os = "android")]
pub(crate) use rt_decode::android_video;
pub(crate) use rt_decode::iso;
#[cfg(any(target_os = "windows", target_os = "linux", test))]
pub(crate) use rt_decode::mpegh;

pub mod audio;
pub mod bitstream;
