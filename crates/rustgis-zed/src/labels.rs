//! The map's label layer: the placed labels rasterised into premultiplied RGBA, for the GPU pass.
//!
//! The GPU scene carries `PlacedLabel`s (the same placement the CPU renderer does), not text
//! pixels — as RustGIS's egui app draws them with egui's text engine over the GPU frame, this
//! window draws them with a small rasteriser over the wgpu frame. It uses the **same font files**
//! ([`rustgis_render::fonts::ui_fonts`]) and the same point→pixel scale as RustGIS's own label
//! renderer, so the text matches what the CPU path draws.
//!
//! Drawn here: upright, multi-line labels with their colour and halo, centred/aligned as placed.
//! Not drawn yet: labels turned along a line (`angle`/`glyphs` — the text is left out rather than
//! printed in the wrong place), icons, callout lines and label backgrounds; those stay on the
//! CPU-rendered path.

use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use rustgis_map::Rgba;
use rustgis_render::PlacedLabel;
use tiny_skia::Pixmap;

/// Points to font pixels, as RustGIS's text renderer scales a label's size.
const POINT_TO_PX: f32 = 1.33;

/// The label font's regular and semibold faces, parsed once from the files the renderer uses.
fn font(bold: bool) -> Option<&'static FontRef<'static>> {
    static REGULAR: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    static BOLD: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    let fonts = rustgis_render::fonts::ui_fonts();
    if bold {
        BOLD.get_or_init(|| FontRef::try_from_slice(&fonts.semibold).ok()).as_ref()
    } else {
        REGULAR.get_or_init(|| FontRef::try_from_slice(&fonts.regular).ok()).as_ref()
    }
}

/// The font's em scale for `px` pixels, as `rustgis_render`'s text renderer computes it (the
/// font's height over its units per em: `PxScale::from(px)` alone is not the same size).
fn em(font: &FontRef<'_>, px: f32) -> PxScale {
    let (height, upm) = (font.height_unscaled(), font.units_per_em().unwrap_or(0.0));
    let scale = if height.is_finite() && upm.is_finite() && height > 0.0 && upm > 0.0 {
        height / upm
    } else {
        1.0
    };
    PxScale::from(px * scale)
}

/// One line's shaped glyphs and advance width (pixels).
fn shape(text: &str, px: f32, face: &FontRef<'_>) -> (Vec<(ab_glyph::GlyphId, f32)>, f32) {
    let scaled = face.as_scaled(em(face, px));
    let mut glyphs = Vec::new();
    let mut x = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let id = face.glyph_id(ch);
        if let Some(prev) = previous {
            x += scaled.kern(prev, id);
        }
        glyphs.push((id, x));
        x += scaled.h_advance(id);
        previous = Some(id);
    }
    (glyphs, x)
}

/// Premultiplied source-over: `color` at `coverage` over the premultiplied pixel.
fn blend(pixmap: &mut Pixmap, x: i32, y: i32, color: Rgba, coverage: f32) {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    if x < 0 || y < 0 || x >= w || y >= h || coverage <= 0.0 {
        return;
    }
    let index = (y * w + x) as usize;
    let Some(pixel) = pixmap.pixels_mut().get_mut(index) else { return };
    let alpha = (f32::from(color.a()) / 255.0) * coverage.min(1.0);
    if alpha <= 0.0 {
        return;
    }
    let inverse = 1.0 - alpha;
    let channel = |value: u8, old: u8| (f32::from(value) * alpha + f32::from(old) * inverse).round() as u8;
    let a = (alpha * 255.0 + f32::from(pixel.alpha()) * inverse).round().min(255.0) as u8;
    let r = channel(color.r(), pixel.red()).min(a);
    let g = channel(color.g(), pixel.green()).min(a);
    let b = channel(color.b(), pixel.blue()).min(a);
    if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, a) {
        *pixel = p;
    }
}

