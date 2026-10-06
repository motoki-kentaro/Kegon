//! Generates Kegon's application icon files from the master SVG.
//!
//! Run from the repository root:
//!
//! ```sh
//! cargo run --manifest-path tools/icon-gen/Cargo.toml
//! ```
//!
//! Input:  `assets/icons/kegon/kegon.svg` (the master artwork, never modified)
//! Output: `assets/icons/kegon/kegon-app-icon.svg` and `assets/icons/kegon/kegon.ico`
//!
//! The master artwork is a near-black silhouette on a transparent background,
//! which disappears on dark taskbars and dark-mode Explorer. The app icon
//! therefore places it on a light rounded plate, cropped tightly to the
//! artwork so that small sizes stay as large as possible.

use std::fs;
use std::path::{Path, PathBuf};

use resvg::{tiny_skia, usvg};

/// Sizes stored in the `.ico`, in pixels.
const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

const PLATE_COLOR: &str = "#f3f3f3";
/// Corner radius of the plate, relative to the icon size.
const PLATE_RADIUS: f32 = 0.18;
/// Padding between the plate edge and the artwork, relative to the artwork
/// size. Small icons use less padding so the artwork keeps more pixels.
const PADDING_LARGE: f32 = 0.10;
const PADDING_SMALL: f32 = 0.06;
const SMALL_ICON_MAX: u32 = 32;

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/kegon");
    let master = fs::read_to_string(dir.join("kegon.svg")).expect("read kegon.svg");

    let artwork = Artwork::parse(&master);

    let app_icon_svg = artwork.app_icon_svg(PADDING_LARGE);
    write(&dir.join("kegon-app-icon.svg"), app_icon_svg.as_bytes());

    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in ICO_SIZES {
        let padding = if size <= SMALL_ICON_MAX {
            PADDING_SMALL
        } else {
            PADDING_LARGE
        };
        let rgba = rasterize(&artwork.app_icon_svg(padding), size);
        let image = ico::IconImage::from_rgba_data(size, size, rgba);

        // PNG for 256 px keeps the file small; BMP for the rest is the most
        // widely supported encoding.
        let entry = if size >= 256 {
            ico::IconDirEntry::encode_as_png(&image)
        } else {
            ico::IconDirEntry::encode_as_bmp(&image)
        }
        .expect("encode icon entry");
        icon_dir.add_entry(entry);
    }

    let mut bytes = Vec::new();
    icon_dir.write(&mut bytes).expect("encode kegon.ico");
    write(&dir.join("kegon.ico"), &bytes);
}

/// The master artwork: a single path, plus its bounding box.
struct Artwork {
    path_data: String,
    fill: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Artwork {
    fn parse(svg: &str) -> Self {
        let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default())
            .expect("parse kegon.svg");
        let bounds = tree.root().abs_bounding_box();

        Self {
            path_data: attribute(svg, "d"),
            fill: attribute(svg, "fill"),
            x: bounds.x(),
            y: bounds.y(),
            width: bounds.width(),
            height: bounds.height(),
        }
    }

    /// The artwork centered on a rounded plate.
    fn app_icon_svg(&self, padding: f32) -> String {
        let side = self.width.max(self.height) * (1.0 + 2.0 * padding);
        let x = self.x + self.width / 2.0 - side / 2.0;
        let y = self.y + self.height / 2.0 - side / 2.0;
        let radius = side * PLATE_RADIUS;

        format!(
            concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{x} {y} {side} {side}">"#,
                r#"<rect x="{x}" y="{y}" width="{side}" height="{side}" rx="{radius}" fill="{plate}"/>"#,
                r#"<path d="{d}" fill="{fill}" fill-rule="evenodd"/>"#,
                "</svg>\n",
            ),
            x = x,
            y = y,
            side = side,
            radius = radius,
            plate = PLATE_COLOR,
            d = self.path_data,
            fill = self.fill,
        )
    }
}

/// Extracts the value of the first `name="..."` attribute on the `<path>`.
fn attribute(svg: &str, name: &str) -> String {
    let path = &svg[svg.find("<path").expect("kegon.svg has a <path>")..];
    let needle = format!(" {name}=\"");
    let start = path.find(&needle).expect("attribute present") + needle.len();
    let end = start + path[start..].find('"').expect("attribute closed");
    path[start..end].to_owned()
}

fn rasterize(svg: &str, size: u32) -> Vec<u8> {
    let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default())
        .expect("parse generated svg");
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("pixmap");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // tiny-skia stores premultiplied alpha; ICO expects straight alpha.
    pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect()
}

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    println!("wrote {}", path.display());
}
