use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Mutex;

use serde_json::Value;

unsafe extern "C" {
    fn milo_control_call(message: *const c_char) -> *mut c_char;
    fn free(pointer: *mut c_void);
}

static TURN: Mutex<()> = Mutex::new(());

pub fn call(request: &Value) -> Result<String, String> {
    let failed = || "it did not work".to_string();
    let message = CString::new(request.to_string()).map_err(|_| failed())?;
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let raw = unsafe { milo_control_call(message.as_ptr()) };
    if raw.is_null() {
        return Err(failed());
    }
    let answer = unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned();
    unsafe { free(raw.cast()) };
    let answer: Value = serde_json::from_str(&answer).map_err(|_| failed())?;
    let text = answer["text"].as_str().unwrap_or_default().to_string();
    match answer["ok"].as_bool() {
        Some(true) => Ok(text),
        _ if text.is_empty() => Err(failed()),
        _ => Err(text),
    }
}
