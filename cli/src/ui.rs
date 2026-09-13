//! How the wallet draws.
//!
//! One place for the frames, rules, columns and colours, so that fifty-three
//! command modules stop each inventing their own. Nothing here is required —
//! the thousand existing `tprintln!` calls are untouched — but everything new
//! should come through here, or the wallet goes on looking like fifty-three
//! people wrote it.
//!
//! # It has to work in a browser
//!
//! The CLI is also the web wallet: `kaspa-cli` is a `cdylib` and
//! `kaspa-wallet-cli-wasm` runs the same code through xterm.js. So there is
//! no cursor addressing here, no alternate screen and no repainting — only
//! lines, written once, in the order they happen. That is not a limitation to
//! work around; it is what lets one wallet be a terminal program and a web
//! page without being written twice.
//!
//! Two things make that safe: `cols()` reports honestly through xterm.js, and
//! `set_colors_enabled(true)` is called at startup for both. So width-aware
//! framing and colour are portable, and nothing in this module needs a
//! native-only path.

use crate::imports::*;
use unicode_width::UnicodeWidthStr;

/// The brand's inks, taken from the note itself.
///
/// A Marigold note is deep green with a gold frame, an orange bloom shading
/// to pale petals at the centre, and cream text. The terminal uses the same
/// five so that the wallet and the thing it holds are recognisably one
/// object — which matters more here than in most software, because the note
/// is the product.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ink {
    /// The frame, and anything that is the note's own furniture.
    Gold,
    /// Outer petals, and the darker half of the bloom.
    Amber,
    /// Inner petals, the wordmark, amounts, and words worth the eye.
    Petal,
    /// Body text.
    Cream,
    /// Secondary text — the part skipped on a second reading.
    Moss,
    /// The microtext band, which is texture and is not meant to be read.
    Micro,
}

impl Ink {
    pub const fn rgb(self) -> (u8, u8, u8) {
        match self {
            Ink::Gold => (0xe4, 0xa3, 0x3d),
            Ink::Amber => (0xc9, 0x76, 0x1f),
            Ink::Petal => (0xf3, 0xcf, 0x82),
            Ink::Cream => (0xef, 0xe7, 0xd3),
            Ink::Moss => (0x9a, 0xa8, 0x94),
            // Moss carried most of the way to the ground it sits on. A real
            // note's microtext is meant to be seen and not read, and an
            // opacity cannot be expressed in a terminal, so it is mixed here.
            Ink::Micro => (0x5c, 0x66, 0x59),
        }
    }
}

/// How much colour this terminal can be asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    /// None at all: `NO_COLOR`, a pipe, or a terminal that said no.
    Plain,
    /// The 256-colour cube — what almost everything understands.
    Indexed,
    /// Twenty-four bit, so the brand's hues arrive exactly.
    True,
}

/// Worked out once. Nothing here changes while the program runs, and asking
/// the environment per printed character would be absurd.
static DEPTH: std::sync::OnceLock<Depth> = std::sync::OnceLock::new();

pub fn depth() -> Depth {
    *DEPTH.get_or_init(|| {
        // NO_COLOR is a promise to the user, and the wallet forces colour on
        // at startup because a browser cannot be sniffed for a terminal.
        // Honour the promise anyway: someone who set it meant it.
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return Depth::Plain;
        }
        if cfg!(target_arch = "wasm32") {
            // xterm.js renders 24-bit, and there is no environment to ask.
            return Depth::True;
        }
        // No terminal-detection beyond this. The wallet is interactive by
        // nature — it cannot run without something to type into — and the
        // browser has no tty to detect, so sniffing for one would turn the
        // web wallet monochrome for no reason. `NO_COLOR` above is the
        // deliberate way out.
        match std::env::var("COLORTERM").as_deref() {
            Ok("truecolor") | Ok("24bit") => Depth::True,
            _ => Depth::Indexed,
        }
    })
}

/// The nearest colour in the 256-colour palette.
///
/// Both halves of the palette are considered: the 6×6×6 cube, and the
/// twenty-four step grey ramp — cream lands much closer on the ramp than in
/// the cube, and picking from the cube alone turns it pink.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let distance = |x: i32, y: i32, z: i32| (r - x).pow(2) + (g - y).pow(2) + (b - z).pow(2);

    let index_of = |v: i32| LEVELS.iter().enumerate().min_by_key(|(_, l)| (v - **l).abs()).map(|(i, _)| i).unwrap();
    let (ri, gi, bi) = (index_of(r), index_of(g), index_of(b));
    let cube = 16 + 36 * ri + 6 * gi + bi;
    let cube_distance = distance(LEVELS[ri], LEVELS[gi], LEVELS[bi]);

    let grey_level = ((r + g + b) / 3 - 8).clamp(0, 238) / 10;
    let grey_value = 8 + grey_level * 10;
    let grey = 232 + grey_level;
    let grey_distance = distance(grey_value, grey_value, grey_value);

    if grey_distance < cube_distance { grey as u8 } else { cube as u8 }
}

