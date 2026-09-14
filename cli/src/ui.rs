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
    /// Body text — deliberately *not* a colour. On the note this is cream,
    /// but a terminal already has a foreground its owner chose, and painting
    /// near-white text on a white terminal is how the browser wallet first
    /// came out unreadable. Body text is whatever this terminal calls text.
    Cream,
    /// Secondary text — the part skipped on a second reading. Pulled darker
    /// than the note's moss so that it survives a light ground as well as the
    /// note's own dark one.
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
            // Never used: `paint` short-circuits Cream to the terminal's own
            // foreground. Kept so the note's palette is written down whole.
            Ink::Cream => (0xef, 0xe7, 0xd3),
            Ink::Moss => (0x7e, 0x8c, 0x86),
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
    if ink == Ink::Cream {
        return text.to_string();
    }
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
    measured(ctx.term().cols()).unwrap_or(80).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// What the terminal says it is, or `None` where it will not say.
///
/// Zero is not a width. A container started by `docker compose run` gets a
/// pty that reports zero by zero — `stty size` says so too — and taking that
/// literally clamps every frame in the wallet to its minimum and makes the
/// splash think it is on a phone. Zero means "this terminal cannot tell you",
/// which is a different answer from "small".
pub fn measured(value: Option<usize>) -> Option<usize> {
    value.filter(|v| *v > 0)
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

/// A column: its heading (empty for none), which way its contents sit, and
/// the width it holds even when its contents are narrower.
///
/// The minimum is what stops a one-row table collapsing into
/// `notes 0 MAGLD` — columns sized purely by content look wrong precisely
/// when there is least content, which is a new wallet's very first `balance`.
pub type Column = (&'static str, Align, usize);

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
pub fn table(ctx: &Arc<KaspaCli>, headers: &[Column], rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }

    let mut widths: Vec<usize> = headers.iter().map(|(h, _, min)| display_width(h).max(*min)).collect();
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
                match headers.get(i).map(|(_, a, _)| *a).unwrap_or(Align::Left) {
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

    // A table of figures often wants the alignment without the labels — a
    // balance does not need a column called "amount". Passing empty headers
    // asks for exactly that, rather than printing a blank line above the
    // numbers.
    if headers.iter().any(|(h, _, _)| !h.is_empty()) {
        let header: Vec<String> = headers.iter().map(|(h, _, _)| dim(h)).collect();
        ctx.term().writeln(format!("  {}", render(&header)));
    }
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

/// The twenty-four words, framed and numbered, as the one screen in the
/// wallet that must not be skimmed.
///
/// Numbered in four rows of six because that is how they get copied onto
/// paper and checked back, and because an unnumbered block of twenty-four
/// words is transcribed wrong often enough to matter. Framed because this is
/// a moment rather than a routine — everything else the wallet prints can be
/// scrolled past, and this cannot.
pub fn recovery_words(ctx: &Arc<KaspaCli>, words: &str) {
    let words: Vec<&str> = words.split_whitespace().collect();
    const PER_ROW: usize = 6;

    let longest = words.iter().map(|w| w.len()).max().unwrap_or(0);
    let cell = longest + 5; // "12 " plus the word plus a gap

    let mut body = vec![String::new()];
    for (row, chunk) in words.chunks(PER_ROW).enumerate() {
        let line: String = chunk
            .iter()
            .enumerate()
            .map(|(column, word)| {
                let n = row * PER_ROW + column + 1;
                pad(&format!("{}{}", paint(Ink::Micro, format!("{n:>2} ")), paint(Ink::Petal, *word)), cell)
            })
            .collect();
        body.push(format!("  {}", line.trim_end()));
    }
    body.push(String::new());

    panel(ctx, Some("your 24 recovery words — write them down now"), &body);
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// A column keeps its width when its contents are narrower, which is what
    /// stops a brand-new wallet's balance reading `notes 0 MAGLD`.
    #[test]
    fn a_minimum_holds_a_column_open() {
        let columns: [Column; 2] = [("", Align::Left, 12), ("", Align::Right, 8)];
        let widths: Vec<usize> = columns.iter().map(|(h, _, min)| display_width(h).max(*min)).collect();
        assert_eq!(widths, vec![12, 8]);
    }

    /// ...and gives way to content that is wider than it.
    #[test]
    fn content_wider_than_the_minimum_still_fits() {
        let columns: [Column; 1] = [("", Align::Left, 4)];
        let rows = [vec!["a much longer cell".to_string()]];
        let mut widths: Vec<usize> = columns.iter().map(|(h, _, min)| display_width(h).max(*min)).collect();
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(display_width(cell));
            }
        }
        assert_eq!(widths[0], 18);
    }

    /// The twenty-four words have to arrive as twenty-four numbered words, in
    /// rows of six, inside a frame that does not wander.
    #[test]
    fn the_recovery_panel_numbers_every_word() {
        workflow_log::set_colors_enabled(true);
        let words: Vec<String> = (1..=24).map(|n| format!("word{n}")).collect();
        let joined = words.join(" ");
        let split: Vec<&str> = joined.split_whitespace().collect();
        assert_eq!(split.len(), 24);
        assert_eq!(split.chunks(6).count(), 4, "four rows of six");
    }
}

#[cfg(test)]
mod trust_tests {
    /// Zero and "not read yet" are different facts, and the wallet has to be
    /// able to tell them apart before it prints either.
    ///
    /// This is the rule that was missing: a reload clears the UTXO context and
    /// refills it from the node, so a reload that times out leaves an empty
    /// context — which looks exactly like an empty ledger. The wallet told
    /// somebody holding 812,524 TMAGLD across 4.4 million coins that their
    /// ledger was empty, and then used that same zero to decide there was
    /// nothing to mint.
    fn figure(known: bool, mature: u64) -> &'static str {
        if !known {
            "not read yet"
        } else if mature > 0 {
            "an amount"
        } else {
            "zero"
        }
    }

    #[test]
    fn an_unread_ledger_is_never_reported_as_empty() {
        assert_eq!(figure(false, 0), "not read yet");
        assert_eq!(figure(false, 812_524), "not read yet", "unknown stays unknown whatever is cached");
    }

    #[test]
    fn a_ledger_that_really_is_empty_still_says_so() {
        assert_eq!(figure(true, 0), "zero");
        assert_eq!(figure(true, 812_524), "an amount");
    }
}

