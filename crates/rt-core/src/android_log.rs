#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        prio: i32,
        tag: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
    ) -> i32;
}

pub fn log_android(prio: i32, msg: &str) {
    if let Ok(c_msg) = std::ffi::CString::new(msg) {
        unsafe {
            __android_log_write(prio, c"RustTracker".as_ptr(), c_msg.as_ptr());
        }
    }
}