/// Write `text` in `ink`, in whatever the terminal can actually show.
pub fn paint<S: AsRef<str>>(ink: Ink, text: S) -> String {
    let text = text.as_ref();
    let (r, g, b) = ink.rgb();
    match depth() {
        Depth::Plain => text.to_string(),
        Depth::True => format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[39m"),
        Depth::Indexed => format!("\x1b[38;5;{}m{text}\x1b[39m", nearest_256(r, g, b)),
    }
}

/// The same, bolder — for the wordmark and the denominations, which on a real
/// note are printed heavier than everything around them.
pub fn paint_bold<S: AsRef<str>>(ink: Ink, text: S) -> String {
    match depth() {
        Depth::Plain => text.as_ref().to_string(),
        _ => format!("\x1b[1m{}\x1b[22m", paint(ink, text)),
    }
}

/// Headings, the wallet's own name, anything that should read as Marigold.
pub fn accent<S: AsRef<str>>(text: S) -> String {
    paint(Ink::Gold, text)
}

/// Secondary text: explanations, hints, the part skipped on a second reading.
pub fn dim<S: AsRef<str>>(text: S) -> String {
    paint(Ink::Moss, text)
}

/// A value worth copying: an address, a serial, an amount.
pub fn value<S: AsRef<str>>(text: S) -> String {
    paint(Ink::Petal, text)
}

/// Something finished, correct, or confirmed.
pub fn ok<S: AsRef<str>>(text: S) -> String {
    style(text.as_ref()).green().to_string()
}

/// Something that needs attention but has not failed.
pub fn warn<S: AsRef<str>>(text: S) -> String {
    style(text.as_ref()).yellow().to_string()
}

/// Something that failed, or money that is not where it should be. Reserved
/// for exactly that — a red line met weekly stops being a warning.
pub fn bad<S: AsRef<str>>(text: S) -> String {
    style(text.as_ref()).red().to_string()
}

/// How wide to draw.
///
/// Clamped at both ends. Below the floor a frame is worse than no frame; past
/// the ceiling a paragraph stretched across a 300-column terminal is
/// unreadable, and the eye wants roughly the same measure a book uses.
pub const MIN_WIDTH: usize = 44;
pub const MAX_WIDTH: usize = 84;

pub fn width(ctx: &Arc<KaspaCli>) -> usize {
    ctx.term().cols().unwrap_or(80).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// How many columns a string occupies once printed.
///
/// `str::len()` is bytes, `chars().count()` is code points, and neither is
/// what a terminal does. Colour escapes occupy no columns at all and a CJK
/// character occupies two, so a frame padded by either of those measures
/// comes out ragged the moment anything inside it is styled — which, here,
/// is always.
pub fn display_width(text: &str) -> usize {
    strip_ansi(text).width()
}

/// Drop the SGR escapes so what remains is what the eye sees.
///
/// Only `ESC [ … m` is recognised, which is all `console::style` emits.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        // Swallow up to and including the terminating letter.
        if chars.next() != Some('[') {
            continue;
        }
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// Pad a possibly-styled string out to `columns`, measuring what shows.
pub fn pad(text: &str, columns: usize) -> String {
    let shown = display_width(text);
    if shown >= columns { text.to_string() } else { format!("{text}{}", " ".repeat(columns - shown)) }
}

/// Trim a possibly-styled string to fit, appending an ellipsis if it had to.
///
/// Styled input is returned unchanged when it already fits, which is the
/// common case; only over-long plain text is cut, because cutting a styled
/// string mid-escape would leave the colour turned on for the rest of the
/// line.
pub fn fit(text: &str, columns: usize) -> String {
    if display_width(text) <= columns {
        return text.to_string();
    }
    let plain = strip_ansi(text);
    let mut out = String::new();
    for ch in plain.chars() {
        if out.width() + 1 >= columns {
            break;
        }
        out.push(ch);
    }
    format!("{out}…")
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

const TL: char = '╭';
const TR: char = '╮';
const BL: char = '╰';
const BR: char = '╯';
const H: char = '─';
const V: char = '│';

/// A framed block, optionally titled.
///
/// Body lines may already be styled; they are measured by what shows, not by
/// what they contain. Lines longer than the frame are trimmed rather than
/// allowed to break it — a panel whose right edge wanders is worse than one
/// that admits it ran out of room.
pub fn panel(ctx: &Arc<KaspaCli>, title: Option<&str>, body: &[String]) {
    let outer = width(ctx);
    let inner = outer.saturating_sub(4); // two frame chars, two spaces

    let top = match title {
        Some(title) => {
            let label = format!(" {} ", accent(title));
            let used = display_width(&label);
            let rest = (outer - 2).saturating_sub(used + 1);
            format!("{TL}{H}{label}{}{TR}", H.to_string().repeat(rest))
        }
        None => format!("{TL}{}{TR}", H.to_string().repeat(outer - 2)),
    };

    ctx.term().writeln(top);
    for line in body {
        ctx.term().writeln(format!("{V} {} {V}", pad(&fit(line, inner), inner)));
    }
    ctx.term().writeln(format!("{BL}{}{BR}", H.to_string().repeat(outer - 2)));
}

/// A horizontal rule, optionally labelled — for separating sections of a
/// long report where a full frame would be too much furniture.
pub fn rule(ctx: &Arc<KaspaCli>, label: Option<&str>) {
    let outer = width(ctx);
    match label {
        Some(label) => {
            let text = format!("{H} {} ", dim(label));
            let used = display_width(&text);
            ctx.term().writeln(format!("{text}{}", H.to_string().repeat(outer.saturating_sub(used))));
        }
        None => ctx.term().writeln(H.to_string().repeat(outer)),
    }
}

// ---------------------------------------------------------------------------
// Columns
// ---------------------------------------------------------------------------

/// Which way a column's contents sit against its width.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A table with a dim header and columns wide enough for their contents.
///
/// Amounts belong in `Align::Right` so the decimal points line up; that is
/// the whole reason this exists rather than another hand-rolled `format!`
/// with a guessed column width in it.
pub fn table(ctx: &Arc<KaspaCli>, headers: &[(&str, Align)], rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }

    let mut widths: Vec<usize> = headers.iter().map(|(h, _)| display_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(display_width(cell));
            }
        }
    }

    let render = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let w = widths.get(i).copied().unwrap_or(0);
                match headers.get(i).map(|(_, a)| *a).unwrap_or(Align::Left) {
                    Align::Left => pad(cell, w),
                    Align::Right => {
                        let shown = display_width(cell);
                        format!("{}{cell}", " ".repeat(w.saturating_sub(shown)))
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };

    let header: Vec<String> = headers.iter().map(|(h, _)| dim(h)).collect();
    ctx.term().writeln(format!("  {}", render(&header)));
    for row in rows {
        ctx.term().writeln(format!("  {}", render(row)));
    }
}

/// One aligned label-and-value line, for the small facts a panel carries.
pub fn kv(label: &str, value_text: &str, label_width: usize) -> String {
    format!("{}  {value_text}", dim(pad(label, label_width)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `console` strips colour when it cannot see a terminal, which a test
    /// binary never can. The wallet turns it on explicitly at startup for
    /// exactly this reason — the browser cannot be sniffed for a TTY either —
    /// so the tests have to do the same or they measure unstyled strings and
    /// prove nothing.
    fn with_colour() {
        workflow_log::set_colors_enabled(true);
    }

    #[test]
    fn width_is_measured_in_what_shows_not_what_is_stored() {
        with_colour();
        let plain = "balance";
        let styled = accent("balance");
        assert!(styled.len() > plain.len(), "the styled string really does carry escapes");
        assert_eq!(display_width(&styled), 7, "but it occupies the same seven columns");
        assert_eq!(display_width(plain), 7);
    }

    /// The bug this module exists to prevent: a frame padded by byte length
    /// comes out ragged the moment anything inside it is coloured.
    #[test]
    fn styled_and_plain_text_pad_to_the_same_place() {
        with_colour();
        assert_eq!(display_width(&pad(&accent("ab"), 10)), 10);
        assert_eq!(display_width(&pad("ab", 10)), 10);
    }

    #[test]
    fn a_wide_character_counts_as_two_columns() {
        assert_eq!(display_width("日本"), 4);
        assert_eq!(display_width(&pad("日本", 10)), 10);
    }

    #[test]
    fn padding_never_truncates() {
        assert_eq!(pad("a rather long value", 4), "a rather long value");
    }

    #[test]
    fn fitting_leaves_short_text_alone_and_marks_what_it_cut() {
        assert_eq!(fit("short", 20), "short");
        let cut = fit("a considerably longer line than fits", 12);
        assert!(cut.ends_with('…'));
        assert!(display_width(&cut) <= 12, "got {} columns", display_width(&cut));
    }

    /// Cutting a styled string mid-escape would leave the colour turned on
    /// for everything after it, so `fit` drops the styling instead.
    #[test]
    fn cutting_a_styled_string_does_not_leak_its_colour() {
        with_colour();
        let cut = fit(&accent("a considerably longer line than fits"), 12);
        assert!(!cut.contains('\x1b'), "no half-finished escape survives: {cut:?}");
    }

    #[test]
    fn the_frame_width_stays_inside_its_band() {
        // A phone-sized terminal and a maximised 4K one both have to give
        // something readable.
        assert_eq!(20_usize.clamp(MIN_WIDTH, MAX_WIDTH), MIN_WIDTH);
        assert_eq!(300_usize.clamp(MIN_WIDTH, MAX_WIDTH), MAX_WIDTH);
        assert_eq!(72_usize.clamp(MIN_WIDTH, MAX_WIDTH), 72);
    }

    #[test]
    fn escapes_are_stripped_whole() {
        assert_eq!(strip_ansi("\x1b[38;5;214mgold\x1b[0m"), "gold");
        assert_eq!(strip_ansi("\x1b[1m\x1b[4mboth\x1b[0m"), "both");
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
    }
}
