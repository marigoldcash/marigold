//! The front door.
//!
//! The wallet opens by showing a banknote. Not a logo with a version string
//! under it — an actual specimen note, with a gold frame, a microtext band,
//! denominations in the corners and a serial at the foot. That is the whole
//! argument for the product in one screen: Marigold is cash you hold, so the
//! first thing it does is hand you one.
//!
//! Two renderings, chosen by what the terminal can fit rather than by a
//! setting. The note is drawn at exactly eighty columns because that is what
//! it was designed at and a banknote does not stretch; where there is not
//! room for it, a five-row specimen mark carries the same identity. Both fall
//! back to unstyled text under `NO_COLOR`, which is the only reason anything
//! here is built from runs of coloured text rather than one baked string.

use crate::imports::*;
use crate::ui::{self, Ink, Ink::*};

/// A stretch of one colour. Rows are lists of these so that a `NO_COLOR`
/// terminal can be handed the same rows with the ink thrown away.
type Run = (Ink, &'static str);

/// The note's designed width. Not negotiable: the bloom, the wordmark and
/// the microtext band are all centred against it, and a note that reflows is
/// not a note.
const NOTE_WIDTH: usize = 80;

/// Frame plus the line beneath it plus the prompt: twenty-one rows of note,
/// a blank, the next-step line, and the prompt make twenty-four — which is
/// what a terminal opens at on a Mac and on most Linux desktops. Below this
/// many rows the note would scroll its own top off the screen on the way in,
/// and the specimen is shown instead.
const NOTE_ROWS: usize = 24;

/// The inside of the frame.
const INNER: usize = NOTE_WIDTH - 2;

// ---------------------------------------------------------------------------
// The note
// ---------------------------------------------------------------------------

const NOTE_TOP: &str = "╔══════════════════════════════════════════════════════════════════════════════╗";
const NOTE_BOTTOM: &str = "╚══════════════════════════════════════════════════════════════════════════════╝";

/// The bloom, as tone: eighteen columns by eighteen half-rows, 0 the ground
/// and 30 the palest petal, reduced from a shaded drawing of the website's
/// marigold (founder, 2026-09-18). Drawn with two tones to a cell, so it takes
/// nine rows of the note.
const BLOOM: [[u8; 18]; 18] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 11, 12, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 3, 12, 8, 5, 20, 20, 5, 8, 12, 3, 0, 0, 0, 0],
    [0, 0, 0, 0, 8, 20, 19, 12, 18, 19, 11, 19, 20, 9, 0, 0, 0, 0],
    [0, 0, 0, 0, 7, 20, 17, 19, 18, 17, 20, 17, 20, 8, 0, 0, 0, 0],
    [0, 0, 10, 15, 12, 19, 16, 20, 19, 18, 19, 16, 19, 12, 15, 12, 0, 0],
    [0, 0, 17, 20, 17, 20, 18, 18, 22, 22, 15, 18, 20, 18, 20, 18, 1, 0],
    [0, 0, 8, 18, 15, 18, 23, 30, 26, 26, 29, 23, 17, 15, 18, 8, 0, 0],
    [0, 2, 8, 15, 20, 18, 23, 29, 22, 21, 26, 25, 20, 20, 15, 8, 2, 0],
    [0, 12, 20, 18, 19, 22, 29, 23, 9, 8, 24, 30, 22, 19, 18, 20, 12, 0],
    [0, 13, 20, 18, 18, 15, 26, 23, 8, 8, 20, 24, 19, 18, 18, 20, 14, 0],
    [0, 4, 11, 16, 20, 20, 28, 27, 18, 18, 26, 29, 20, 20, 16, 11, 4, 0],
    [0, 0, 4, 16, 16, 17, 25, 26, 28, 27, 27, 26, 18, 16, 17, 5, 0, 0],
    [0, 0, 16, 20, 18, 20, 20, 22, 26, 23, 22, 18, 20, 17, 20, 17, 0, 0],
    [0, 0, 15, 20, 15, 15, 16, 19, 20, 18, 20, 17, 16, 15, 20, 15, 1, 0],
    [0, 0, 3, 7, 6, 14, 14, 20, 19, 18, 20, 16, 15, 8, 7, 3, 0, 0],
    [0, 0, 0, 0, 8, 20, 19, 13, 17, 18, 13, 19, 20, 9, 0, 0, 0, 0],
    [0, 0, 0, 0, 4, 16, 12, 8, 20, 20, 8, 11, 16, 5, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 2, 0, 1, 16, 17, 2, 0, 1, 0, 0, 0, 0, 0],
];
const BLOOM_WIDTH: usize = 18;

