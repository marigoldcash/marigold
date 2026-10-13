//! What the phone apps call. PH-M2 puts one C symbol here so the libraries built for the phones
//! have something to link and `nm` can show; PH-M3 replaces it with the UniFFI interface.

use std::ffi::{c_char, CStr};

static VERSION: &CStr = match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
    Ok(v) => v,
    Err(_) => panic!("version has a NUL"),
};

/// The version of the core, as a NUL-terminated string that lives as long as the library.
#[unsafe(no_mangle)]
pub extern "C" fn marigold_core_version() -> *const c_char {
    VERSION.as_ptr()
}
