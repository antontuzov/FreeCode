//! Assertions on the picture a capture holds.
//!
//! A capture is a PNG of what the page drew, and the two things a run can say about a picture
//! without a model looking at it are measurements: the picture is not one flat colour, so the page
//! drew something, or one point of it is the colour it should be, so what the run placed there
//! landed there. Both are decided from the bytes the host already holds, which is what makes the
//! answer a property of the page rather than a description of it — and what lets a text-only route
//! check a page it cannot see.

use serde::{Deserialize, Serialize};

/// The slack a [`Expect::Colour`] allows per channel when none was asked for. A page drawn at
/// fractional sizes lands between pixels, and anti-aliasing moves a value by a few steps even away
/// from an edge, so an exact match would fail on a page that is right.
pub const DEFAULT_TOLERANCE: u8 = 24;

/// What one check asserts about the picture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "expect", rename_all = "kebab-case")]
pub enum Expect {
    /// The picture holds more than one colour: the page rendered something rather than a flat fill.
    NotBlank,
    /// The pixel at a point of the picture is the colour named, within `tolerance` per channel.
    Colour {
        /// Across the picture: 0 is the left edge, 1 the right.
        x: f64,
        /// Down the picture: 0 is the top edge, 1 the bottom.
        y: f64,
        /// `#rrggbb`.
        rgb: String,
        #[serde(default = "default_tolerance")]
        tolerance: u8,
    },
}

fn default_tolerance() -> u8 {
    DEFAULT_TOLERANCE
}

impl Expect {
    /// Whether the assertion is one this build can make. The executor asks before the panel is
    /// touched, so a malformed check is a refused call rather than a capture taken for nothing.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::NotBlank => Ok(()),
            Self::Colour { x, y, rgb, .. } => {
                if !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y) {
                    return Err(format!(
                        "x and y are fractions of the picture between 0 and 1; got {x} and {y}"
                    ));
                }
                parse_rgb(rgb).map(|_| ())
            }
        }
    }

    /// The assertion as the row and the model's result name it: the token the model wrote, and, for
    /// a colour, the point it named.
    pub fn name(&self) -> String {
        match self {
            Self::NotBlank => "not-blank".to_string(),
            Self::Colour { x, y, rgb, .. } => format!(
                "{} at {}% × {}%",
                hex(parse_rgb(rgb).unwrap_or([0, 0, 0])),
                (x * 100.0).round() as i64,
                (y * 100.0).round() as i64
            ),
        }
    }
}

/// What a check measured: whether the assertion held, and the sentence that says what the pixels
/// showed. The sentence is English because it goes back into the prompt; the row has its own words.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub passed: bool,
    pub detail: String,
    pub width: u32,
    pub height: u32,
}

/// Judge a PNG against one assertion. `Err` is a picture this build cannot read or an assertion it
/// cannot make — never a failed check, which is a `Verdict` with `passed: false`.
pub fn evaluate(png: &[u8], expect: &Expect) -> Result<Verdict, String> {
    expect.validate()?;
    let picture = decode(png)?;
    match expect {
        Expect::NotBlank => Ok(not_blank(&picture)),
        Expect::Colour {
            x,
            y,
            rgb,
            tolerance,
        } => Ok(colour(&picture, *x, *y, parse_rgb(rgb)?, *tolerance)),
    }
}

/// The picture again, small enough to hang on a row of the run report, as a PNG of its own.
///
/// A capture is around 100 KB and the report's column is a few dozen pixels wide, so the row that
/// says a picture was taken carries a copy at `max_width` rather than the file: a reopened
/// conversation then draws the frame it was shown instead of a name for a file that may be gone.
/// Each output pixel is the average of the block it covers, so a page's text and edges stay
/// readable as shapes rather than turning into speckle.
pub fn thumbnail(png: &[u8], max_width: u32) -> Result<Vec<u8>, String> {
    let picture = decode(png)?;
    let width = picture.width.min(max_width.max(1));
    let height =
        (u64::from(picture.height) * u64::from(width) / u64::from(picture.width)).max(1) as u32;
    let mut small = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        let y0 = block_start(y, height, picture.height);
        let y1 = block_end(y, height, picture.height);
        for x in 0..width {
            let x0 = block_start(x, width, picture.width);
            let x1 = block_end(x, width, picture.width);
            let mut sum = [0u64; 3];
            let mut read = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let [r, g, b] = picture.rgb(sx, sy);
                    sum[0] += u64::from(r);
                    sum[1] += u64::from(g);
                    sum[2] += u64::from(b);
                    read += 1;
                }
            }
            let read = read.max(1);
            for channel in sum {
                small.push((channel / read) as u8);
            }
        }
    }
    encode(&small, width, height)
}

