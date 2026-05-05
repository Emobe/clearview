# clear-view TTS Plan

Screen reader / TTS feature plan for clear-view.
When starting a session, tell Claude Code which stage(s) you are working on.

---

## What We Are Building

Four reading modes that coexist:

| Mode | Trigger | Speaks |
|------|---------|--------|
| Hover | Cursor rests over a UI element | Element name / role |
| Selection | User selects text with mouse or keyboard | The selected text |
| Caret following | User moves cursor with arrow keys in a text field | Word or character at new caret position |
| Typing echo | User types | Each word when space/punctuation hit; each character in character mode |

### Priority when modes conflict

Typing echo > Selection > Caret following > Hover

New speech always interrupts old speech via `SPF_PURGEBEFORESPEAK`.
Hover polling is paused whenever a text element has focus. It resumes when focus
leaves text entirely.

### App coverage

| App type | API used |
|----------|----------|
| Windows native (Notepad, Win32, WPF, UWP) | UIA |
| Chrome / Edge / Brave (v126+) | UIA (native, no shim needed) |
| Microsoft Word | UIA |
| Firefox | IAccessible2 (IA2) — see Stage 9 |
| LibreOffice | IAccessible2 (IA2) — see Stage 9 |

LibreOffice and Firefox both use IA2 on Windows, not UIA. UIA clients can see
LibreOffice UI chrome (menus, toolbars) but not the document body. IA2 is unavoidable
for those apps if document reading is needed.

---

## Architecture

### New crate: `crates/cv-tts`

A single **TTS thread** owns all UIA and SAPI state. Rules:
- Call `CoInitializeEx(NULL, COINIT_MULTITHREADED)` as the very first call on this
  thread, before creating any COM object.
- This thread must never own a window.
- All `AddXxxEventHandler` / `RemoveXxxEventHandler` calls happen on this thread only.
  Never add or remove event handlers from two threads simultaneously.

### Internal state machine

```
Idle
  → focus enters text element    → TextFocus(element)
  → hover tick (no text focus)   → Hovering

TextFocus(element)
  → selection changes            → speak selection
  → caret moves (degenerate sel) → speak word/char at caret
  → key input                    → typing echo
  → focus leaves text            → Idle

Hovering
  → hover tick (no text focus)   → speak element name if changed
  → focus enters text element    → TextFocus(element)
```

Event handlers fire on a UIA-internal thread. Send only plain data (strings,
booleans) over an `mpsc` channel back to the TTS thread — never send COM pointers
across threads or call `Speak` from inside a handler.

### AccessibilityBackend — future-proofing IA2

All element queries go through a single internal trait so Stage 8 (IA2) can be added
without touching the state machine or speech logic:

```rust
enum AccessibilityBackend { Uia(UiaBackend), Ia2(Ia2Backend) }

impl AccessibilityBackend {
    fn element_at_point(&self, pt: POINT) -> Option<ElementInfo>;
    fn focused_element(&self) -> Option<ElementInfo>;
    fn text_at_caret(&self, granularity: TtsGranularity) -> Option<String>;
    fn selected_text(&self) -> Option<String>;
}

struct ElementInfo {
    name: String,
    role: String,
    has_text_pattern: bool,
}
```

Detection logic (which backend to use) lives in one place:

```rust
fn detect_backend(hwnd: HWND) -> AccessibilityBackend {
    match get_class_name(hwnd).as_str() {
        "SALFRAME" | "MozillaWindowClass" => AccessibilityBackend::Ia2(...),
        _ => AccessibilityBackend::Uia(...),
    }
}
```

Stages 1–8 implement only `UiaBackend`. Stage 9 adds `Ia2Backend` as a second branch.
The state machine never needs to change.

### describe_element — verbosity hook

All speech output for element descriptions goes through one function:

```rust
fn describe_element(info: &ElementInfo, verbosity: TtsVerbosity) -> String {
    match verbosity {
        TtsVerbosity::NameOnly  => info.name.clone(),
        TtsVerbosity::NameRole  => format!("{}, {}", info.name, info.role),
    }
}
```

Add to `AppState`:
```rust
pub tts_verbosity: TtsVerbosity, // default: NameOnly
```
```rust
pub enum TtsVerbosity { NameOnly, NameRole }
```

