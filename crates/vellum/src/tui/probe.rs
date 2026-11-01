//! Asking the terminal which graphics protocol it speaks.
//!
//! ratatui-image's own probe restarts its timeout on every byte and keeps
//! reading stdin after it gives up, so on a terminal that never answers,
//! typing at startup could stall it or lose keys. This one sends the same
//! query, but bounds the whole probe, and hands back every key typed
//! meanwhile so the TUI still sees it.

use std::time::{Duration, Instant};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_image::FontSize;
use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions, Response};
use ratatui_image::picker::{Picker, ProtocolType};

/// How long the whole probe may take. Real terminals answer within
/// milliseconds; one that stays silent costs this once, at startup.
const BUDGET: Duration = Duration::from_secs(1);

/// Probes the terminal on stdin/stdout, which must be in raw mode with
/// nothing else reading stdin yet. Returns the best Picker it answers for
/// (halfblocks otherwise) and the keys typed meanwhile. `VELLUM_GRAPHICS`
/// overrides the protocol for terminals that claim more than they draw.
pub fn picker() -> (Picker, Vec<KeyEvent>) {
    use std::io::Write as _;
    let forced = forced_by_env();
    if forced == Some(ProtocolType::Halfblocks) {
        return (Picker::halfblocks(), Vec::new());
    }
    let is_tmux = std::env::var("TERM").is_ok_and(|term| term.starts_with("tmux"))
        || std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "tmux");
    let mut options = QueryStdioOptions::default();
    // As ratatui-image does: their kitty placeholders and (Konsole) sixels
    // are broken, so do not ask.
    let set = |name| std::env::var(name).is_ok_and(|v| !v.is_empty());
    if set("WEZTERM_EXECUTABLE") || set("KONSOLE_VERSION") {
        options.blacklist_protocols = vec![ProtocolType::Kitty, ProtocolType::Sixel];
    }
    let mut stdout = std::io::stdout();
    let sent = stdout
        .write_all(Parser::query(is_tmux, options).as_bytes())
        .and_then(|()| stdout.flush());
    if sent.is_err() {
        return (Picker::halfblocks(), Vec::new());
    }
    let answers = collect(read_stdin, BUDGET);
    let mut picker = choose(&answers.responses);
    if let Some(protocol) = forced {
        picker.set_protocol_type(protocol);
    }
    (picker, answers.keys)
}

/// The protocol the user forced through `VELLUM_GRAPHICS`, if any.
pub fn forced_by_env() -> Option<ProtocolType> {
    forced(std::env::var("VELLUM_GRAPHICS").ok().as_deref())
}

/// The protocol `VELLUM_GRAPHICS` asks for: `halfblocks`, `kitty`, `sixel`
/// or `iterm2`. Anything else, including `auto` or unset, means probe.
fn forced(value: Option<&str>) -> Option<ProtocolType> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "halfblocks" => Some(ProtocolType::Halfblocks),
        "kitty" => Some(ProtocolType::Kitty),
        "sixel" => Some(ProtocolType::Sixel),
        "iterm2" => Some(ProtocolType::Iterm2),
        _ => None,
    }
}

/// The bytes waiting on stdin within `wait`.
fn read_stdin(wait: Duration) -> std::io::Result<Vec<u8>> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let stdin = std::io::stdin();
    let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
    let timeout = Timespec::try_from(wait).unwrap_or(Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    });
    match poll(&mut fds, Some(&timeout)) {
        Ok(0) | Err(rustix::io::Errno::INTR) => return Ok(Vec::new()),
        Ok(_) => {}
        Err(err) => return Err(err.into()),
    }
    let mut buf = [0u8; 256];
    match rustix::io::read(&stdin, &mut buf) {
        Ok(0) => Err(std::io::ErrorKind::UnexpectedEof.into()),
        Ok(n) => Ok(buf[..n].to_vec()),
        Err(rustix::io::Errno::INTR) => Ok(Vec::new()),
        Err(err) => Err(err.into()),
    }
}