/// Draws one line with its centre at (`cx`, `cy`) in premultiplied `color`, moved by `offset`.
fn draw_line(pixmap: &mut Pixmap, text: &str, cx: f32, cy: f32, px: f32, color: Rgba, bold: bool, offset: (f32, f32)) {
    let Some(face) = font(bold) else { return };
    let scaled = face.as_scaled(em(face, px));
    let (glyphs, width) = shape(text, px, face);
    let start = cx - width / 2.0 + offset.0;
    let baseline = cy + (scaled.ascent() + scaled.descent()) / 2.0 + offset.1;
    for (id, advance) in glyphs {
        let glyph = id.with_scale_and_position(em(face, px), ab_glyph::point(start + advance, baseline));
        let Some(outlined) = face.outline_glyph(glyph) else { continue };
        let bounds = outlined.px_bounds();
        outlined.draw(|gx, gy, cov| {
            blend(
                pixmap,
                bounds.min.x as i32 + gx as i32,
                bounds.min.y as i32 + gy as i32,
                color,
                cov,
            );
        });
    }
}

/// Rasterises `labels` for a `width`×`height` logical view at `dpi` device pixels per point.
///
/// Returns premultiplied RGBA at the view's physical size (`width * dpi` × `height * dpi`), or
/// `None` when there is nothing to draw or the size is degenerate.
pub fn rasterize(labels: &[PlacedLabel], width: u32, height: u32, dpi: f32) -> Option<(u32, u32, Vec<u8>)> {
    let ppp = if dpi.is_finite() && dpi > 0.0 { dpi.clamp(0.1, 8.0) } else { 1.0 };
    let (pw, ph) = (
        (f64::from(width) * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32,
        (f64::from(height) * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32,
    );
    if labels.is_empty() {
        return None;
    }
    let mut pixmap = Pixmap::new(pw, ph)?;
    let mut drawn = 0usize;
    for label in labels {
        if label.text.is_empty() || !label.glyphs.is_empty() {
            continue;
        }
        if label.angle.is_some_and(|angle| angle.is_finite() && angle.abs() > 1e-3) {
            // Turned along a line: the small rasteriser does not rotate a block yet.
            continue;
        }
        let px = label.size * POINT_TO_PX * ppp;
        if !px.is_finite() || px <= 0.0 {
            continue;
        }
        let lines: Vec<&str> = label.text.split('\n').take(8).collect();
        let pitch = label.size * 1.33 * 1.25 * ppp;
        let block_width = lines
            .iter()
            .map(|line| font(label.bold).map_or(0.0, |face| shape(line, px, face).1))
            .fold(0.0f32, f32::max);
        // `x` is where `align` says: 0 the text's centre, -0.5 its left edge, 0.5 its right edge.
        let centre = label.x * ppp - label.align * block_width;
        let mut halos: Vec<(f32, f32)> = Vec::new();
        if let Some(_halo) = label.halo {
            let reach = match label.halo_width {
                Some(width) => (rustgis_render::halo_reach(width) * POINT_TO_PX * ppp).min(12.0),
                None => 1.2 * ppp,
            };
            // A ring of 8 offsets: the text drawn round itself, as the renderer's thin halo.
            for (dx, dy) in [
                (-1.0f32, 0.0),
                (1.0, 0.0),
                (0.0, -1.0),
                (0.0, 1.0),
                (-0.7, -0.7),
                (0.7, 0.7),
                (-0.7, 0.7),
                (0.7, -0.7),
            ] {
                halos.push((dx * reach, dy * reach));
            }
        }
        let top = label.y * ppp - (lines.len().saturating_sub(1) as f32) * pitch / 2.0;
        if let Some(halo) = label.halo {
            for (index, line) in lines.iter().enumerate() {
                let cy = top + index as f32 * pitch;
                for (dx, dy) in &halos {
                    draw_line(&mut pixmap, line, centre, cy, px, halo, label.bold, (*dx, *dy));
                }
            }
        }
        for (index, line) in lines.iter().enumerate() {
            let cy = top + index as f32 * pitch;
            draw_line(&mut pixmap, line, centre, cy, px, label.color, label.bold, (0.0, 0.0));
        }
        drawn += 1;
    }
    if drawn == 0 {
        return None;
    }
    Some((pw, ph, pixmap.data().to_vec()))
}
