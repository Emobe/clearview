use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use cv_core::SharedState;
use windows::{
    core::{w, HRESULT, HSTRING, Ref},
    Win32::Foundation::POINT,
    Win32::Media::Speech::{ISpVoice, SPF_ASYNC, SPF_PURGEBEFORESPEAK, SpVoice},
    Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize,
        CLSCTX_ALL, COINIT_MULTITHREADED,
    },
    Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomationElement,
        IUIAutomationFocusChangedEventHandler,
        IUIAutomationFocusChangedEventHandler_Impl,
        IUIAutomationTextPattern,
        UIA_CONTROLTYPE_ID, UIA_E_ELEMENTNOTAVAILABLE,
        UIA_TextPatternId,
    },
    Win32::UI::WindowsAndMessaging::GetPhysicalCursorPos,
};

// ─── Internal message types ──────────────────────────────────────────────────

#[derive(Debug)]
enum Msg {
    TextFocusGained,
    TextFocusLost,
}

#[derive(Debug, PartialEq)]
enum TtsMode {
    Idle,
    TextFocus,
}

// ─── UIA focus-change event handler ─────────────────────────────────────────

/// Implements IUIAutomationFocusChangedEventHandler.
/// Fires on a UIA-internal thread — only plain data sent over the channel.
#[windows::core::implement(IUIAutomationFocusChangedEventHandler)]
struct FocusHandler {
    tx: mpsc::Sender<Msg>,
    own_pid: i32,
}

impl IUIAutomationFocusChangedEventHandler_Impl for FocusHandler_Impl {
    fn HandleFocusChangedEvent(
        &self,
        sender: Ref<IUIAutomationElement>,
    ) -> windows::core::Result<()> {
        // Null sender is legal — ignore silently.
        let el = match sender.as_ref() {
            Some(e) => e,
            None => return Ok(()),
        };

        // Filter events from our own process (egui panel) to avoid noise.
        if let Ok(pid) = unsafe { el.CurrentProcessId() } {
            if pid == self.own_pid {
                return Ok(());
            }
        }

        // IUIAutomationTextPattern presence == text-bearing element.
        let has_text = unsafe {
            el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        }
        .is_ok();

        let _ = self.tx.send(if has_text {
            Msg::TextFocusGained
        } else {
            Msg::TextFocusLost
        });

        Ok(())
    }
}

// ─── Thread entry point ───────────────────────────────────────────────────────

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

        // ── Stage 4: focus-change event handler ──────────────────────────────

        let (tx, rx) = mpsc::channel::<Msg>();
        let own_pid = std::process::id() as i32;

        let handler: IUIAutomationFocusChangedEventHandler =
            FocusHandler { tx, own_pid }.into();

        if let Err(e) = unsafe {
            automation.AddFocusChangedEventHandler(
                None::<&windows::Win32::UI::Accessibility::IUIAutomationCacheRequest>,
                &handler,
            )
        } {
            eprintln!("[tts] AddFocusChangedEventHandler failed: {e}");
            // Non-fatal: hover still works; state machine stays in Idle.
        }

        // ─────────────────────────────────────────────────────────────────────

        let mut last_name = String::new();
        let mut mode = TtsMode::Idle;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));

            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // Drain all pending focus-change messages before any speech decisions.
            loop {
                match rx.try_recv() {
                    Ok(Msg::TextFocusGained) => {
                        if mode != TtsMode::TextFocus {
                            mode = TtsMode::TextFocus;
                            println!("[tts] mode → TextFocus");
                        }
                    }
                    Ok(Msg::TextFocusLost) => {
                        if mode != TtsMode::Idle {
                            mode = TtsMode::Idle;
                            println!("[tts] mode → Idle");
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
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

            // Hover speech is suppressed while a text element has focus.
            if mode == TtsMode::TextFocus {
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

            let control_type =
                unsafe { element.CurrentControlType() }.unwrap_or(UIA_CONTROLTYPE_ID(0));
            println!("[tts] hover: {:?} (type {})", name, control_type.0);

            last_name = name.clone();

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

        // Deregister before dropping COM objects.
        if let Err(e) = unsafe { automation.RemoveFocusChangedEventHandler(&handler) } {
            eprintln!("[tts] RemoveFocusChangedEventHandler failed: {e}");
        }

        drop(handler);
        drop(automation);
        drop(voice);
        unsafe { CoUninitialize() };
    })
}
