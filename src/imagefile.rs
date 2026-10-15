//! Image tabs: which files open as an image, and reading one. Only the
//! header is decoded here (size and format); iced decodes the pixels when it
//! first draws them, and frees them once they are on the GPU.

use std::io::{Cursor, Read};
use std::path::Path;

/// Largest image file opened (bytes).
pub const MAX_BYTES: u64 = 32 * 1024 * 1024;
/// Largest image shown, in pixels: decoded, it takes 4 bytes per pixel.
pub const MAX_PIXELS: u64 = 50_000_000;

/// Raster formats mira decodes (the `image` features in Cargo.toml).
const RASTER: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Raster,
    /// Opened as text (it is source); its preview is an image tab.
    Svg,
}

/// How `path` opens as an image, by its extension.
pub fn kind(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "svg" { Some(Kind::Svg) } else { RASTER.contains(&ext.as_str()).then_some(Kind::Raster) }
}

/// An image file read for a tab.
pub struct Loaded {
    pub bytes: Vec<u8>,
    /// Pixel size (`None` for SVG: it scales).
    pub size: Option<(u32, u32)>,
}

/// Read `path` as an image of `kind`: at most [`MAX_BYTES`], and for raster
/// images a header that decodes to at most [`MAX_PIXELS`].
pub fn load(path: &Path, kind: Kind) -> Result<Loaded, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("image over {} MB", MAX_BYTES / (1024 * 1024)));
    }
    let size = match kind {
        Kind::Svg => None,
        Kind::Raster => {
            let (w, h) = image::ImageReader::new(Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|e| e.to_string())?
                .into_dimensions()
                .map_err(|e| format!("not a readable image ({e})"))?;
            if u64::from(w) * u64::from(h) > MAX_PIXELS {
                return Err(format!("{w}×{h} is too large to preview"));
            }
            Some((w, h))
        }
    };
    Ok(Loaded { bytes, size })
}

/// `n` bytes as `512 B`, `12.3 KB`, `4.5 MB`.
pub fn human_bytes(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_by_extension() {
        assert_eq!(kind(Path::new("a/shot.PNG")), Some(Kind::Raster));
        assert_eq!(kind(Path::new("photo.jpeg")), Some(Kind::Raster));
        assert_eq!(kind(Path::new("icon.svg")), Some(Kind::Svg));
        assert_eq!(kind(Path::new("main.rs")), None);
        assert_eq!(kind(Path::new("png")), None);
    }

    #[test]
    fn reads_the_size_from_the_header() {
        let dir = crate::files::test_dir("imagefile");
        let png = dir.join("dot.png");
        // A 3×2 RGBA PNG, written with the same crate.
        image::RgbaImage::new(3, 2).save(&png).unwrap();
        let loaded = load(&png, Kind::Raster).unwrap();
        assert_eq!(loaded.size, Some((3, 2)));
        assert!(!loaded.bytes.is_empty());
        // Not an image, whatever its name.
        let fake = dir.join("fake.png");
        std::fs::write(&fake, "hello").unwrap();
        assert!(load(&fake, Kind::Raster).is_err());
        // SVG is read as is.
        let svg = dir.join("i.svg");
        std::fs::write(&svg, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        assert_eq!(load(&svg, Kind::Svg).unwrap().size, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(12_595), "12.3 KB");
        assert_eq!(human_bytes(4_718_592), "4.5 MB");
    }
}
