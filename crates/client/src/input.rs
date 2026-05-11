//! Unified stdin input pipeline.
//!
//! A single crossterm `EventStream` reads keyboard events; both the menu
//! (which consumes `KeyEvent` directly) and the PTY relay (which converts
//! `KeyEvent` back into raw byte sequences) read from the same channel, so
//! ownership of stdin is never contended.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use tokio::sync::mpsc;

pub type KeyRx = mpsc::Receiver<KeyEvent>;

const KEY_QUEUE: usize = 256;

pub fn spawn_reader() -> KeyRx {
    let (tx, rx) = mpsc::channel::<KeyEvent>(KEY_QUEUE);
    tokio::spawn(async move {
        let mut events = EventStream::new();
        while let Some(item) = events.next().await {
            match item {
                Ok(Event::Key(k)) => {
                    if tx.send(k).await.is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    rx
}

/// Translate a crossterm KeyEvent back into the raw byte sequence a terminal
/// would normally produce, so the PTY relay can hand them through to claude
/// unchanged. Covers the common keys (control characters, arrows, function
/// keys, alt-prefixed). Unknown keys → None (dropped).
pub fn key_event_to_bytes(k: KeyEvent) -> Option<Vec<u8>> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let bytes: Vec<u8> = match k.code {
        KeyCode::Char(c) => {
            if ctrl {
                let lc = c.to_ascii_lowercase();
                if lc.is_ascii_lowercase() {
                    vec![(lc as u8) - b'a' + 1]
                } else if c == ' ' {
                    vec![0]
                } else if c == '[' {
                    vec![0x1b]
                } else if c == '\\' {
                    vec![0x1c]
                } else if c == ']' {
                    vec![0x1d]
                } else if c == '^' {
                    vec![0x1e]
                } else if c == '_' {
                    vec![0x1f]
                } else {
                    vec![c as u8]
                }
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => return None,
        },
        _ => return None,
    };
    if alt {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.push(0x1b);
        out.extend_from_slice(&bytes);
        Some(out)
    } else {
        Some(bytes)
    }
}