/// Where the block of source pixels a thumbnail pixel covers begins.
fn block_start(at: u32, cells: u32, source: u32) -> u32 {
    (u64::from(at) * u64::from(source) / u64::from(cells)) as u32
}

/// Where it ends — never empty, so a thumbnail taller than its source cannot skip a row.
fn block_end(at: u32, cells: u32, source: u32) -> u32 {
    let start = block_start(at, cells, source);
    block_start(at + 1, cells, source)
        .max(start + 1)
        .min(source)
}

fn encode(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|why| format!("the picture's thumbnail cannot be written: {why}"))?;
        writer
            .write_image_data(rgb)
            .map_err(|why| format!("the picture's thumbnail cannot be written: {why}"))?;
        writer
            .finish()
            .map_err(|why| format!("the picture's thumbnail cannot be written: {why}"))?;
    }
    Ok(bytes)
}

/// A decoded frame, read as RGB. Palette and sub-byte depths are expanded by the decoder; a 16-bit
/// sample keeps its high byte, which is all a colour check needs.
struct Picture {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    channels: usize,
    sample: usize,
    stride: usize,
}

impl Picture {
    fn offset(&self, x: u32, y: u32) -> usize {
        y as usize * self.stride + x as usize * self.channels * self.sample
    }

    /// The pixel's colour. A greyscale frame repeats its one channel, so every assertion reads the
    /// same picture whether the webview encoded it as grey, RGB or RGBA.
    fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let at = self.offset(x, y);
        match self.channels {
            1 | 2 => [self.pixels[at]; 3],
            _ => [
                self.pixels[at],
                self.pixels[at + self.sample],
                self.pixels[at + 2 * self.sample],
            ],
        }
    }

    /// The pixel's alpha, when the frame carries one. `None` is an opaque picture.
    fn alpha(&self, x: u32, y: u32) -> Option<u8> {
        match self.channels {
            2 => Some(self.pixels[self.offset(x, y) + self.sample]),
            4 => Some(self.pixels[self.offset(x, y) + 3 * self.sample]),
            _ => None,
        }
    }
}

fn decode(png: &[u8]) -> Result<Picture, String> {
    let mut decoder = png::Decoder::new(png);
    // A capture can be written as palette or 1–4 bit pixels by a webview that optimises away
    // detail; expanding first means every assertion below sees 8-bit samples and named channels.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .map_err(|why| format!("the capture is not a PNG this build can read: {why}"))?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|why| format!("the capture's pixels cannot be read: {why}"))?;
    if info.width < 2 || info.height < 2 {
        return Err(format!(
            "the capture is {}×{}, which is not a picture of a page",
            info.width, info.height
        ));
    }
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => {
            return Err(format!(
                "the capture's pixels are {other:?}, which this build cannot read"
            ))
        }
    };
    let sample = match info.bit_depth {
        png::BitDepth::Sixteen => 2,
        _ => 1,
    };
    pixels.truncate(info.buffer_size());
    Ok(Picture {
        pixels,
        width: info.width,
        height: info.height,
        channels,
        sample,
        stride: info.line_size,
    })
}

/// How many points a `not-blank` check reads across and down. A page that drew anything at all —
/// a line, a word, a border — shows up in 1024 points; a page that painted nothing is one colour
/// everywhere, whatever that colour is.
const SAMPLES: u32 = 32;

fn not_blank(picture: &Picture) -> Verdict {
    let mut seen = std::collections::HashSet::new();
    let mut read = 0;
    for row in 0..SAMPLES {
        for col in 0..SAMPLES {
            let x = col * (picture.width - 1) / (SAMPLES - 1);
            let y = row * (picture.height - 1) / (SAMPLES - 1);
            seen.insert(picture.rgb(x, y));
            read += 1;
        }
    }
    let (passed, detail) = if seen.len() < 2 {
        let [r, g, b] = seen.into_iter().next().unwrap_or([0, 0, 0]);
        (
            false,
            format!(
                "All {read} sampled points are the same colour ({}): the page painted a single flat \
                 fill and drew nothing else.",
                hex([r, g, b])
            ),
        )
    } else {
        (
            true,
            format!(
                "{} sampled points hold {} different colours, so the page drew something.",
                read,
                seen.len()
            ),
        )
    };
    Verdict {
        passed,
        detail,
        width: picture.width,
        height: picture.height,
    }
}

