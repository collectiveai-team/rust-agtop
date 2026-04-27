//! Braille sparkline. Renders a Vec<f32> series as a single line of braille
//! characters where each cell encodes 2×4 dots from up to 8 data points.

/// Render a series of values into braille characters covering `width` cells.
///
/// Values are normalized to `[0.0, max]` (auto-detected from the series if
/// `max <= 0.0`). Each cell consumes `series.len() / width` consecutive
/// samples, taking the maximum (so spikes survive downsampling).
///
/// Returns a String of `width` braille characters.
#[must_use]
pub fn render_braille(series: &[f32], width: usize, max: f32) -> String {
    if width == 0 || series.is_empty() {
        return String::new();
    }
    let max = if max > 0.0 {
        max
    } else {
        series.iter().copied().fold(0.0_f32, f32::max).max(1e-6)
    };

    let mut out = String::with_capacity(width * 3);
    let samples_per_cell = (series.len() as f32 / width as f32).ceil() as usize;
    for cell in 0..width {
        // 2 columns × 4 rows of dots per cell.
        let start = cell * samples_per_cell;
        let end = (start + samples_per_cell).min(series.len());
        if start >= series.len() {
            out.push('\u{2800}'); // empty braille
            continue;
        }
        // Take up to 2 samples for left/right column.
        let chunk = &series[start..end];
        let mid = chunk.len().div_ceil(2);
        let left_max = chunk[..mid].iter().copied().fold(0.0_f32, f32::max);
        let right_max = chunk[mid..].iter().copied().fold(0.0_f32, f32::max);
        let left_dots = ((left_max / max) * 4.0).round().clamp(0.0, 4.0) as u8;
        let right_dots = ((right_max / max) * 4.0).round().clamp(0.0, 4.0) as u8;
        out.push(braille_cell(left_dots, right_dots));
    }
    out
}

/// Build a braille char from (left-column-height, right-column-height) in [0, 4].
/// Braille bit layout (Unicode block U+2800):
///   col0: bit0=top, bit1, bit2, bit6=bottom
///   col1: bit3=top, bit4, bit5, bit7=bottom
fn braille_cell(left: u8, right: u8) -> char {
    const LEFT_BITS:  [u8; 5] = [0, 0b0000_0001, 0b0000_0011, 0b0000_0111, 0b0100_0111];
    const RIGHT_BITS: [u8; 5] = [0, 0b0000_1000, 0b0001_1000, 0b0011_1000, 0b1011_1000];
    let bits = LEFT_BITS[left as usize] | RIGHT_BITS[right as usize];
    char::from_u32(0x2800 + bits as u32).unwrap_or('\u{2800}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_series_returns_empty_string() {
        assert_eq!(render_braille(&[], 8, 100.0), "");
    }

    #[test]
    fn flat_series_renders_consistent_chars() {
        let s = render_braille(&[5.0; 8], 4, 10.0);
        assert_eq!(s.chars().count(), 4);
    }

    #[test]
    fn empty_cell_is_used_for_no_data() {
        let s = render_braille(&[1.0], 4, 10.0);
        // Only first cell has data; rest should be U+2800 (empty braille).
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars.len(), 4);
        assert_eq!(chars[1], '\u{2800}');
        assert_eq!(chars[2], '\u{2800}');
        assert_eq!(chars[3], '\u{2800}');
    }

    #[test]
    fn full_intensity_renders_full_block() {
        // All samples at max → all dots filled → braille pattern with all 8 dots set.
        let s = render_braille(&[10.0; 8], 1, 10.0);
        // Expected: U+28FF (all 8 dots)
        assert_eq!(s, "\u{28FF}");
    }
}