/// The Picker for what the terminal answered: kitty over sixel, else what
/// the environment hints at, else halfblocks. Images need the cell size:
/// from the answer, or from the pty's pixel size.
fn choose(responses: &[Response]) -> Picker {
    let cell = responses.iter().find_map(|r| match r {
        Response::CellSize(Some((w, h))) => Some((*w, *h)),
        _ => None,
    });
    let Some((width, height)) = cell.or_else(cell_from_winsize) else {
        return Picker::halfblocks();
    };
    // Deprecated in favor of ratatui-image's own probe, which this replaces;
    // it still reads the tmux and iTerm2 hints from the environment.
    #[allow(deprecated)]
    let mut picker = Picker::from_fontsize(FontSize::new(width, height));
    if responses.contains(&Response::Kitty) {
        picker.set_protocol_type(ProtocolType::Kitty);
    } else if responses.contains(&Response::Sixel) {
        picker.set_protocol_type(ProtocolType::Sixel);
    }
    picker
}

fn cell_from_winsize() -> Option<(u16, u16)> {
    let size = rustix::termios::tcgetwinsize(std::io::stdout()).ok()?;
    if size.ws_xpixel == 0 || size.ws_ypixel == 0 || size.ws_col == 0 || size.ws_row == 0 {
        return None;
    }
    Some((size.ws_xpixel / size.ws_col, size.ws_ypixel / size.ws_row))
}

/// What came back from the terminal during the probe.
#[derive(Debug, Default)]
pub struct Answers {
    /// The terminal's replies, in order.
    pub responses: Vec<Response>,
    /// Keys the user typed during the probe, in order.
    pub keys: Vec<KeyEvent>,
    /// Whether the terminal answered the final status report.
    pub done: bool,
}

/// Reads the terminal's answers through `read`, which returns the bytes
/// available within the given time (empty when none came). Stops at the
/// status report or once `budget` is spent, whatever the user types.
pub fn collect(
    mut read: impl FnMut(Duration) -> std::io::Result<Vec<u8>>,
    budget: Duration,
) -> Answers {
    let deadline = Instant::now() + budget;
    let mut answers = Answers::default();
    let mut input = Input::default();
    while !answers.done {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match read(left) {
            Ok(bytes) => input.push(&bytes, &mut answers),
            Err(_) => break,
        }
    }
    input.finish(&mut answers);
    answers
}

/// Splits what the terminal sends into its replies and the user's keys,
/// across reads.
#[derive(Default)]
struct Input {
    /// The start of a sequence or character the next read completes.
    pending: Vec<u8>,
    parser: Parser,
}

impl Input {
    fn push(&mut self, bytes: &[u8], answers: &mut Answers) {
        self.pending.extend_from_slice(bytes);
        let mut at = 0;
        while let Some((len, token)) = token(&self.pending[at..]) {
            match token {
                Token::Reply => {
                    for &byte in &self.pending[at..at + len] {
                        for response in self.parser.push(char::from(byte)) {
                            if response == Response::Status {
                                answers.done = true;
                            } else {
                                answers.responses.push(response);
                            }
                        }
                    }
                }
                Token::Key(key) => answers.keys.push(key),
                Token::Ignored => {}
            }
            at += len;
        }
        self.pending.drain(..at);
    }

    /// Whatever is left when the probe ends: a lone ESC was the Esc key;
    /// half a sequence is dropped.
    fn finish(&mut self, answers: &mut Answers) {
        if self.pending == [ESC] {
            answers.keys.push(plain(KeyCode::Esc));
        }
        self.pending.clear();
    }
}

const ESC: u8 = 0x1b;