Stages 1–7 use `NameOnly`. Adding `NameRole` later is a one-line change.
Nothing else in the codebase needs to know about verbosity.

### Future-proofing: scripting and per-app behaviour

Eventually this project may support JAWS-style scripting — per-application behaviour
overrides, custom keystroke handlers, and user-defined speech rules. The architectural
rule that prevents a rewrite is simple:

**All app-specific behaviour must go through a single dispatch point, never be
scattered in the TTS thread loop.**

Concretely: never write `if app_name == "chrome" { ... }` directly in the state
machine. Instead, route it through an `AppContext` struct that is resolved once when
focus changes and consulted everywhere else:

```rust
struct AppContext {
    app_name: String,       // e.g. "chrome", "notepad"
    app_class: String,      // window class name
    backend: AccessibilityBackend,
    // Future: script_hooks: Option<ScriptHooks>
}
```

When scripting is added, `ScriptHooks` is populated from a loaded script file for
that app. The state machine calls `app_context.describe_element(info)` and
`app_context.on_focus_changed(element)` — the hooks intercept if a script is loaded,
otherwise fall through to defaults. Nothing else in the codebase changes.

For now `AppContext` just holds the backend — the hook slots are `// Future:` stubs.

### Future-proofing: OCR

OCR (for inaccessible apps, image-based PDFs, games) is explicitly out of scope for
the current plan. When it is added, it must be a separate backend variant:

```rust
enum AccessibilityBackend { Uia(UiaBackend), Ia2(Ia2Backend), Ocr(OcrBackend) }
```

`OcrBackend` uses `Windows.Media.Ocr` (WinRT, available via the `windows` crate's
`Media_Ocr` feature) to capture a screen region and extract text. It is only
activated when UIA and IA2 both return nothing useful for an element. Detection is
the hard part — a placeholder note: check `CurrentControlType == Image` and
`CurrentName.is_empty()` as the trigger condition.

No OCR code is written in the current plan. The enum variant slot is reserved so
adding it later is an additive change.

---

### New AppState fields (add to `cv-core`)

```rust
pub tts_enabled: bool,               // default: false — master switch, gates everything
pub tts_hover_enabled: bool,         // default: true
pub tts_selection_enabled: bool,     // default: true
pub tts_caret_enabled: bool,         // default: true
pub tts_typing_enabled: bool,        // default: true
pub tts_appreader_enabled: bool,     // default: true
pub tts_volume: u32,                 // 0–100, default: 80
pub tts_rate: i32,                   // -10 to 10, default: 0
pub tts_granularity: TtsGranularity, // default: Word
pub tts_char_mode: bool,             // default: false (for codes/terminals)
pub tts_verbosity: TtsVerbosity,     // default: NameOnly
```

```rust
pub enum TtsGranularity { Word, Character }
pub enum TtsVerbosity   { NameOnly, NameRole }
```

### egui panel additions

- TTS on/off toggle (master)
- Per-mode checkboxes, greyed out when master is off:
  - Hover echo
  - Selection reading
  - Caret following
  - Typing echo
  - AppReader
- Volume slider
- Rate slider
- Granularity radio (Word / Character)
- Character mode checkbox ("Character mode — for codes and terminals")
- Verbosity radio (Name only / Name + role)

Each mode checks its own flag at the point speech would be triggered in the state
machine — not in one central place. The master `tts_enabled` is checked first; if
false, nothing else is evaluated.

---

## Stage 1 — Crate Scaffolding + SAPI Smoke Test

**Goal:** `crates/cv-tts` exists, the TTS thread starts with correct MTA COM init,
and speaks one hardcoded phrase on launch to prove SAPI works. Nothing else.

### Tasks

1. Create `crates/cv-tts`, add to workspace `Cargo.toml`.
2. Add to `cv-tts/Cargo.toml`:
   ```toml
   [dependencies.windows]
   features = [
     "Win32_System_Com",
     "Win32_Media_Speech",
   ]
   ```
