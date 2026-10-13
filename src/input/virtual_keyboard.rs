use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsFd;
use std::thread;
use std::time::{Duration, Instant};
use wayland_client::{
    delegate_noop,
    protocol::{wl_keyboard, wl_registry, wl_seat},
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

// Linux evdev keys that normally produce text. Do not allocate Unicode to
// Esc, Backspace, Tab, Enter, modifiers or input-method switching keys.
const TEXT_KEYS: &[u32] = &[
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 57,
];

#[derive(Default)]
struct Globals {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" if state.manager.is_none() => {
                    state.manager = Some(registry.bind(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(Globals: ignore wl_seat::WlSeat);
delegate_noop!(Globals: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(Globals: ignore ZwpVirtualKeyboardV1);

pub(super) struct VirtualKeyboard {
    queue: EventQueue<Globals>,
    globals: Globals,
    keyboard: ZwpVirtualKeyboardV1,
    started: Instant,
}

impl VirtualKeyboard {
    pub(super) fn connect() -> Result<Self> {
        let connection = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let mut queue = connection.new_event_queue();
        let qh = queue.handle();
        let _registry = connection.display().get_registry(&qh, ());
        let mut globals = Globals::default();
        queue
            .roundtrip(&mut globals)
            .context("Failed to discover Wayland globals")?;
        let seat = globals.seat.as_ref().context("No Wayland seat found")?;
        let manager = globals
            .manager
            .as_ref()
            .context("Compositor does not support the virtual keyboard protocol")?;
        let keyboard = manager.create_virtual_keyboard(seat, &qh, ());
        Ok(Self {
            queue,
            globals,
            keyboard,
            started: Instant::now(),
        })
    }

    pub(super) fn type_text(&mut self, text: &str) -> Result<()> {
        // Validate the whole segment before sending anything. Keep this
        // keyboard alive across segments: destroying it can reset the keymap
        // before XWayland clients finish consuming queued key events.
        for batch in plan(text)? {
            let keymap = batch.keymap();
            let mut file = tempfile::tempfile().context("Failed to create keymap file")?;
            file.write_all(keymap.as_bytes())?;
            self.keyboard.keymap(1, file.as_fd(), keymap.len() as u32);
            self.keyboard.modifiers(0, 0, 0, 0);
            self.roundtrip()?;
            for code in batch.keys {
                self.keyboard.key(
                    self.timestamp(),
                    code,
                    wl_keyboard::KeyState::Pressed as u32,
                );
                self.roundtrip()?;
                thread::sleep(Duration::from_millis(2));
                self.keyboard.key(
                    self.timestamp(),
                    code,
                    wl_keyboard::KeyState::Released as u32,
                );
                self.roundtrip()?;
                thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }

    fn timestamp(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    fn roundtrip(&mut self) -> Result<()> {
        self.queue
            .roundtrip(&mut self.globals)
            .context("Virtual keyboard roundtrip failed")?;
        Ok(())
    }
}

#[derive(Default)]
struct Batch {
    mapping: Vec<(char, u32)>,
    keys: Vec<u32>,
}

impl Batch {
    fn keymap(&self) -> String {
        let mut map = String::from(
            "xkb_keymap {\nxkb_keycodes \"audio2text\" { minimum = 8; maximum = 65;\n",
        );
        for (_, code) in &self.mapping {
            map.push_str(&format!("<K{code}> = {};\n", code + 8));
        }
        map.push_str("};\nxkb_types \"audio2text\" { include \"complete\" };\nxkb_compatibility \"audio2text\" { include \"complete\" };\nxkb_symbols \"audio2text\" {\n");
        for (ch, code) in &self.mapping {
            let keysym = match ch {
                '\n' => 0xff0d, // Only a real newline is assigned KEY_ENTER.
                '\t' => 0xff09, // Only a real tab is assigned KEY_TAB.
                ch if (*ch as u32) <= 0xff => *ch as u32,
                ch => 0x01000000 | *ch as u32,
            };
            map.push_str(&format!(
                "key <K{code}> {{ type=\"ONE_LEVEL\", [0x{keysym:08x}] }};\n"
            ));
        }
        map.push_str("};\n};\n\0");
        map
    }
}

fn plan(text: &str) -> Result<Vec<Batch>> {
    let mut batches = Vec::new();
    let mut batch = Batch::default();
    let mut codes = HashMap::new();
    let mut allocated = 0;
    for ch in text.chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            bail!("Refusing to inject control character U+{:04X}", ch as u32);
        }
        let code = if let Some(code) = codes.get(&ch) {
            *code
        } else {
            if allocated == TEXT_KEYS.len() && ch != '\n' && ch != '\t' {
                batches.push(batch);
                batch = Batch::default();
                codes.clear();
                allocated = 0;
            }
            let code = match ch {
                '\n' => 28,
                '\t' => 15,
                _ => {
                    let code = TEXT_KEYS[allocated];
                    allocated += 1;
                    code
                }
            };
            codes.insert(ch, code);
            batch.mapping.push((ch, code));
            code
        };
        batch.keys.push(code);
    }
    if !batch.keys.is_empty() {
        batches.push(batch);
    }
    Ok(batches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(text: &str) {
        let batches = plan(text).unwrap();
        let mut decoded = String::new();
        for batch in batches {
            for (ch, code) in &batch.mapping {
                match ch {
                    '\n' => assert_eq!(*code, 28),
                    '\t' => assert_eq!(*code, 15),
                    _ => assert!(TEXT_KEYS.contains(code), "unsafe keycode {code}"),
                }
            }
            assert!(batch.keymap().ends_with('\0'));
            for code in batch.keys {
                decoded.push(
                    batch
                        .mapping
                        .iter()
                        .find(|(_, mapped)| *mapped == code)
                        .unwrap()
                        .0,
                );
            }
        }
        assert_eq!(decoded, text);
    }

    #[test]
    fn preserves_unicode_repetition_and_boundaries() {
        for text in [
            "",
            "首字测试首字",
            "你好 hello zzz 🙂𠮷",
            "第一行\n第二行\t结束",
            "-k VoidSymbol",
        ] {
            roundtrip(text);
        }
        let batch = &plan("首字测试").unwrap()[0];
        assert_eq!(batch.keys[0], 2);
        assert_eq!(batch.keys[1], 3);
        assert!(batch.keymap().contains("0x01009996"));
    }

    #[test]
    fn long_text_reuses_only_printable_keycodes() {
        let text: String = (0x4e00..0x4f00)
            .map(|value| char::from_u32(value).unwrap())
            .collect();
        assert!(plan(&text).unwrap().len() > 1);
        roundtrip(&format!("{text}{text}\n结束"));
    }

    #[test]
    fn rejects_control_keys_before_injecting_any_text() {
        for ch in ['\0', '\u{1b}', '\u{8}', '\u{7f}', '\r'] {
            assert!(plan(&format!("开头{ch}结尾")).is_err());
        }
    }

    #[test]
    fn printable_pool_has_no_control_or_modifier_keys() {
        let mut unique = std::collections::HashSet::new();
        for code in TEXT_KEYS {
            assert!(unique.insert(*code));
            assert!(
                ![1, 14, 15, 28, 29, 42, 54, 56, 58, 97, 100, 122, 123, 125, 126].contains(code)
            );
        }
    }

    #[test]
    #[ignore = "requires a Wayland session exposing the virtual keyboard protocol; sends no key events"]
    fn connects_to_virtual_keyboard_without_typing() {
        let mut keyboard = VirtualKeyboard::connect().unwrap();
        keyboard.roundtrip().unwrap();
    }

    #[test]
    #[ignore = "requires xkbcli to validate generated keymaps with libxkbcommon"]
    fn keymaps_compile_with_xkbcommon() {
        use std::process::{Command, Stdio};
        let text: String = (0x4e00..0x4f00)
            .map(|value| char::from_u32(value).unwrap())
            .collect();
        for batch in plan(&format!("首字测试 zzz 🙂𠮷\n\t{text}")).unwrap() {
            let mut child = Command::new("xkbcli")
                .args([
                    "compile-keymap",
                    "--from-xkb",
                    "-",
                    "--test",
                    "--input-format",
                    "1",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let keymap = batch.keymap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(keymap.trim_end_matches('\0').as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