enum Token {
    /// Part of the terminal's answer.
    Reply,
    Key(KeyEvent),
    /// A key the probe does not decode, or a stray byte.
    Ignored,
}

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// The first token in `bytes` and its length, or `None` when `bytes` is
/// empty or holds only the beginning of one.
fn token(bytes: &[u8]) -> Option<(usize, Token)> {
    let first = *bytes.first()?;
    if first != ESC {
        return char_key(bytes);
    }
    // A lone ESC: the Esc key, or a sequence still arriving.
    let kind = *bytes.get(1)?;
    match kind {
        b'[' => {
            let end = 2 + bytes[2..].iter().position(|b| (0x40..=0x7e).contains(b))?;
            let token = match bytes[end] {
                // Device attributes, cell size, status report.
                b'c' | b't' | b'n' => Token::Reply,
                last => csi_key(&bytes[2..end], last).map_or(Token::Ignored, Token::Key),
            };
            Some((end + 1, token))
        }
        // OSC replies end with BEL or ST; APC (kitty) and DCS with ST.
        b']' | b'_' | b'P' => {
            let body = &bytes[2..];
            let st = body
                .windows(2)
                .position(|w| w == [ESC, b'\\'])
                .map(|i| i + 2);
            let bel = body.iter().position(|&b| b == 0x07).map(|i| i + 1);
            let end = match (kind, bel, st) {
                (b']', Some(bel), Some(st)) => bel.min(st),
                (b']', Some(bel), None) => bel,
                (_, _, st) => st?,
            };
            Some((2 + end, Token::Reply))
        }
        b'O' => {
            let code = match *bytes.get(2)? {
                b'P' => Some(KeyCode::F(1)),
                b'Q' => Some(KeyCode::F(2)),
                b'R' => Some(KeyCode::F(3)),
                b'S' => Some(KeyCode::F(4)),
                other => csi_key(&[], other).map(|key| key.code),
            };
            Some((
                3,
                code.map_or(Token::Ignored, |code| Token::Key(plain(code))),
            ))
        }
        // ESC ESC: the first one was the Esc key.
        ESC => Some((1, Token::Key(plain(KeyCode::Esc)))),
        // Alt held with a key.
        _ => {
            let (len, token) = char_key(&bytes[1..])?;
            let token = match token {
                Token::Key(mut key) => {
                    key.modifiers |= KeyModifiers::ALT;
                    Token::Key(key)
                }
                other => other,
            };
            Some((1 + len, token))
        }
    }
}

/// A plain (possibly multi-byte) character, as crossterm reports it.
fn char_key(bytes: &[u8]) -> Option<(usize, Token)> {
    let first = bytes[0];
    let len = match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => return Some((1, Token::Ignored)),
    };
    let Ok(text) = std::str::from_utf8(bytes.get(..len)?) else {
        return Some((1, Token::Ignored));
    };
    let c = text.chars().next().expect("one character");
    let token = match c {
        '\r' | '\n' => Token::Key(plain(KeyCode::Enter)),
        '\t' => Token::Key(plain(KeyCode::Tab)),
        '\x7f' | '\x08' => Token::Key(plain(KeyCode::Backspace)),
        '\x01'..='\x1a' => Token::Key(KeyEvent::new(
            KeyCode::Char(char::from(b'a' + first - 1)),
            KeyModifiers::CONTROL,
        )),
        c if c.is_control() => Token::Ignored,
        c if c.is_uppercase() => Token::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)),
        c => Token::Key(plain(KeyCode::Char(c))),
    };
    Some((len, token))
}

