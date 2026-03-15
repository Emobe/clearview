use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use cv_core::SharedState;
use windows::{
    core::w,
    Win32::Media::Speech::{ISpVoice, SPF_ASYNC, SpVoice},
    Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize,
        CLSCTX_ALL, COINIT_MULTITHREADED,
    },
};

pub fn spawn_tts_thread(
    shutdown: Arc<AtomicBool>,
    state: SharedState,
) -> std::thread::JoinHandle<()> {
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

        // Apply initial volume and rate from AppState, then announce.
        {
            let s = state.read();
            unsafe {
                let _ = voice.SetVolume(s.tts_volume as u16);
                let _ = voice.SetRate(s.tts_rate);
            }
        }
        unsafe {
            // Startup announcement is unconditional — proves SAPI works regardless
            // of the tts_enabled toggle, which defaults to false.
            if let Err(e) = voice.Speak(w!("clear-view ready"), SPF_ASYNC.0 as u32, None) {
                eprintln!("[tts] Speak failed: {e}");
            }
        }

        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if shutdown.load(Ordering::Relaxed) {
                break;
            }
            // tts_enabled gates all future speech modes (hover, selection, caret).
            // Nothing to do here yet — placeholder for Stage 2+.
            let _enabled = state.read().tts_enabled;
        }

        // Drop voice before CoUninitialize.
        drop(voice);
        unsafe { CoUninitialize() };
    })
}