/// The bloom for a terminal without colour, where two tones to a cell mean
/// nothing: a line drawing at the same nine rows.
const LINE_BLOOM: [&str; 9] = [
    "         ,.-~*'\"'*~-.,  ",
    "      ,-'  ( \\ | / )  '-, ",
    "    ,'  ( \\  ,---.  / )  ',",
    "   /  ( \\  ,'  _  ',  / )  \\",
    "  |  ( |  (  ( @ )  )  | )  |",
    "   \\  ( /  ',  ~  ,'  \\ )  /",
    "    ',  ( /  '---'  \\ )  ,' ",
    "      '-,  ( / | \\ )  ,-'  ",
    "         '~-.,*.*,.-~'   ",
];

/// The wordmark at three rows. Five was a poster; this is a note (founder,
/// 2026-09-18). The R closes its bowl underneath and steps its leg down and
/// to the right, and sits one cell from the I rather than two.
const WORDMARK: [&str; 3] = [
    "█▄ ▄█   ▄▀▄   █▀▀▄  █  ▄▀▀▀▄  ▄▀▀▀▄  █      █▀▀▄ ",
    "█ ▀ █  █▀▀▀█  █▀█▀  █  █  ▄▄  █   █  █      █   █",
    "█   █  █   █  █  ▀▄ █  ▀▄▄▄▀  ▀▄▄▄▀  █▄▄▄▄  █▄▄▀ ",
];

/// The two lines under the wordmark.
const TAGLINE: [&[Run]; 2] = [
    &[(Cream, "Digital cash in fixed notes of "), (Petal, "1, 10, 100"), (Cream, " and "), (Petal, "1,000"), (Cream, ".")],
    &[(Moss, "Notes you hold, hand over, and understand.")],
];

/// The denominations printed in the note's corners, top pair then bottom.
const CORNERS: [(&str, &str); 2] = [("1", "10"), ("100", "1000")];

/// The band that runs along the top and bottom of a real note — there to be
/// seen rather than read, which is why it is mixed nearly into the ground.
const MICROTEXT: &str = "MARIGOLD · DIGITAL CASH · ";

// ---------------------------------------------------------------------------
// The specimen
// ---------------------------------------------------------------------------

/// The same bloom at five rows, for terminals the note will not fit.
const MARK: [&[Run]; 5] = [
    &[(Amber, "  .o0o.  ")],
    &[(Amber, "o0"), (Gold, "O"), (Petal, "@@@"), (Gold, "O"), (Amber, "0o")],
    &[(Amber, "0"), (Gold, "O"), (Petal, "@"), (Amber, "#"), (Gold, "%"), (Amber, "#"), (Petal, "@"), (Gold, "O"), (Amber, "0")],
    &[(Amber, "o0"), (Gold, "O"), (Petal, "@@@"), (Gold, "O"), (Amber, "0o")],
    &[(Amber, "  '0o0'  ")],
];

// ---------------------------------------------------------------------------

fn render(row: &[Run]) -> String {
    row.iter().map(|(ink, text)| ui::paint_exact(*ink, text)).collect()
}

/// A tone from the bloom table as a colour: the ground, then amber, gold and
/// petal at ten, twenty and thirty, blended between.
fn tone(level: u8) -> (u8, u8, u8) {
    let stops = [Ground.rgb(), Amber.rgb(), Gold.rgb(), Petal.rgb()];
    let level = level.min(30) as usize;
    let (i, t) = (level / 10, (level % 10) as u16);
    if i >= 3 {
        return stops[3];
    }
    let (a, b) = (stops[i], stops[i + 1]);
    let lerp = |x: u8, y: u8| (x as i32 + (y as i32 - x as i32) * t as i32 / 10) as u8;
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

/// The nine rows of the bloom, each `INNER` wide, in whatever the terminal
/// can show: two tones to a cell where there is colour, the line drawing
/// where there is none.
fn bloom_rows() -> Vec<String> {
    if ui::depth() == ui::Depth::Plain {
        // Padded to one width first, or centring would shift the rows
        // against each other by a column.
        return LINE_BLOOM.iter().map(|line| centre(&ui::pad(line, 29), INNER)).collect();
    }
    (0..BLOOM.len() / 2)
        .map(|r| {
            let cells: String = (0..BLOOM_WIDTH).map(|c| ui::two_tone(tone(BLOOM[2 * r][c]), tone(BLOOM[2 * r + 1][c]))).collect();
            centre(&cells, INNER)
        })
        .collect()
}

/// Centre `text` in `columns`, measuring what shows rather than what is
/// stored — every line here is styled, so byte length would sit it crooked.
fn centre(text: &str, columns: usize) -> String {
    let shown = ui::display_width(text);
    if shown >= columns {
        return text.to_string();
    }
    let left = (columns - shown) / 2;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(columns - shown - left))
}

