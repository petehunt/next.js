//! `next/image` from Rust.
//!
//! The optimizer itself is a Next route (`/_next/image`), and its URL contract
//! is simple and stable: `?url=<encoded>&w=<width>&q=<quality>`. What a Rust
//! island lacks is not the ability to *serve* an optimized image but the
//! build-time knowledge that `<Image src={logo}>` gets from a static import —
//! the resolved path and the intrinsic dimensions.
//!
//! So the build emits that knowledge as [`Img`] constants, and this module
//! reproduces the markup `next/image` would have produced from them. The
//! optimizer, the cache, and the served bytes are all the same; only the code
//! that writes the `<img>` tag differs.
//!
//! Width selection mirrors Next's own rules, because a mismatch would mean
//! requesting sizes the optimizer has not warmed and quietly halving the cache
//! hit rate.

use std::fmt::Write as _;

/// An image the build resolved: public path plus intrinsic size.
#[derive(Debug, Clone, Copy)]
pub struct Img {
    /// Path under `public/`, e.g. `/rust-images/nasa.png`.
    pub src: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Next's default `deviceSizes`, for images that declare `sizes`.
const DEVICE_SIZES: &[u32] = &[640, 750, 828, 1080, 1200, 1920, 2048, 3840];
/// Next's default `imageSizes`, for fixed-width images.
const IMAGE_SIZES: &[u32] = &[16, 32, 48, 64, 96, 128, 256, 384];

const DEFAULT_QUALITY: u8 = 75;

fn all_sizes() -> Vec<u32> {
    let mut v: Vec<u32> = IMAGE_SIZES.iter().chain(DEVICE_SIZES.iter()).copied().collect();
    v.sort_unstable();
    v
}

/// Smallest allowed width at or above `target`, falling back to the largest.
fn snap(target: u32) -> u32 {
    let sizes = all_sizes();
    sizes.iter().copied().find(|s| *s >= target).unwrap_or_else(|| *sizes.last().unwrap())
}

/// Percent-encodes a path for the `url` query parameter.
///
/// Only the characters that actually matter in a query value are escaped; over-
/// escaping would produce a different URL string than the JS side and split the
/// optimizer's cache.
fn encode(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 8);
    for b in src.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*'
            | b'\'' | b'(' | b')' => out.push(b as char),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

pub fn optimized_url(src: &str, width: u32, quality: u8) -> String {
    format!("/_next/image?url={}&w={}&q={}", encode(src), width, quality)
}

/// How the image participates in layout, which decides the `srcset` shape.
#[derive(Debug, Clone, Copy)]
pub enum Sizing<'a> {
    /// Rendered at a known CSS width. Emits a `1x`/`2x` descriptor pair, which
    /// is what `next/image` does when no `sizes` attribute is given.
    Fixed { width: u32, height: u32 },
    /// Width depends on the viewport. Emits a `w`-descriptor srcset across the
    /// device sizes, plus the `sizes` attribute itself.
    Responsive(&'a str),
}

#[derive(Debug, Clone)]
pub struct ImageProps<'a> {
    pub alt: &'a str,
    pub class: Option<&'a str>,
    pub sizing: Sizing<'a>,
    pub quality: u8,
    /// `loading="lazy"` unless this is above the fold.
    pub priority: bool,
}

impl<'a> Default for ImageProps<'a> {
    fn default() -> Self {
        ImageProps {
            alt: "",
            class: None,
            sizing: Sizing::Responsive("100vw"),
            quality: DEFAULT_QUALITY,
            priority: false,
        }
    }
}

