use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use windows::{
    core::w,
    Win32::Media::Speech::{ISpVoice, SPF_ASYNC, SpVoice},
    Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize,
        CLSCTX_ALL, COINIT_MULTITHREADED,
    },
};

pub fn spawn_tts_thread(shutdown: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // MTA COM init — must be the very first COM call on this thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_err() {
            eprintln!("[tts] CoInitializeEx failed: {:?}", hr);
            return;
        }

        let voice: ISpVoice = match unsafe {
            CoCreateInstance(&SpVoice, None, CLSCTX_ALL)
        } {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[tts] CoCreateInstance ISpVoice failed: {e}");
                unsafe { CoUninitialize() };
                return;
            }
        };

        unsafe {
            let _ = voice.SetVolume(80);
            let _ = voice.SetRate(0);
            if let Err(e) = voice.Speak(w!("clear-view ready"), SPF_ASYNC.0 as u32, None) {
                eprintln!("[tts] Speak failed: {e}");
            }
        }

        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
        }

        // Drop voice before CoUninitialize.
        drop(voice);
        unsafe { CoUninitialize() };
    })
}