/// `TESTNET-10` becomes `T E S T N E T - 1 0`, the way a note prints a
/// denomination it wants read slowly.
fn letterspaced(text: &str) -> String {
    text.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

/// One microtext band: a denomination at each end, texture in between, cut to
/// whatever room the two numbers leave.
fn band(left: &str, right: &str) -> String {
    let numbers = left.len() + right.len();
    let room = INNER.saturating_sub(numbers + 4); // a space either side of each
    let mut filler = String::new();
    while filler.chars().count() < room {
        filler.push_str(MICROTEXT);
    }
    let filler: String = filler.chars().take(room).collect();
    format!(
        " {} {} {} ",
        ui::paint_bold(Petal, left),
        ui::paint(Micro, &filler),
        ui::paint_bold(Petal, right)
    )
}

/// The line naming what this build actually is.
///
/// Letterspaced where it fits, which is the whole point of it — a note prints
/// its denomination to be read slowly. Letterspacing doubles the width
/// though, and a network named something longer than `testnet-10` would push
/// the line straight through the frame, so it steps down rather than
/// breaking: spaced, then unspaced, then trimmed. A frame that survives a
/// value nobody expected is worth more than one that always letterspaces.
fn imprint(version: &str, network: &str) -> String {
    let network = network.to_uppercase();
    let version = format!("V{version}");

    let spaced = format!("{}   ·   {}   ·   {}", letterspaced(&network), letterspaced(&version), letterspaced("MAGLD"));
    let text = if ui::display_width(&spaced) <= INNER {
        spaced
    } else {
        let plain = format!("{network}  ·  {version}  ·  MAGLD");
        ui::fit(&plain, INNER)
    };

    ui::paint(Gold, centre(&text, INNER))
}

/// Wrap a body line in the frame.
fn framed(line: &str) -> String {
    ui::on_ground(format!("{}{}{}", ui::paint(Gold, "║"), line, ui::paint(Gold, "║")))
}

/// Every printed line of the note, top frame to bottom, each exactly
/// `NOTE_WIDTH` wide.
fn note_lines(version: &str, network: &str) -> Vec<String> {
    let blank = " ".repeat(INNER);
    let mut lines = vec![ui::on_ground(ui::paint(Gold, NOTE_TOP))];
    lines.push(framed(&band(CORNERS[0].0, CORNERS[0].1)));
    lines.push(framed(&blank));
    for row in bloom_rows() {
        lines.push(framed(&row));
    }
    lines.push(framed(&blank));
    for row in WORDMARK {
        lines.push(framed(&centre(&ui::paint_exact(Petal, row), INNER)));
    }
    for row in TAGLINE {
        lines.push(framed(&centre(&render(row), INNER)));
    }
    lines.push(framed(&imprint(version, network)));
    lines.push(framed(&band(CORNERS[1].0, CORNERS[1].1)));
    lines.push(ui::on_ground(ui::paint(Gold, NOTE_BOTTOM)));
    lines
}

/// The full note. Callers should have checked it fits.
pub fn banknote(ctx: &Arc<KaspaCli>, version: &str, network: &str, has_wallet: Option<bool>) {
    let term = ctx.term();
    for line in note_lines(version, network) {
        term.writeln(line);
    }
    term.writeln("");

    // Under the frame, only what to type. A specimen serial and the site
    // used to sit here too; under the note they read as part of it, and
    // confused rather than annotated (founder, 2026-09-15).
    term.writeln(next_steps("  ", has_wallet));
}

/// The compact mark, for anything the note will not fit.
pub fn specimen(ctx: &Arc<KaspaCli>, version: &str, network: &str, has_wallet: Option<bool>) {
    specimen_because(ctx, version, network, has_wallet, false)
}

/// The specimen, and — when the note was left out for want of room — one
/// quiet line saying what would bring it back. The wallet does not resize
/// the window itself: most terminals ignore the request and the rest move a
/// window nobody asked to move (founder, 2026-09-18).
fn specimen_because(ctx: &Arc<KaspaCli>, version: &str, network: &str, has_wallet: Option<bool>, too_small: bool) {
    let term = ctx.term();
    let facts = [
        ui::paint_bold(Petal, letterspaced("MARIGOLD").replace(' ', "  ")),
        ui::paint(Cream, "Digital cash in fixed notes."),
        ui::paint(Moss, "Notes you hold, hand over, and understand."),
        ui::paint(Gold, format!("{}  ·  V{version}  ·  MAGLD", network.to_uppercase())),
        next_steps("", has_wallet),
    ];

    term.writeln("");
    for (i, row) in MARK.iter().enumerate() {
        let mark = render(row);
        let fact = facts.get(i).cloned().unwrap_or_default();
        term.writeln(format!("   {}    {fact}", ui::pad(&mark, 9)));
    }
    if too_small {
        term.writeln("");
        term.writeln(format!(
            "   {}",
            ui::paint(Micro, format!("the full note needs {NOTE_WIDTH} by {NOTE_ROWS} · widen the window and type about"))
        ));
    }
    term.writeln("");
}

/// The one line that tells somebody what to do next. Two verbs, not thirty —
/// the full list is one word away and a wall of commands at the door teaches
/// nobody anything.
///
/// It names the step for this machine: someone with a wallet opens it,
/// someone without makes one, and a build that could not look (`about` from
/// inside a session, a harness) gets the general pair.
fn next_steps(indent: &str, has_wallet: Option<bool>) -> String {
    let (first, first_hint, second, second_hint) = match has_wallet {
        Some(true) => ("open", " to open your wallet · ", "help", " for a list of commands"),
        Some(false) => ("wallet create", " to make one · ", "guide", " for a walkthrough"),
        None => ("help", " for a list of commands · ", "guide", " for a walkthrough"),
    };
    format!(
        "{indent}{}{}{}{}{}",
        ui::paint(Moss, "type "),
        ui::paint(Petal, first),
        ui::paint(Moss, first_hint),
        ui::paint(Petal, second),
        ui::paint(Moss, second_hint)
    )
}

/// Show whichever fits.
///
/// The note needs its full eighty columns and enough rows not to scroll its
/// own frame away as it prints. A terminal that cannot give both gets the
/// specimen, which says the same thing in five rows.
pub fn show(ctx: &Arc<KaspaCli>, version: &str, network: Option<&str>, has_wallet: Option<bool>) {
    let network = network.unwrap_or("unknown network");

    match (ui::measured(ctx.term().cols()), ui::measured(ctx.term().rows())) {
        (Some(cols), Some(rows)) if cols < NOTE_WIDTH || rows < NOTE_ROWS => specimen_because(ctx, version, network, has_wallet, true),
        // A terminal that will not say how big it is is not a small terminal.
        // It is a container, a pipe, or a test harness — `docker compose run`
        // hands the container a pty reporting zero by zero. Guessing "small"
        // there means no Docker tester ever sees the note, which is the one
        // thing this screen exists for; guessing "roomy" costs a wrapped line
        // in the rare case it is wrong.
        _ => banknote(ctx, version, network, has_wallet),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every printed row of the note has to be exactly eighty columns, or the
    /// right-hand frame wanders and the whole thing stops reading as a note.
    #[test]
    fn every_row_of_the_note_is_the_same_width() {
        let lines = note_lines("2.0.46", "testnet-10");
        for (i, line) in lines.iter().enumerate() {
            let width = ui::display_width(line);
            assert_eq!(width, NOTE_WIDTH, "line {i} is {width} columns, not {NOTE_WIDTH}");
        }
        assert_eq!(ui::display_width(NOTE_TOP), NOTE_WIDTH);
        assert_eq!(ui::display_width(NOTE_BOTTOM), NOTE_WIDTH);
    }

    /// Twenty-one rows of note, so that note, blank, next step and prompt fit
    /// a terminal that opens at twenty-four.
    #[test]
    fn the_note_fits_a_default_terminal() {
        assert_eq!(note_lines("2.0.46", "testnet-10").len(), NOTE_ROWS - 3);
    }

    #[test]
    fn the_bloom_and_wordmark_are_the_shape_they_claim() {
        assert_eq!(BLOOM.len(), 18);
        assert!(BLOOM.iter().all(|row| row.len() == BLOOM_WIDTH));
        assert!(BLOOM.iter().flatten().all(|&t| t <= 30));
        for line in LINE_BLOOM {
            assert!(ui::display_width(line) <= INNER);
        }
        for row in WORDMARK {
            assert_eq!(ui::display_width(row), 49, "{row}");
        }
        assert_eq!(tone(0), Ground.rgb());
        assert_eq!(tone(10), Amber.rgb());
        assert_eq!(tone(20), Gold.rgb());
        assert_eq!(tone(30), Petal.rgb());
    }

    /// The bands carry runtime-sized numbers, so they are the rows most
    /// likely to come out a column short.
    #[test]
    fn the_microtext_bands_fill_the_frame_exactly() {
        for (left, right) in CORNERS {
            let rendered = band(left, right);
            assert_eq!(ui::display_width(&rendered), INNER, "band {left}/{right} misfits");
        }
        // And a denomination that grows later must not break it either.
        assert_eq!(ui::display_width(&band("1000000", "1")), INNER);
    }

    /// The imprint is built from the live network name and version, which are
    /// the only things on the note that change between builds.
    #[test]
    fn the_imprint_fits_and_names_the_build() {
        let line = imprint("2.0.46", "testnet-10");
        assert_eq!(ui::display_width(&line), INNER);
        let bare: String = line.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        assert!(bare.contains("TESTNET10"), "the network is named: {bare}");
        assert!(bare.contains("V2046"), "the version is named: {bare}");

        // A longer network name must still not push the frame out.
        assert_eq!(ui::display_width(&imprint("10.20.30", "some-long-network-name")), INNER);
    }

    #[test]
    fn letterspacing_reads_slowly() {
        assert_eq!(letterspaced("MAGLD"), "M A G L D");
        assert_eq!(letterspaced(""), "");
    }

    #[test]
    fn centring_measures_what_shows_not_what_is_stored() {
        let styled = ui::paint(Gold, "abc");
        let centred = centre(&styled, 11);
        assert_eq!(ui::display_width(&centred), 11);
        // Four spaces each side of three columns of text.
        assert!(centred.starts_with("    "), "{centred:?}");
    }

    /// The compact mark is square-ish and every row the same width, or it
    /// leans and the text beside it steps in and out.
    #[test]
    fn the_specimen_mark_is_even() {
        let widths: Vec<usize> = MARK.iter().map(|row| row.iter().map(|(_, t)| ui::display_width(t)).sum()).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "uneven mark rows: {widths:?}");
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    /// Which rendering a given terminal gets. Kept in one place so the rule
    /// can be stated as a table rather than inferred from the branch.
    fn choice(cols: Option<usize>, rows: Option<usize>) -> &'static str {
        match (ui::measured(cols), ui::measured(rows)) {
            (Some(c), Some(r)) if c < NOTE_WIDTH || r < NOTE_ROWS => "specimen",
            _ => "note",
        }
    }

    #[test]
    fn a_terminal_with_room_gets_the_note() {
        assert_eq!(choice(Some(80), Some(30)), "note", "exactly the minimum still fits");
        assert_eq!(choice(Some(120), Some(45)), "note");
    }

    #[test]
    fn a_terminal_without_room_gets_the_specimen() {
        assert_eq!(choice(Some(79), Some(40)), "specimen", "one column short");
        assert_eq!(choice(Some(100), Some(NOTE_ROWS - 1)), "specimen", "one row short");
        assert_eq!(choice(Some(40), Some(10)), "specimen");
    }

    /// The case that sent every Docker tester the compact mark: `docker
    /// compose run` gives the container a pty reporting zero by zero, and
    /// zero read as "tiny" rather than "it will not say".
    #[test]
    fn a_terminal_that_will_not_say_its_size_gets_the_note() {
        assert_eq!(choice(Some(0), Some(0)), "note", "a container pty reports zero");
        assert_eq!(choice(None, None), "note", "so does a pipe");
        assert_eq!(choice(Some(0), Some(40)), "note");
        assert_eq!(choice(Some(120), Some(0)), "note");
    }
}
