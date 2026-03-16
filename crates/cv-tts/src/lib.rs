use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use cv_core::SharedState;
use windows::{
    core::{w, HRESULT, HSTRING},
    Win32::Foundation::POINT,
    Win32::Media::Speech::{ISpVoice, SPF_ASYNC, SPF_PURGEBEFORESPEAK, SpVoice},
    Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize,
        CLSCTX_ALL, COINIT_MULTITHREADED,
    },
    Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomationTextPattern,
        UIA_CONTROLTYPE_ID, UIA_E_ELEMENTNOTAVAILABLE, UIA_TextPatternId,
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

        // Apply initial volume/rate; track last-applied to avoid setting on every tick.
        let (mut last_volume, mut last_rate) = {
            let s = state.read();
            unsafe {
                let _ = voice.SetVolume(s.tts_volume as u16);
                let _ = voice.SetRate(s.tts_rate);
            }
            (s.tts_volume, s.tts_rate)
        };

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

            let (tts_enabled, tts_hover_enabled, volume, rate) = {
                let s = state.read();
                (s.tts_enabled, s.tts_hover_enabled, s.tts_volume, s.tts_rate)
            };

            if !tts_enabled {
                continue;
            }

            // Apply volume/rate only when they differ from last applied.
            unsafe {
                if volume != last_volume {
                    let _ = voice.SetVolume(volume as u16);
                    last_volume = volume;
                }
                if rate != last_rate {
                    let _ = voice.SetRate(rate);
                    last_rate = rate;
                }
            }

            if !tts_hover_enabled {
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

            // Update last_name regardless — so we don't re-speak the same element
            // immediately when text focus leaves.
            last_name = name.clone();

            // Suppress hover speech when a text-bearing element has focus (user is typing).
            let text_focused = match unsafe { automation.GetFocusedElement() } {
                Ok(focused_el) => unsafe {
                    focused_el
                        .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                        .is_ok()
                },
                Err(_) => false,
            };

            if text_focused {
                continue;
            }

            unsafe {
                let hstring = HSTRING::from(name.as_str());
                if let Err(e) = voice.Speak(
                    &hstring,
                    (SPF_ASYNC.0 | SPF_PURGEBEFORESPEAK.0) as u32,
                    None,
                ) {
                    eprintln!("[tts] Speak failed: {e}");
                }
            }
        }

        // Drop COM objects before CoUninitialize.
        drop(automation);
        drop(voice);
        unsafe { CoUninitialize() };
    })
}