fn colour(picture: &Picture, x: f64, y: f64, want: [u8; 3], tolerance: u8) -> Verdict {
    let px = ((x * picture.width as f64) as u32).min(picture.width - 1);
    let py = ((y * picture.height as f64) as u32).min(picture.height - 1);
    let at = (x * 100.0).round() as i64;
    let down = (y * 100.0).round() as i64;
    let size = (picture.width, picture.height);
    let verdict = |passed, detail| Verdict {
        passed,
        detail,
        width: size.0,
        height: size.1,
    };
    if picture.alpha(px, py) == Some(0) {
        return verdict(
            false,
            format!(
                "The pixel at {at}% × {down}% ({px}, {py}) is transparent, so nothing was drawn \
                 there; {} was expected.",
                hex(want)
            ),
        );
    }
    let got = picture.rgb(px, py);
    let off: Vec<u16> = (0..3)
        .map(|channel| (got[channel] as i16 - want[channel] as i16).unsigned_abs())
        .collect();
    let worst = *off.iter().max().unwrap_or(&0);
    if worst <= tolerance as u16 {
        return verdict(
            true,
            format!(
                "The pixel at {at}% × {down}% ({px}, {py}) is {}, within {tolerance} of {}.",
                hex(got),
                hex(want)
            ),
        );
    }
    let channel = ["red", "green", "blue"][off.iter().position(|one| *one == worst).unwrap_or(0)];
    verdict(
        false,
        format!(
            "The pixel at {at}% × {down}% ({px}, {py}) is {}, not {}: it is off by {worst} in \
             {channel}, more than the {tolerance} allowed. The page did not draw that colour at \
             that point.",
            hex(got),
            hex(want)
        ),
    )
}