3. Spawn TTS thread from `app/src/main.rs` before the egui event loop.
4. TTS thread body:
   - `CoInitializeEx(None, COINIT_MULTITHREADED)` — first line, no exceptions
   - `CoCreateInstance::<ISpVoice>` with CLSID `SpVoice`
   - `voice.SetVolume(80)`, `voice.SetRate(0)`
   - `voice.Speak(w!("clear-view ready"), SPF_ASYNC, None)`
   - Loop: sleep 100ms, check shutdown flag, exit when set
5. Shutdown: `Arc<AtomicBool>` shutdown flag, set on app exit, thread exits loop.

### Verify

Hear "clear-view ready" on launch. App opens and closes without hang or panic.

### Notes

- `CoInitializeEx` must be the first COM call. If anything touches COM before it
  on this thread, UIA will silently use STA later and misbehave.
- Do not call `CoUninitialize` until all COM objects on the thread have been dropped.

---

## Stage 2 — UIA Bootstrap + Hover Logging

**Goal:** Poll `ElementFromPoint` every 250ms. Log element name to stdout only.
No speech yet. Proves UIA is working before wiring anything up.

### Tasks

1. Add to `cv-tts/Cargo.toml`:
   ```toml
   "Win32_UI_Accessibility"
   ```
2. After SAPI init on TTS thread, create UIA:
   ```rust
   CoCreateInstance::<IUIAutomation>(&CLSID_CUIAutomation8, ...)
   ```
3. Poll loop, 250ms sleep:
   - `GetPhysicalCursorPos(&mut pt)` — never `GetCursorPos`
   - `automation.ElementFromPoint(pt)` — handle `UIA_E_ELEMENTNOTAVAILABLE`
     gracefully (log and `continue`, not `unwrap`)
   - Log `element.CurrentName()` and `element.CurrentControlType()` to stdout
   - Skip if name is empty or same as previous

### Verify

Move cursor over taskbar, Start, Notepad, a browser — sensible names in stdout.
No crashes on empty/unavailable elements.

### Notes

- Always `CLSID_CUIAutomation8`, not `CLSID_CUIAutomation`. The `8` variant gives
  correct TextPattern for Notepad (Document control type vs the old Edit control).
- `GetPhysicalCursorPos` is mandatory. At 125% DPI `GetCursorPos` returns scaled
  logical coordinates and `ElementFromPoint` will return the wrong element.
- `ElementFromPoint` can block 20–100ms on browsers. At 250ms on a dedicated thread
  this never affects the render loop.

---

## Stage 3 — Wire Hover into SAPI

**Goal:** Speak the hovered element name. Add egui controls. Toggle works.

### Tasks

1. Thread receives `Arc<RwLock<AppState>>`.
2. Each poll tick: check `tts_enabled`, skip if false.
3. On new non-empty element name: call
   `voice.Speak(name, SPF_ASYNC | SPF_PURGEBEFORESPEAK, None)`.
4. Apply `tts_volume` and `tts_rate` from AppState (use a dirty flag to avoid setting
   them on every tick).
5. Suppress hover speech when a text-bearing element has focus: check
   `automation.GetFocusedElement()` → try `GetCurrentPatternAs::<IUIAutomationTextPattern>`
   → if present, skip hover speech.
6. Add egui controls: toggle, volume slider, rate slider.

### Verify

Move over UI elements — hear names. Toggle off — silence. Sliders change voice.
No speech while typing in Notepad.

---

## Stage 4 — Focus Change Detection

**Goal:** Detect when focus enters/leaves a text element. Switch state machine.
No reading yet — just the transition, logged to stdout.

### Tasks

1. Subscribe to `UIA_AutomationFocusChangedEventId` via
   `IUIAutomation::AddFocusChangedEventHandler` — called on the TTS thread.
2. In the handler:
   - Try `GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)`.
   - If present → send `Msg::TextFocusGained` over mpsc channel.
   - If absent → send `Msg::TextFocusLost`.
3. TTS thread receives messages and updates internal mode.
4. Log mode transitions to stdout.

### Notes

- Handlers fire on a UIA thread. Send strings/booleans over the channel — never COM
  pointers. Do all COM work inside the handler; the handler thread is COM-safe.
- `AddFocusChangedEventHandler` is system-wide. It fires for all windows including
  the egui settings panel — filter out events from our own process HWND if noisy.

---

## Stage 5 — Selection Reading

