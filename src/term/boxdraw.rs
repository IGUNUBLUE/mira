//! Box-drawing lines and block elements drawn as rectangles instead of font
//! glyphs, as Alacritty and Ghostty do: they meet exactly at cell edges (a
//! font's `│` leaves gaps when the line is taller than the glyph), so herdr's
//! pane borders and TUI tables join up.

/// A rectangle in cell coordinates (pixels from the cell's top-left), with
/// the opacity of the text color to fill it with (shades are partial).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub alpha: f32,
}

/// Arms of a line character: up, down, left, right; 0 none, 1 light, 2 heavy.
fn arms(c: char) -> Option<[u8; 4]> {
    Some(match c {
        '─' => [0, 0, 1, 1],
        '━' => [0, 0, 2, 2],
        '│' => [1, 1, 0, 0],
        '┃' => [2, 2, 0, 0],
        // Rounded corners are drawn square: close at these sizes, and joined.
        '┌' | '╭' => [0, 1, 0, 1],
        '┐' | '╮' => [0, 1, 1, 0],
        '└' | '╰' => [1, 0, 0, 1],
        '┘' | '╯' => [1, 0, 1, 0],
        '├' => [1, 1, 0, 1],
        '┤' => [1, 1, 1, 0],
        '┬' => [0, 1, 1, 1],
        '┴' => [1, 0, 1, 1],
        '┼' => [1, 1, 1, 1],
        '┏' => [0, 2, 0, 2],
        '┓' => [0, 2, 2, 0],
        '┗' => [2, 0, 0, 2],
        '┛' => [2, 0, 2, 0],
        '┣' => [2, 2, 0, 2],
        '┫' => [2, 2, 2, 0],
        '┳' => [0, 2, 2, 2],
        '┻' => [2, 0, 2, 2],
        '╋' => [2, 2, 2, 2],
        '╴' => [0, 0, 1, 0],
        '╵' => [1, 0, 0, 0],
        '╶' => [0, 0, 0, 1],
        '╷' => [0, 1, 0, 0],
        _ => return None,
    })
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Piece {
    Piece { x, y, w, h, alpha: 1.0 }
}

/// The pieces that draw `c` in a `w × h` cell, or `None` to use the font.
pub fn pieces(c: char, w: f32, h: f32) -> Option<Vec<Piece>> {
    if let Some([up, down, left, right]) = arms(c) {
        let light = (w / 8.0).round().max(1.0);
        let mut out = Vec::with_capacity(4);
        // Each arm reaches the far edge of the crossing, so corners and tees close.
        let v = |t: f32| ((w - t) / 2.0).round();
        let hz = |t: f32| ((h - t) / 2.0).round();
        let thick = |weight: u8| light * f32::from(weight);
        if up > 0 {
            let t = thick(up);
            out.push(rect(v(t), 0.0, t, hz(t) + t));
        }
        if down > 0 {
            let t = thick(down);
            out.push(rect(v(t), hz(t), t, h - hz(t)));
        }
        if left > 0 {
            let t = thick(left);
            out.push(rect(0.0, hz(t), v(t) + t, t));
        }
        if right > 0 {
            let t = thick(right);
            out.push(rect(v(t), hz(t), w - v(t), t));
        }
        return Some(out);
    }
    let eighth = |n: f32| (h * n / 8.0).round();
    let col_eighth = |n: f32| (w * n / 8.0).round();
    let piece = match c {
        '█' => rect(0.0, 0.0, w, h),
        '▀' => rect(0.0, 0.0, w, eighth(4.0)),
        '▔' => rect(0.0, 0.0, w, eighth(1.0)),
        '▁'..='▇' => {
            let n = (c as u32 - '▁' as u32 + 1) as f32;
            rect(0.0, h - eighth(n), w, eighth(n))
        }
        '▐' => rect(col_eighth(4.0), 0.0, w - col_eighth(4.0), h),
        '▕' => rect(w - col_eighth(1.0), 0.0, col_eighth(1.0), h),
        '▉'..='▏' => {
            // ▉ is seven eighths wide, then one eighth less each, down to ▏.
            let n = (7 - (c as u32 - '▉' as u32)) as f32;
            rect(0.0, 0.0, col_eighth(n), h)
        }
        '░' => Piece { alpha: 0.25, ..rect(0.0, 0.0, w, h) },
        '▒' => Piece { alpha: 0.5, ..rect(0.0, 0.0, w, h) },
        '▓' => Piece { alpha: 0.75, ..rect(0.0, 0.0, w, h) },
        _ => return None,
    };
    Some(vec![piece])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_cross_the_whole_cell() {
        let h = pieces('─', 8.0, 18.0).unwrap();
        // The left arm reaches over the crossing column, the right one starts on it.
        assert_eq!(
            h,
            [
                Piece { x: 0.0, y: 9.0, w: 5.0, h: 1.0, alpha: 1.0 },
                Piece { x: 4.0, y: 9.0, w: 4.0, h: 1.0, alpha: 1.0 }
            ]
        );
        let v = pieces('│', 8.0, 18.0).unwrap();
        // Top to bottom: the next row's line starts where this one ends.
        assert_eq!(v[0].y, 0.0);
        assert_eq!(v[1].y + v[1].h, 18.0);
        assert_eq!(pieces('┼', 8.0, 18.0).map(|p| p.len()), Some(4));
        let heavy = pieces('┃', 8.0, 18.0).unwrap();
        assert_eq!(heavy[0].w, 2.0);
    }

    #[test]
    fn corners_meet_at_the_center() {
        let p = pieces('┌', 9.0, 19.0).unwrap();
        let (down, right) = (p[0], p[1]);
        // The arms overlap on the crossing square.
        assert_eq!((down.x, right.y), (4.0, 9.0));
        assert_eq!(right.x, down.x);
        assert_eq!(right.x + right.w, 9.0);
        assert_eq!(down.y + down.h, 19.0);
    }

    #[test]
    fn blocks_and_shades() {
        assert_eq!(pieces('█', 8.0, 16.0), Some(vec![rect(0.0, 0.0, 8.0, 16.0)]));
        assert_eq!(pieces('▄', 8.0, 16.0), Some(vec![rect(0.0, 8.0, 8.0, 8.0)]));
        assert_eq!(pieces('▁', 8.0, 16.0), Some(vec![rect(0.0, 14.0, 8.0, 2.0)]));
        assert_eq!(pieces('▌', 8.0, 16.0), Some(vec![rect(0.0, 0.0, 4.0, 16.0)]));
        assert_eq!(pieces('▏', 8.0, 16.0), Some(vec![rect(0.0, 0.0, 1.0, 16.0)]));
        assert_eq!(pieces('▒', 8.0, 16.0).map(|p| p[0].alpha), Some(0.5));
        assert_eq!(pieces('a', 8.0, 16.0), None);
    }
}