/// A CSI key sequence (`ESC [ params last`): arrows, Home/End and the `~`
/// keys, with xterm's modifier parameter.
fn csi_key(params: &[u8], last: u8) -> Option<KeyEvent> {
    let params = std::str::from_utf8(params).ok()?;
    let mut fields = params.split(';');
    let number = fields.next().unwrap_or("");
    let bits = fields
        .next()
        .and_then(|m| m.parse::<u8>().ok())
        .map_or(0, |m| m.saturating_sub(1));
    let mut modifiers = KeyModifiers::NONE;
    for (bit, modifier) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
    ] {
        if bits & bit != 0 {
            modifiers |= modifier;
        }
    }
    let code = match last {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'Z' => return Some(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        b'~' => match number {
            "1" | "7" => KeyCode::Home,
            "2" => KeyCode::Insert,
            "3" => KeyCode::Delete,
            "4" | "8" => KeyCode::End,
            "5" => KeyCode::PageUp,
            "6" => KeyCode::PageDown,
            _ => return None,
        },
        _ => return None,
    };
    Some(KeyEvent::new(code, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &[u8] = b"\x1b[6;20;10t\x1b[?62;4;22c\x1b[0n";

    #[test]
    fn vellum_graphics_names_a_protocol_and_anything_else_means_auto() {
        assert_eq!(forced(Some("halfblocks")), Some(ProtocolType::Halfblocks));
        assert_eq!(forced(Some("Kitty")), Some(ProtocolType::Kitty));
        assert_eq!(forced(Some(" sixel ")), Some(ProtocolType::Sixel));
        assert_eq!(forced(Some("iterm2")), Some(ProtocolType::Iterm2));
        assert_eq!(forced(Some("auto")), None);
        assert_eq!(forced(Some("")), None);
        assert_eq!(forced(Some("vt340")), None);
        assert_eq!(forced(None), None);
    }

    fn feed(chunks: &[&[u8]]) -> Answers {
        let mut chunks = chunks
            .iter()
            .map(|c| c.to_vec())
            .collect::<Vec<_>>()
            .into_iter();
        collect(
            move |wait| {
                Ok(chunks.next().unwrap_or_else(|| {
                    std::thread::sleep(wait);
                    Vec::new()
                }))
            },
            Duration::from_millis(100),
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn a_terminal_that_answers_reports_its_cell_size_and_sixel() {
        let answers = feed(&[ANSWER]);
        assert!(answers.done);
        assert!(answers.keys.is_empty());
        assert!(
            answers
                .responses
                .contains(&Response::CellSize(Some((10, 20))))
        );
        assert!(answers.responses.contains(&Response::Sixel));
    }

    #[test]
    fn keys_typed_around_the_answer_are_kept_in_order() {
        let answers = feed(&[b"j", b"\x1b[6;20;10t", b"k\r", b"\x1b[?62c\x1b[0n", b"q"]);
        assert!(answers.done);
        assert_eq!(
            answers.keys,
            [
                key(KeyCode::Char('j')),
                key(KeyCode::Char('k')),
                key(KeyCode::Enter)
            ]
        );
        assert!(
            answers
                .responses
                .contains(&Response::CellSize(Some((10, 20))))
        );
    }

    #[test]
    fn answers_split_across_reads_still_parse() {
        let (a, b) = ANSWER.split_at(5);
        let answers = feed(&[a, b]);
        assert!(answers.done);
        assert!(answers.keys.is_empty());
        assert!(
            answers
                .responses
                .contains(&Response::CellSize(Some((10, 20))))
        );
    }

    #[test]
    fn arrows_control_keys_and_escape_typed_during_the_probe_survive() {
        let answers = feed(&[b"\x1b[B\x1bOA\x03G\x7f\t", b"\x1b"]);
        assert!(!answers.done);
        assert_eq!(
            answers.keys,
            [
                key(KeyCode::Down),
                key(KeyCode::Up),
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
                key(KeyCode::Backspace),
                key(KeyCode::Tab),
                key(KeyCode::Esc),
            ]
        );
    }

    #[test]
    fn typing_at_a_silent_terminal_cannot_stretch_the_probe() {
        // A key every 20 ms, forever, and never an answer.
        let start = Instant::now();
        let answers = collect(
            |wait| {
                std::thread::sleep(wait.min(Duration::from_millis(20)));
                Ok(b"x".to_vec())
            },
            Duration::from_millis(200),
        );
        let took = start.elapsed();
        assert!(took < Duration::from_millis(400), "took {took:?}");
        assert!(!answers.done);
        assert!(answers.keys.len() >= 5, "{:?}", answers.keys);
        assert!(answers.keys.iter().all(|k| *k == key(KeyCode::Char('x'))));
    }

    #[test]
    fn utf8_typed_during_the_probe_is_one_key() {
        let answers = feed(&["é".as_bytes(), ANSWER]);
        assert_eq!(answers.keys, [key(KeyCode::Char('é'))]);
    }
}