**Goal:** When text is selected anywhere, speak it.

### Tasks

1. When `TextFocusGained` is received, subscribe to
   `UIA_Text_TextSelectionChangedEventId` on the focused element with
   `TreeScope_Element`.
2. In handler, when selection is non-degenerate (length > 0):
   - `IUIAutomationTextPattern::GetSelection()` → `IUIAutomationTextRangeArray`
   - `array.GetElement(0)` → `range.GetText(-1)` — cap at 500 chars, speak
     "…and more" if exceeded
   - Send text over channel → TTS thread speaks with `SPF_ASYNC | SPF_PURGEBEFORESPEAK`
3. Unsubscribe on `TextFocusLost`.

### Verify

Select text in Notepad, Word, browser address bar — hear it. Deselect — silence.
New selection interrupts previous speech.

---

## Stage 6 — Caret Following

**Goal:** Arrow key navigation in text speaks the word (or character) at the new
caret position.

### Tasks

1. Same `UIA_Text_TextSelectionChangedEventId` subscription as Stage 5.
2. When selection IS degenerate (length 0 = caret move):
   - Try `IUIAutomationTextPattern2::GetCaretRange(&is_active, &range)`.
   - If `is_active`, expand to `TextUnit_Word` or `TextUnit_Character` per
     `tts_granularity`.
   - Trim whitespace. Send over channel → speak.
   - If `TextPattern2` unavailable, fall back to `TextPattern::GetSelection()[0]`.
3. Add granularity radio to egui (Word / Character).

### Notes

- `ExpandToEnclosingUnit` modifies the range object in-place.
- Word expansion includes trailing whitespace — trim before speaking.
- A caret move and a zero-length selection are the same event. Distinguish by
  checking whether the selection length is 0.

---

## Stage 7 — Typing Echo

**Goal:** Speak each typed word when the user hits space or punctuation. In character
mode, speak each character.

### Tasks

1. Subscribe to `UIA_Text_TextChangedEventId` on the focused element.
2. On event: read full text from `TextPattern::GetDocumentRange().GetText(-1)`.
   Diff against previous snapshot to find what was inserted.
3. If inserted text ends with a word boundary (space, `.`, `,`, `!`, `?`, `;`, `:`,
   newline): speak the last completed word.
4. If `tts_char_mode == true`: speak each inserted character by name
   ("space", "period", "enter", "a", "b", etc.).
5. Suppress if selection was non-degenerate before the change (typing over selection —
   Selection mode takes priority).
6. For pastes (>3 words inserted at once): speak "pasted" rather than content.
7. Add character mode checkbox to egui.

### Notes

- `TextChangedEvent` does not describe what changed. You must diff against a cached
  previous text snapshot.
- Do not call `GetText(-1)` on very large documents — cap at the last 200 chars for
  the diff.
- Backspace: inserted text will be empty but the snapshot will be shorter. Speak
  "deleted" or the deleted character if in character mode.

---

## Stage 8 — AppReader (Continuous Reading)

**Goal:** A hotkey starts continuous reading of the focused document from the word
under the pointer, advancing automatically word by word. A second press or Escape
stops it. This is ZoomText's AppReader equivalent.

### How it works

AppReader is caret following on autopilot. Instead of waiting for the user to press
an arrow key, the TTS thread advances the caret position itself after each word is
spoken, using SAPI's word-boundary status to know when to advance.

### Tasks

1. Add `AppReading` as a new TTS mode alongside `Hovering` and `TextFocus`:
   ```
   AppReading(element, current_range)
     → SAPI word spoken   → advance range by one word, speak next
     → hotkey / Escape    → stop, return to previous mode
     → focus lost         → stop
   ```
2. On AppReader hotkey (registered in existing hotkey thread, new `Msg::AppReaderToggle`
   variant):
   - Get element under cursor via `AccessibilityBackend::element_at_point`.
   - If it has a TextPattern, get the text range at cursor via
     `TextPattern::RangeFromPoint(pt)`.
   - Expand to word boundary → speak first word → enter `AppReading` mode.
3. Advance logic: poll `ISpVoice::GetStatus` every 50ms while in AppReading mode.
   When `dwRunningState == SPRS_DONE`, call `range.Move(TextUnit_Word, 1)` →
   `range.GetText(-1)` → speak next word. Stop when `Move` returns 0 (end of doc).