/// Renders the `<img>` tag `next/image` would render.
pub fn render(img: &Img, props: &ImageProps<'_>) -> String {
    let q = props.quality;

    let (srcset, src, width, height, sizes_attr) = match props.sizing {
        Sizing::Fixed { width, height } => {
            let one = snap(width);
            let two = snap(width.saturating_mul(2));
            let srcset = format!(
                "{} 1x, {} 2x",
                optimized_url(img.src, one, q),
                optimized_url(img.src, two, q)
            );
            // Next points `src` at the densest candidate so a browser without
            // srcset support still gets a usable image.
            (srcset, optimized_url(img.src, two, q), width, height, None)
        }
        Sizing::Responsive(sizes) => {
            let srcset = DEVICE_SIZES
                .iter()
                .map(|w| format!("{} {}w", optimized_url(img.src, *w, q), w))
                .collect::<Vec<_>>()
                .join(", ");
            let largest = *DEVICE_SIZES.last().unwrap();
            (srcset, optimized_url(img.src, largest, q), img.width, img.height, Some(sizes))
        }
    };

    let mut out = String::with_capacity(srcset.len() + 256);
    out.push_str("<img alt=\"");
    escape_attr_into(&mut out, props.alt);
    out.push('"');

    if let Some(class) = props.class {
        out.push_str(" class=\"");
        escape_attr_into(&mut out, class);
        out.push('"');
    }

    let _ = write!(out, " width=\"{width}\" height=\"{height}\"");

    if props.priority {
        out.push_str(" fetchpriority=\"high\" loading=\"eager\"");
    } else {
        out.push_str(" loading=\"lazy\"");
    }
    out.push_str(" decoding=\"async\"");

    if let Some(sizes) = sizes_attr {
        out.push_str(" sizes=\"");
        escape_attr_into(&mut out, sizes);
        out.push('"');
    }

    out.push_str(" srcset=\"");
    escape_attr_into(&mut out, &srcset);
    out.push_str("\" src=\"");
    escape_attr_into(&mut out, &src);
    out.push_str("\">");
    out
}

fn escape_attr_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

#[cfg(feature = "maud")]
impl maud::Render for Img {
    fn render_to(&self, buf: &mut String) {
        buf.push_str(&render(self, &ImageProps::default()));
    }
}

/// Convenience for the common fixed-size logo case.
pub fn fixed(img: &Img, alt: &str, class: &str, width: u32, height: u32) -> maud::PreEscaped<String> {
    maud::PreEscaped(render(
        img,
        &ImageProps {
            alt,
            class: Some(class),
            sizing: Sizing::Fixed { width, height },
            ..Default::default()
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGO: Img = Img { src: "/rust-images/nasa.png", width: 586, height: 708 };

    #[test]
    fn fixed_images_get_a_1x_2x_pair() {
        let html = render(
            &LOGO,
            &ImageProps { sizing: Sizing::Fixed { width: 100, height: 120 }, ..Default::default() },
        );
        // 100 snaps up to 128, 200 snaps up to 256.
        assert!(html.contains("w=128&amp;q=75 1x"), "{html}");
        assert!(html.contains("w=256&amp;q=75 2x"), "{html}");
        assert!(html.contains("loading=\"lazy\""));
    }

    #[test]
    fn responsive_images_get_w_descriptors_and_sizes() {
        let html = render(
            &LOGO,
            &ImageProps { sizing: Sizing::Responsive("(min-width: 768px) 50vw, 100vw"), ..Default::default() },
        );
        assert!(html.contains("sizes=\"(min-width: 768px) 50vw, 100vw\""), "{html}");
        assert!(html.contains("w=640&amp;q=75 640w"), "{html}");
        assert!(html.contains("w=3840&amp;q=75 3840w"), "{html}");
    }

    #[test]
    fn ampersands_are_escaped_in_the_attribute() {
        let html = render(&LOGO, &ImageProps::default());
        assert!(!html.contains("&w="), "raw & in an attribute is invalid HTML:\n{html}");
    }

    #[test]
    fn the_url_matches_what_next_image_builds() {
        assert_eq!(
            optimized_url("/rust-images/nasa.png", 640, 75),
            "/_next/image?url=%2Frust-images%2Fnasa.png&w=640&q=75"
        );
    }

    #[test]
    fn priority_images_are_eager() {
        let html = render(&LOGO, &ImageProps { priority: true, ..Default::default() });
        assert!(html.contains("fetchpriority=\"high\""), "{html}");
        assert!(!html.contains("loading=\"lazy\""), "{html}");
    }
}
