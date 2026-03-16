use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use cv_core::SharedState;
use windows::{
    core::{w, HRESULT},
    Win32::Foundation::POINT,
    Win32::Media::Speech::{ISpVoice, SPF_ASYNC, SpVoice},
    Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize,
        CLSCTX_ALL, COINIT_MULTITHREADED,
    },
    Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, UIA_CONTROLTYPE_ID, UIA_E_ELEMENTNOTAVAILABLE,
    },
    Win32::UI::WindowsAndMessaging::GetPhysicalCursorPos,
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

        // Bootstrap UIA — CUIAutomation8 gives correct TextPattern on Notepad.
        let automation: IUIAutomation = match unsafe {
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_ALL)
        } {
            Ok(a) => a,
            Err(e) => {
                eprintln!("[tts] CoCreateInstance IUIAutomation failed: {e}");
                drop(voice);
                unsafe { CoUninitialize() };
                return;
            }
        };

        let mut last_name = String::new();

        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));

            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // tts_enabled gates all future speech modes (hover, selection, caret).
            if !state.read().tts_enabled {
                continue;
            }

            // GetPhysicalCursorPos: mandatory at any DPI scaling — logical coords
            // from GetCursorPos would target the wrong element.
            let mut pt = POINT::default();
            if unsafe { GetPhysicalCursorPos(&mut pt) }.is_err() {
                continue;
            }

            let element = match unsafe { automation.ElementFromPoint(pt) } {
                Ok(el) => el,
                Err(e) => {
                    // UIA_E_ELEMENTNOTAVAILABLE is expected (desktop, UAC, screensaver).
                    if e.code() != HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32) {
                        eprintln!("[tts] ElementFromPoint failed: {e}");
                    }
                    continue;
                }
            };

            let name = match unsafe { element.CurrentName() } {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };

            if name.is_empty() || name == last_name {
                continue;
            }

            let control_type = unsafe { element.CurrentControlType() }.unwrap_or(UIA_CONTROLTYPE_ID(0));
            println!("[tts] hover: {:?} (type {})", name, control_type.0);

            last_name = name;
        }

        // Drop COM objects before CoUninitialize.
        drop(automation);
        drop(voice);
        unsafe { CoUninitialize() };
    })
}
