//! CPU-only title image, allocated once when a native drag starts.
use super::FontLibrary;
use swash::scale::{Render, ScaleContext, Source};
use swash::shape::ShapeContext;

pub struct DragImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Uses the separate UI font and the existing fallback registry. The bounded
/// image does not allocate a terminal surface or change terminal font metrics.
pub fn render(
    library: &FontLibrary,
    title: &str,
    family: Option<&str>,
    size: f32,
    width: u32,
    height: u32,
    background: [f32; 4],
    foreground: [f32; 4],
) -> Option<DragImage> {
    if !size.is_finite() || !(1.0..=256.0).contains(&size) {
        return None;
    }
    if !(24..=4096).contains(&width) || !(20..=256).contains(&height) {
        return None;
    }
    let primary = family
        .and_then(|f| library.register_font_family(f))
        .unwrap_or(0);
    let rgb = |c: [f32; 4]| c.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
    let bg = rgb(background);
    let fg = rgb(foreground);
    let mut rgba = vec![0; width as usize * height as usize * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
    }
    let mut runs: Vec<(usize, String)> = Vec::new();
    let title: String = title
        .chars()
        .take(4096)
        .filter(|c| !c.is_control())
        .collect();
    for cluster in rio_unicode::grapheme::Graphemes::new(&title) {
        let covered =
            library
                .inner
                .read()
                .get_data(&primary)
                .is_some_and(|(data, offset, key)| {
                    let map = swash::FontRef {
                        data: data.as_ref(),
                        offset,
                        key,
                    }
                    .charmap();
                    cluster.chars().all(|ch| map.map(ch) != 0)
                });
        let id = if covered {
            primary
        } else {
            cluster
                .chars()
                .map(|ch| {
                    library
                        .resolve_font_for_char(ch, &Default::default(), None)
                        .0
                })
                .find(|id| {
                    library.inner.read().get_data(id).is_some_and(
                        |(data, offset, key)| {
                            let map = swash::FontRef {
                                data: data.as_ref(),
                                offset,
                                key,
                            }
                            .charmap();
                            cluster.chars().all(|ch| map.map(ch) != 0)
                        },
                    )
                })
                .unwrap_or(primary)
        };
        if let Some((_, text)) = runs.last_mut().filter(|(font, _)| *font == id) {
            text.push_str(cluster);
        } else {
            runs.push((id, cluster.to_string()));
        }
    }
    let mut shaping = ShapeContext::new();
    let mut scaling = ScaleContext::new();
    let mut pen = 8.0;
    for (id, text) in runs {
        let Some((data, offset, key)) = library.inner.read().get_data(&id) else {
            continue;
        };
        let font = swash::FontRef {
            data: data.as_ref(),
            offset,
            key,
        };
        let mut shaper = shaping.builder(font).size(size).build();
        shaper.add_str(&text);
        let metrics = font.metrics(&[]).scale(size);
        let baseline = (height as f32 + metrics.ascent - metrics.descent) / 2.0;
        let mut scaler = scaling.builder(font).size(size).hint(true).build();
        shaper.shape_with(|cluster| {
            for glyph in cluster.glyphs {
                if let Some(image) =
                    Render::new(&[Source::Outline]).render(&mut scaler, glyph.id)
                {
                    let left = (pen + glyph.x).round() as i32 + image.placement.left;
                    let top = (baseline - glyph.y).round() as i32 - image.placement.top;
                    for y in 0..image.placement.height as i32 {
                        for x in 0..image.placement.width as i32 {
                            let (dx, dy) = (left + x, top + y);
                            if dx < 4
                                || dx >= width as i32 - 4
                                || dy < 1
                                || dy >= height as i32 - 1
                            {
                                continue;
                            }
                            let alpha = image.data
                                [(y as u32 * image.placement.width + x as u32) as usize]
                                as u32;
                            let pixel = &mut rgba
                                [((dy as u32 * width + dx as u32) * 4) as usize..][..4];
                            for channel in 0..3 {
                                pixel[channel] = ((fg[channel] as u32 * alpha
                                    + pixel[channel] as u32 * (255 - alpha))
                                    / 255)
                                    as u8;
                            }
                        }
                    }
                }
                pen += glyph.advance;
            }
        });
        if pen >= width as f32 {
            break;
        }
    }
    for y in 0..height {
        for x in 0..width {
            if x == 0 || y == 0 || x == width - 1 || y == height - 1 {
                rgba[((y * width + x) * 4) as usize..][..3].copy_from_slice(&fg[..3]);
            }
        }
    }
    Some(DragImage {
        rgba,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_image_is_bounded_and_renders_mixed_clusters() {
        let (library, _) = FontLibrary::new(Default::default());
        let bg = [0.1, 0.2, 0.3, 1.0];
        let fg = [1.0; 4];
        let title = "Shell · Проект · 日本語 · e\u{301}";
        let image = render(&library, title, None, 16.0, 320, 32, bg, fg).unwrap();
        assert_eq!(image.rgba.len(), 320 * 32 * 4);
        assert!(image.rgba.chunks_exact(4).enumerate().any(|(i, p)| {
            let (x, y) = (i % 320, i / 320);
            x > 4 && x < 315 && y > 2 && y < 29 && p[0] > 100
        }));
        assert!(render(&library, title, None, 16.0, u32::MAX, 32, bg, fg).is_none());
        assert!(render(&library, title, None, f32::NAN, 320, 32, bg, fg).is_none());
    }
}
