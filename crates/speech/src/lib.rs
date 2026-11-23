use std::ffi::{CStr, c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};

use anyhow::{Result, anyhow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeechEvent {
    Partial { text: String, speech_end_ms: Option<u64> },
    Eou { text: String, speech_end_ms: Option<u64> },
}

pub type OnEvent = Box<dyn Fn(SpeechEvent) + Send + Sync>;
pub type OnCleaned = Box<dyn Fn(&[f32]) + Send + Sync>;

struct Handler {
    event: OnEvent,
    cleaned: Option<OnCleaned>,
}

type EventFn = extern "C" fn(*mut c_void, i32, *const c_char, i64);
type AudioFn = extern "C" fn(*mut c_void, *const f32, usize);

unsafe extern "C" {
    fn milo_speech_start(
        aec: bool,
        callback: EventFn,
        cleaned: Option<AudioFn>,
        context: *mut c_void,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn milo_speech_push(handle: *mut c_void, mic: *const f32, mic_len: usize, reference: *const f32, ref_len: usize);
    fn milo_speech_transcribe(handle: *mut c_void, samples: *const f32, len: usize) -> *mut c_char;
    fn milo_speech_rearm(handle: *mut c_void, keep_transcript: bool);
    fn milo_speech_destroy(handle: *mut c_void);
    fn milo_speech_free_string(string: *mut c_char);
}

extern "C" fn trampoline(context: *mut c_void, kind: i32, text: *const c_char, ms: i64) {
    let handler = unsafe { &*(context as *const Handler) };
    let text = if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text) }.to_string_lossy().trim().to_string()
    };
    let event = match kind {
        0 => SpeechEvent::Partial { text, speech_end_ms: u64::try_from(ms).ok() },
        1 => SpeechEvent::Eou { text, speech_end_ms: u64::try_from(ms).ok() },
        3 => {
            log::info!("{text}");
            return;
        }
        _ => {
            log::warn!("{text}");
            return;
        }
    };
    if catch_unwind(AssertUnwindSafe(|| (handler.event)(event))).is_err() {
        log::error!("speech event handler panicked");
    }
}

extern "C" fn cleaned_trampoline(context: *mut c_void, samples: *const f32, len: usize) {
    let handler = unsafe { &*(context as *const Handler) };
    let Some(cleaned) = &handler.cleaned else { return };
    if samples.is_null() || len == 0 {
        return;
    }
    let audio = unsafe { std::slice::from_raw_parts(samples, len) };
    if catch_unwind(AssertUnwindSafe(|| cleaned(audio))).is_err() {
        log::error!("cleaned audio handler panicked");
    }
}

pub struct Speech {
    handle: NonNull<c_void>,
    handler: *mut Handler,
    aec: bool,
}

unsafe impl Send for Speech {}
unsafe impl Sync for Speech {}

impl Speech {
    pub fn start(aec: bool, on_event: OnEvent, on_cleaned: Option<OnCleaned>) -> Result<Speech> {
        let cleaned = on_cleaned.is_some().then_some(cleaned_trampoline as AudioFn);
        let handler = Box::into_raw(Box::new(Handler { event: on_event, cleaned: on_cleaned }));
        let mut error: *mut c_char = ptr::null_mut();
        let handle = unsafe { milo_speech_start(aec, trampoline, cleaned, handler.cast(), &mut error) };
        match NonNull::new(handle) {
            Some(handle) => Ok(Speech { handle, handler, aec }),
            None => {
                drop(unsafe { Box::from_raw(handler) });
                let message = if error.is_null() {
                    "speech start failed".to_string()
                } else {
                    let message = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
                    unsafe { milo_speech_free_string(error) };
                    message
                };
                Err(anyhow!(message))
            }
        }
    }

    pub fn push(&self, mic16k: &[f32], ref16k: &[f32]) {
        if mic16k.is_empty() {
            return;
        }
        debug_assert!(!self.aec || mic16k.len() == ref16k.len());
        let (reference, ref_len) = if self.aec { (ref16k.as_ptr(), ref16k.len()) } else { (ptr::null(), 0) };
        unsafe { milo_speech_push(self.handle.as_ptr(), mic16k.as_ptr(), mic16k.len(), reference, ref_len) };
    }

    pub fn transcribe(&self, audio16k: &[f32]) -> Option<String> {
        if audio16k.is_empty() {
            return None;
        }
        let raw = unsafe { milo_speech_transcribe(self.handle.as_ptr(), audio16k.as_ptr(), audio16k.len()) };
        if raw.is_null() {
            return None;
        }
        let text = unsafe { CStr::from_ptr(raw) }.to_string_lossy().trim().to_string();
        unsafe { milo_speech_free_string(raw) };
        (!text.is_empty()).then_some(text)
    }

    pub fn rearm(&self, keep_transcript: bool) {
        unsafe { milo_speech_rearm(self.handle.as_ptr(), keep_transcript) };
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        unsafe {
            milo_speech_destroy(self.handle.as_ptr());
            drop(Box::from_raw(self.handler));
        }
    }
}