4. Stop on: second hotkey press, Escape (`WH_KEYBOARD_LL` checking VK_ESCAPE only
   while in AppReading mode), focus change, or end of document.
5. Pause/resume: first hotkey press while reading = pause (stop SAPI, hold position).
   Second press = resume from held position. Third press = stop entirely.
6. Add configurable hotkey slot to AppState — do not hardcode:
   ```rust
   pub hotkey_appreader: Option<HotkeyBinding>, // default: None (unbound)
   ```

### Notes

- Advancement must be driven by SAPI completion, not a timer — otherwise fast/slow
  rate settings cause words to be skipped or doubled.
- AppReader works in any UIA TextPattern app (browsers, Word, Notepad). Will not work
  in LibreOffice until Stage 9 (IA2).
- The `HotkeyBinding` type should be added to `cv-core` as a stub now even if unused,
  so future hotkey stages have a defined home.

---

## Stage 9 — IAccessible2 (LibreOffice + Firefox)

**Context:**
LibreOffice on Windows exposes document content exclusively via **IAccessible2 (IA2)**,
not UIA. Stages 5–8 silently do nothing in LibreOffice Writer without this stage.
Firefox also uses IA2, though it has an in-progress UIA implementation — check its
status before implementing the Firefox path.

### Detection

Detect IA2 apps by window class name via `GetForegroundWindow` + `GetClassName`:
- LibreOffice: `SALFRAME`
- Firefox: `MozillaWindowClass`

### IA2 access pattern

1. `AccessibleObjectFromPoint(pt)` → `IAccessible` pointer (oleacc, already enabled).
2. `QueryInterface` for `IAccessible2` using GUID
   `{E89F726E-C4F4-4c19-BB19-B647D7FA8478}`.
3. If present: read `accName`, `accRole`.
4. `QueryInterface` for `IAccessibleText` → `caretOffset`,
   `getTextAtOffset(caret, BOUNDARY_WORD)` for caret following.
5. For selection: `IAccessibleText::nSelections`, `IAccessibleText::selection(0)`.
6. For change notification: `SetWinEventHook` for `EVENT_OBJECT_VALUECHANGE` and
   `EVENT_OBJECT_TEXTSELECTIONCHANGED` on the target process.

### IA2 bindings

The `windows` crate does not include IA2. Hand-roll `windows::core::Interface` impls
for just `IAccessible2` and `IAccessibleText` using vtable layouts from:
https://github.com/LinuxA11y/IAccessible2

Only implement the methods needed — `accName`, `accRole`, `caretOffset`,
`getTextAtOffset`, `nSelections`, `selection`. Leave the rest as stubs.

### IA2 proxy registration

`IAccessible2Proxy.dll` must be registered on the system. Present on any machine that
has had a screen reader installed. If absent, `QueryInterface` for `IAccessible2` will
fail silently — fall back to UIA gracefully.

---

## Dead Ends — Do Not Retry

- **`GetCursorPos` for UIA** — logical coords, wrong element at non-100% DPI.
  Always `GetPhysicalCursorPos`.
- **STA threading** — must be MTA. STA causes marshalling failures.
- **`AddEventHandler` from multiple threads** — one thread, always.
- **Synchronous `ISpVoice::Speak`** — blocks. Always `SPF_ASYNC`.
- **`CLSID_CUIAutomation` without `8`** — wrong TextPattern on Notepad and others.
- **UIA from render or egui thread** — TTS thread only.
- **COM pointers across threads** — send strings over channel instead.
- **`TextPattern2::GetCaretRange` without fallback** — not available everywhere.
- **LibreOffice UIA TextPattern for document body** — does not exist. IA2 required.
- **`GetText(-1)` on large documents** — cap at reasonable limit (500 chars for
  speech, 200 chars for diffs).

---

## Build Notes

```
RUSTFLAGS="-Awarnings" cargo build 2>&1 | grep -E "^error\["
```

Windows only. `windows` crate v0.62. Rust 2024 edition.
Add only the `windows` features needed per stage — keeps compile times short.
Never merge to master without running the app and confirming audio output.