fn parse_rgb(raw: &str) -> Result<[u8; 3], String> {
    let digits = raw.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return Err(format!(
            "\"{raw}\" is not a colour: write it as #rrggbb, six hexadecimal digits"
        ));
    }
    let at = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).unwrap_or(0);
    Ok([at(0), at(2), at(4)])
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `width × height` RGBA PNG whose pixels come from one function.
    fn png_of(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("a PNG header");
            let mut data = Vec::with_capacity((width * height * 4) as usize);
            for y in 0..height {
                for x in 0..width {
                    data.extend_from_slice(&pixel(x, y));
                }
            }
            writer.write_image_data(&data).expect("pixels");
        }
        bytes
    }

    #[test]
    fn a_picture_of_one_colour_did_not_draw_anything() {
        let flat = png_of(40, 30, |_, _| [255, 255, 255, 255]);
        let verdict = evaluate(&flat, &Expect::NotBlank).expect("a readable picture");
        assert!(!verdict.passed);
        assert_eq!((verdict.width, verdict.height), (40, 30));
        assert!(verdict.detail.contains("#ffffff"), "{}", verdict.detail);
        assert!(verdict.detail.contains("flat fill"), "{}", verdict.detail);

        let drawn = png_of(40, 30, |x, _| {
            if x < 20 {
                [220, 40, 40, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let verdict = evaluate(&drawn, &Expect::NotBlank).expect("a readable picture");
        assert!(verdict.passed);
        assert!(
            verdict.detail.contains("2 different colours"),
            "{}",
            verdict.detail
        );
    }

    #[test]
    fn a_thumbnail_keeps_the_shape_and_the_colours_of_the_frame() {
        // Red on the left half, white on the right: the shape a page's layout would leave in pixels.
        let full = png_of(40, 30, |x, _| {
            if x < 20 {
                [220, 40, 40, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let small = thumbnail(&full, 10).expect("a picture can be drawn small");
        let picture = decode(&small).expect("the thumbnail is a PNG this module reads");
        // A quarter of the width and the same proportions, rounded down to whole rows.
        assert_eq!((picture.width, picture.height), (10, 7));
        // The halves are still red and white: the row holds the frame, not a blur of it.
        assert_eq!(picture.rgb(1, 4), [220, 40, 40]);
        assert_eq!(picture.rgb(8, 4), [255, 255, 255]);

        // A frame already narrower than the row is copied rather than stretched.
        let tiny = png_of(2, 2, |x, _| {
            if x == 0 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let small = thumbnail(&tiny, 96).expect("a picture can be drawn small");
        let picture = decode(&small).expect("the thumbnail is a PNG this module reads");
        assert_eq!((picture.width, picture.height), (2, 2));
        assert_eq!(picture.rgb(0, 0), [0, 0, 0]);
    }

    #[test]
    fn a_colour_check_reads_the_point_it_names() {
        // Ten columns: the left half red, the right half blue, so 25% and 75% fall in different
        // halves and 75% is inside the blue rather than on the seam.
        let half = png_of(10, 10, |x, _| {
            if x < 5 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let blue = Expect::Colour {
            x: 0.75,
            y: 0.5,
            rgb: "#0000ff".into(),
            tolerance: 0,
        };
        let verdict = evaluate(&half, &blue).expect("a readable picture");
        assert!(verdict.passed, "{}", verdict.detail);
        assert!(verdict.detail.contains("(7, 5)"), "{}", verdict.detail);

        let wrong = Expect::Colour {
            x: 0.25,
            y: 0.5,
            rgb: "#0000ff".into(),
            tolerance: 0,
        };
        let verdict = evaluate(&half, &wrong).expect("a readable picture");
        assert!(!verdict.passed);
        assert!(verdict.detail.contains("#ff0000"), "{}", verdict.detail);
        assert!(verdict.detail.contains("red"), "{}", verdict.detail);

        // A near miss is inside the slack: a page drawn one step off the colour is not a failure.
        let near = Expect::Colour {
            x: 0.75,
            y: 0.5,
            rgb: "#0a0aff".into(),
            tolerance: 12,
        };
        assert!(evaluate(&half, &near).expect("a readable picture").passed);
    }

    #[test]
    fn a_transparent_point_has_no_colour_to_read() {
        let hole = png_of(8, 8, |x, y| {
            if x == 0 && y == 0 {
                [0, 0, 0, 0]
            } else {
                [255, 255, 255, 255]
            }
        });
        let at_corner = Expect::Colour {
            x: 0.0,
            y: 0.0,
            rgb: "#000000".into(),
            tolerance: 0,
        };
        let verdict = evaluate(&hole, &at_corner).expect("a readable picture");
        assert!(!verdict.passed);
        assert!(verdict.detail.contains("transparent"), "{}", verdict.detail);
    }

    #[test]
    fn a_check_refuses_a_picture_it_cannot_read() {
        let why = evaluate(b"not a png", &Expect::NotBlank).expect_err("garbage is no picture");
        assert!(why.contains("PNG"), "{why}");
    }

    #[test]
    fn an_assertion_parses_the_way_a_model_writes_it() {
        let blank: Expect = serde_json::from_value(serde_json::json!({ "expect": "not-blank" }))
            .expect("the short form parses");
        assert_eq!(blank, Expect::NotBlank);

        let colour: Expect = serde_json::from_value(serde_json::json!({
            "expect": "colour",
            "x": 0.5,
            "y": 0.25,
            "rgb": "#2f6b3f"
        }))
        .expect("a colour check parses");
        assert_eq!(
            colour,
            Expect::Colour {
                x: 0.5,
                y: 0.25,
                rgb: "#2f6b3f".into(),
                tolerance: DEFAULT_TOLERANCE,
            }
        );
        assert_eq!(colour.name(), "#2f6b3f at 50% × 25%");

        assert!(
            serde_json::from_value::<Expect>(serde_json::json!({ "expect": "bright" })).is_err()
        );
    }

    #[test]
    fn an_assertion_outside_the_picture_or_the_colour_space_is_refused() {
        let outside = Expect::Colour {
            x: 1.4,
            y: 0.5,
            rgb: "#ffffff".into(),
            tolerance: 0,
        };
        assert!(outside.validate().unwrap_err().contains("between 0 and 1"));

        let not_hex = Expect::Colour {
            x: 0.5,
            y: 0.5,
            rgb: "#gggggg".into(),
            tolerance: 0,
        };
        assert!(not_hex.validate().unwrap_err().contains("#rrggbb"));
    }

    #[test]
    fn a_colour_round_trips_through_the_fields_the_host_is_sent() {
        // The executor validates and forwards this object; the host parses it back into the same
        // assertion, so the two halves cannot disagree about what was asked.
        let asked = Expect::Colour {
            x: 0.25,
            y: 0.75,
            rgb: "#0033cc".into(),
            tolerance: 8,
        };
        let wire = serde_json::to_value(&asked).expect("an assertion serialises");
        let back: Expect = serde_json::from_value(wire).expect("and parses back");
        assert_eq!(back, asked);
    }
}