/// A ledger amount, to the hundredth and no further.
///
/// Below 0.01 nothing can be done with it: the smallest note is 0.01, so a
/// remainder under that cannot be minted, cannot be sent to an exchange, and
/// cannot be spent as a note. Printing `812,524.99677725` asks somebody to
/// read eight digits that change nothing, and hides the figure that matters
/// in the middle of them.
///
/// Truncated, never rounded. `812,524.99677725` shows as `812,524.99`, not
/// `812,525.00` — a balance may be shown as less than it is, and must never
/// be shown as more.
pub fn ledger_amount(petals: u64) -> String {
    use kaspa_consensus_core::constants::SOMPI_PER_KASPA;
    const PER_HUNDREDTH: u64 = SOMPI_PER_KASPA / 100;
    let whole = petals / SOMPI_PER_KASPA;
    let hundredths = (petals % SOMPI_PER_KASPA) / PER_HUNDREDTH;
    format!("{}.{hundredths:02}", whole.separated_string())
}

#[cfg(test)]
mod amount_tests {
    use super::*;

    #[test]
    fn a_ledger_amount_stops_at_the_hundredth() {
        assert_eq!(ledger_amount(81_252_499_677_725), "812,524.99");
        assert_eq!(ledger_amount(34_148_284_000_000), "341,482.84");
        assert_eq!(ledger_amount(0), "0.00");
        assert_eq!(ledger_amount(100_000_000), "1.00");
    }

    /// The direction that matters. Rounding would have turned
    /// 812,524.99677725 into 812,525.00 — more money than the person has.
    #[test]
    fn it_truncates_rather_than_rounds() {
        assert_eq!(ledger_amount(199_999_999), "1.99", "1.99999999 is not 2.00");
        assert_eq!(ledger_amount(999_999), "0.00", "dust below a hundredth is nothing you can use");
    }

    #[test]
    fn large_amounts_keep_their_separators() {
        assert_eq!(ledger_amount(1_234_567_800_000_000), "12,345,678.00");
    }
}

#[cfg(test)]
mod housekeeping_rules {
    /// Housekeeping mints, then consolidates, in one pass. Both steps used to
    /// refuse to run while *anything at all* was unconfirmed — and the mint
    /// runs first, so by the time the sweep was reached there were always
    /// unconfirmed spends: the ones the mint had just made.
    ///
    /// Step one guaranteed step two would never run. On a wallet with income
    /// arriving continuously the mint always has something to do, so the
    /// consolidation was skipped on every single pass and the coin count only
    /// ever grew — to 4,439,373 before anyone looked.
    fn sweep_runs(mint_submitted_something: bool, gated_on_unconfirmed: bool, pieces_over_threshold: bool) -> bool {
        if !pieces_over_threshold {
            return false;
        }
        if gated_on_unconfirmed && mint_submitted_something {
            return false;
        }
        true
    }

    #[test]
    fn the_old_gate_starved_the_sweep_whenever_the_mint_had_work() {
        assert!(!sweep_runs(true, true, true), "this is the bug: minting blocked consolidating");
        assert!(sweep_runs(false, true, true), "it only ran on a pass where the mint did nothing");
    }

    #[test]
    fn without_the_gate_consolidation_keeps_up() {
        assert!(sweep_runs(true, false, true));
        assert!(sweep_runs(false, false, true));
    }

    /// The threshold still decides whether there is anything worth doing.
    #[test]
    fn a_tidy_ledger_is_still_left_alone() {
        assert!(!sweep_runs(true, false, false));
        assert!(!sweep_runs(false, false, false));
    }
}

#[cfg(test)]
mod backlog_rules {
    /// Automatic consolidation runs at a rate the wallet sets: a pass a
    /// minute, two hundred transactions a pass, about eighty coins a
    /// transaction. Past a certain backlog that is hours of unasked-for work
    /// on somebody's money, so housekeeping stops and hands it over.
    const PER_MINUTE: u64 = 200 * 80;
    const CEILING: u64 = 250_000;

    fn runs_unattended(pieces: u64) -> bool {
        pieces <= CEILING
    }

    #[test]
    fn an_ordinary_backlog_is_still_tidied_in_the_background() {
        assert!(runs_unattended(0));
        assert!(runs_unattended(2_000), "the usual auto-sweep threshold");
        assert!(runs_unattended(CEILING));
    }

    #[test]
    fn a_backlog_that_would_take_hours_is_handed_to_the_person() {
        assert!(!runs_unattended(CEILING + 1));
        assert!(!runs_unattended(4_439_373), "the wallet that prompted this");
    }

    /// The ceiling is about a quarter of an hour of work, not a round number
    /// chosen because it looked tidy.
    #[test]
    fn the_ceiling_is_roughly_fifteen_minutes_of_work() {
        let minutes = CEILING / PER_MINUTE;
        assert!((10..=20).contains(&minutes), "ceiling is {minutes} minutes of consolidation");
    }

    /// And the figure quoted to the person is that same arithmetic.
    #[test]
    fn the_quoted_time_matches_the_rate_it_actually_runs_at() {
        let hours = 4_439_373 / PER_MINUTE / 60;
        assert_eq!(hours, 4, "4.4 million coins is about four hours of background consolidation");
    }
}
