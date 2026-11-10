//! The map's label layer: the placed labels rasterised into premultiplied RGBA, for the GPU pass.
//!
//! The GPU scene carries `PlacedLabel`s — the placement the CPU renderer does — not text pixels.
//! RustGIS's egui app draws them with egui's text engine over the GPU frame; this window draws
//! them with the renderer's own text code over the wgpu frame.
//!
//! # Where this drawing comes from
//!
//! The drawing below is **ported from RustGIS 0.6.0**, `crates/render/src/text.rs` — the code
//! `rustgis_render::render_placed` draws a map's labels with, so a settled GPU frame's labels are
//! the CPU/export path's labels. RustGIS is copied verbatim into `crates/rustgis` (Apache-2.0,
//! see `LICENSE-RUSTGIS`); the ported functions are `draw` (`text.rs:621`), `draw_callout`
//! (`:645`), `draw_turned` (`:674`), `draw_upright` (`:702`), `drawn_halo_reach` (`:844`),
//! `spread` and its `Edt` (`:865`, `:894`), `frame` (`:960`), `draw_text` (`:505`),
//! `styled_extent`'s slant (`:526`), `styled_at` (`:554`), `slanted_glyph` (`:589`),
//! `draw_background` (`:463`), `frame_box` (`:478`), `background_box` (`:441`),
//! `background_reach` (`:454`), `label_face`/`label_width`/`spaced_block_width`/`line_pitch`/
//! `block_height` (`:363`–`:434`), `shape`/`glyph`/`cjk`/`cjk_order`/`invisible`/`em`/
//! `sans_beside`/`leans` (`:139`–`:241`), `font`/`face_font`/`family_font` (`:15`, `:86`, `:94`)
//! and `rounded_rect` (`layout.rs:512`).
//!
//! **Modified** for a host that draws the map with wgpu instead of egui:
//!
//! * the crate-private `Canvas` (a `Pixmap` plus a vector-recording mode) becomes a plain
//!   [`tiny_skia::Pixmap`]: this host only ever wants pixels, never a vector drawing;
//! * the serif face table is dropped — `text::label_face` never asks for the serif, and
//!   `rustgis_render` does not export its `Face` type, so the face here is a local subset
//!   (bold, italic, named family);
//! * `text.rs`'s width caches are left out: a label layer is rasterised once per settled frame,
//!   not once per glyph of a many-page layout;
//! * [`rasterize`] draws the unplaced labels before the placed ones (the order `render_placed`
//!   draws them in) into one RGBA pixmap and hands it to the GPU as a texture.
//!
//! **Not drawn here: icons.** A label's `icon` is already in the frame the GPU draws: upstream's
//! `render_frame` calls `draw_icon` into the pixel accumulation before a GPU scene is built
//! (`render/src/lib.rs:4517`, `:4585`), so the base GPU frame carries every icon. Drawing them
//! again in this layer would double them.

use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, VariableFont};
use rustgis_map::Rgba;
use rustgis_map::layout::TextAlign;
use rustgis_render::PlacedLabel;
use rustgis_render::families;
use rustgis_render::fonts::{self, CjkRegion};
use tiny_skia::{FillRule, FilterQuality, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

// ---- fonts and faces (`text.rs:15`–`:158`) ----

/// The label font's regular or semibold face, parsed once from the files the renderer uses.
fn font(bold: bool) -> Option<&'static FontRef<'static>> {
    static REGULAR: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    static BOLD: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    let files = fonts::ui_fonts();
    if bold {
        BOLD.get_or_init(|| {
            let mut face = FontRef::try_from_slice(&files.semibold).ok()?;
            // Heavier where the face is variable (macOS's system font); static ones ignore it.
            face.set_variation(b"wght", fonts::SEMIBOLD_WEIGHT);
            Some(face)
        })
        .as_ref()
    } else {
        REGULAR.get_or_init(|| FontRef::try_from_slice(&files.regular).ok()).as_ref()
    }
}

/// The family and style a label is drawn in (`text.rs:35`, without the serif: labels never use
/// it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Face {
    bold: bool,
    italic: bool,
    /// An installed family, by its place in [`families::installed`] (a label class's font).
    family: Option<u16>,
}

/// The face a label is drawn in (`text.rs:380`).
fn label_face(l: &PlacedLabel) -> Face {
    Face { bold: l.bold, italic: l.italic, family: l.font }
}

/// The face `face` is drawn in, and whether it is slanted by hand: a real italic face where the
/// family has one, else the upright face slanted (`text.rs:86` for the label font's path).
fn face_font(face: Face) -> Option<(&'static FontRef<'static>, bool)> {
    let named = face.family.and_then(|i| families::installed().get(usize::from(i)));
    named.and_then(|fam| family_font(fam, face)).or_else(|| font(face.bold).map(|f| (f, face.italic)))
}

/// The face of an installed family that draws `face`'s weight and slope, parsed once and kept,
/// and whether it is slanted by hand (the family has no italic face) (`text.rs:94`).
fn family_font(fam: &families::Family, face: Face) -> Option<(&'static FontRef<'static>, bool)> {
    type Parsed = std::collections::HashMap<(std::path::PathBuf, u32, bool), Option<&'static FontRef<'static>>>;
    static PARSED: OnceLock<std::sync::Mutex<Parsed>> = OnceLock::new();
    let (installed, slanted) = families::pick(fam, face.bold, face.italic)?;
    let heavier = families::heavier(installed, face.bold);
    let key = (installed.path.clone(), installed.index, heavier);
    let lock = PARSED.get_or_init(Default::default);
    if let Some(f) = lock.lock().ok()?.get(&key) {
        return f.map(|f| (f, slanted));
    }
    // Read and parsed with the lock let go: a file can be 48 MB, and other text is drawn meanwhile.
    let read = families::bytes(installed).and_then(|b| FontRef::try_from_slice_and_index(b, installed.index).ok());
    let mut parsed = lock.lock().ok()?;
    if let Some(f) = parsed.get(&key) {
        return f.map(|f| (f, slanted));
    }
    let f = read.map(|mut f| {
        if heavier {
            f.set_variation(b"wght", families::BOLD_WEIGHT);
        }
        &*Box::leak(Box::new(f))
    });
    parsed.insert(key, f);
    f.map(|f| (f, slanted))
}

/// The label font of `face`'s weight, for the characters `f` lacks: `None` when `f` is it
/// (`text.rs:139`).
fn sans_beside(f: &'static FontRef<'static>, face: Face) -> Option<&'static FontRef<'static>> {
    font(face.bold).filter(|s| !std::ptr::eq(*s, f))
}

/// Whether a glyph drawn from `g` leans: the whole line does when its face is slanted by hand
/// (`slanted`), and in a real italic face `f` a glyph taken from another face is slanted to match
/// it (`text.rs:146`).
fn leans(face: Face, f: &FontRef<'static>, slanted: bool, g: &FontRef<'static>) -> bool {
    slanted || (face.italic && !std::ptr::eq(f, g))
}

/// The scale that draws `f` with an em `px` pixels tall, as the map view draws it (`text.rs:153`).
fn em(f: &FontRef, px: f32) -> PxScale {
    let (h, upm) = (f.height_unscaled(), f.units_per_em().unwrap_or(0.0));
    let k = if h.is_finite() && upm.is_finite() && h > 0.0 && upm > 0.0 { h / upm } else { 1.0 };
    PxScale::from(px * k)
}

/// Characters that never need a glyph (`text.rs:161`).
fn invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c, '\u{AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{E0100}'..='\u{E01EF}')
}

/// An installed CJK face for `region`, parsed once (`text.rs:168`).
fn cjk(region: CjkRegion) -> Option<&'static FontRef<'static>> {
    static FACES: [OnceLock<Option<FontRef<'static>>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let slot = match region {
        CjkRegion::SimplifiedChinese => &FACES[0],
        CjkRegion::Japanese => &FACES[1],
        CjkRegion::Korean => &FACES[2],
    };
    slot.get_or_init(|| fonts::cjk_font(region).and_then(|(b, i)| FontRef::try_from_slice_and_index(b, *i).ok())).as_ref()
}

/// The CJK faces to try for `c`, the region whose glyph style suits it first (`text.rs:180`).
fn cjk_order(c: char) -> [CjkRegion; 3] {
    use CjkRegion::{Japanese as Jp, Korean as Kr, SimplifiedChinese as Sc};
    match c {
        '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{A960}'..='\u{A97F}' | '\u{AC00}'..='\u{D7FF}' => [Kr, Sc, Jp],
        '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}' => [Jp, Sc, Kr],
        _ => [Sc, Jp, Kr],
    }
}

/// The face and glyph that draw `c` (`text.rs:192`).
fn glyph(c: char, f: &'static FontRef<'static>, sans: Option<&'static FontRef<'static>>) -> Option<(&'static FontRef<'static>, GlyphId)> {
    let id = f.glyph_id(c);
    if id.0 != 0 || invisible(c) {
        return (id.0 != 0).then_some((f, id));
    }
    if let Some(s) = sans {
        let id = s.glyph_id(c);
        if id.0 != 0 {
            return Some((s, id));
        }
    }
    cjk_order(c).into_iter().filter_map(cjk).find_map(|f| {
        let id = f.glyph_id(c);
        (id.0 != 0).then_some((f, id))
    })
}

/// Lays out `text` at `px` pixels in `face` (`text.rs:219`): each drawable glyph with its face,
/// pen position (from 0) and character, and the total advance.
fn shape(text: &str, px: f32, face: &'static FontRef<'static>, sans: Option<&'static FontRef<'static>>) -> (Vec<(&'static FontRef<'static>, GlyphId, f32, char)>, f32) {
    let mut out = Vec::new();
    let mut x = 0.0;
    let mut prev: Option<(&'static FontRef<'static>, GlyphId)> = None;
    for ch in text.chars().take(500) {
        let Some((f, id)) = glyph(ch, face, sans) else { continue };
        let sf = f.as_scaled(em(f, px));
        if let Some((pf, pid)) = prev
            && std::ptr::eq(pf, f)
        {
            x += sf.kern(pid, id);
        }
        out.push((f, id, x, ch));
        x += sf.h_advance(id);
        prev = Some((f, id));
    }
    (out, x)
}

// ---- measurement (`text.rs:243`–`:460`) ----

/// Most lines a label draws (`text.rs:244`).
const MAX_LINES: usize = 200;

/// The lines of a label, split at each `\n` (`text.rs:248`).
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').take(MAX_LINES)
}

/// How many lines a label draws (`text.rs:253`).
fn line_count(text: &str) -> usize {
    lines(text).count()
}

/// The distance between a label's lines in points (`text.rs:363`).
fn line_pitch(size: f32) -> f32 {
    size * 1.33 * 1.25
}

/// A label's height in points, for its collision box (`text.rs:369`).
fn block_height(text: &str, size: f32) -> f32 {
    size * 1.25 + line_count(text).saturating_sub(1) as f32 * line_pitch(size)
}

/// Whether a label is slanted or letter-spaced (`text.rs:375`).
fn styled(l: &PlacedLabel) -> bool {
    l.italic || l.spacing != 0.0 || l.font.is_some()
}

/// A line's advance in points at `size` in `face` (`text.rs:275`, without the width caches).
fn measure_in(text: &str, size: f32, face: Face) -> f32 {
    match face_font(face) {
        Some((f, _)) => shape(text, size * 1.33, f, sans_beside(f, face)).1,
        None => text.chars().count() as f32 * size * 0.6,
    }
}

/// How many glyphs a line is drawn with in `face` (`text.rs:405`; shaped each time, uncached).
fn letters_of(line: &str, face: Face) -> usize {
    match face_font(face) {
        Some((f, _)) => shape(line, 12.0, f, sans_beside(f, face)).0.len(),
        None => line.chars().count(),
    }
}

/// [`measure_in`] of a line of `size` in `face`, `spacing` points between its letters, in pixels
/// at `ppp` (`text.rs:392`).
fn spaced_line_width(line: &str, size: f32, face: Face, spacing: f32, ppp: f32) -> f32 {
    let advance = measure_in(line, size, face) * ppp;
    if spacing == 0.0 {
        return advance;
    }
    let letters = letters_of(line, face);
    (advance + spacing * 1.33 * ppp * letters.saturating_sub(1) as f32).max(0.0)
}

/// A line of `l` as it is laid out, in pixels at `ppp` (`text.rs:387`).
fn line_width(l: &PlacedLabel, line: &str, ppp: f32) -> f32 {
    spaced_line_width(line, l.size, label_face(l), l.spacing, ppp)
}

/// A label's width in points: its widest line (`text.rs:434`).
fn label_width(l: &PlacedLabel) -> f32 {
    lines(&l.text).map(|line| line_width(l, line, 1.0)).fold(0.0, f32::max)
}

/// The rectangle a label's background fills, in points (`text.rs:441`).
fn background_box(l: &PlacedLabel) -> Option<[f32; 4]> {
    let b = l.background.as_ref().filter(|b| b.draws() && l.glyphs.is_empty() && !l.text.is_empty())?;
    let w = label_width(l);
    let h = l.size * 1.33 * 1.2 + line_count(&l.text).saturating_sub(1) as f32 * line_pitch(l.size);
    let (px, py) = (b.padding[0] * 1.33, b.padding[1] * 1.33);
    let cx = l.center_x();
    let r = [cx - w / 2.0 - px, l.y - h / 2.0 - py, cx + w / 2.0 + px, l.y + h / 2.0 + py];
    r.iter().all(|v| v.is_finite()).then_some(r)
}

/// How far a label's background reaches past the box its text collides with (`text.rs:454`).
fn background_reach(l: &PlacedLabel) -> [f32; 2] {
    let (Some([x0, y0, _, y1]), Some(b)) = (background_box(l), l.background.as_ref()) else { return [0.0; 2] };
    let outline = if b.stroke.is_some_and(|s| s.a() > 0) { b.stroke_width * 1.33 / 2.0 } else { 0.0 };
    let across = l.center_x() - label_width(l) / 2.0 - x0;
    let up_down = (y1 - y0) / 2.0 - block_height(&l.text, l.size) / 2.0;
    [across, up_down].map(|r| (r + outline).max(0.0))
}

/// The selection frame [`draw_upright`] puts round a label, in points (`text.rs:478`).
fn frame_box(l: &PlacedLabel) -> [f32; 4] {
    let (w, pad) = (label_width(l), 3.0);
    let h = l.size * 1.33 * 1.2 + line_count(&l.text).saturating_sub(1) as f32 * line_pitch(l.size);
    let cx = l.center_x();
    [cx - w / 2.0 - pad, l.y - h / 2.0 - pad, cx + w / 2.0 + pad, l.y + h / 2.0 + pad]
}

// ---- pixels (`text.rs:485`–`:617`) ----

/// Premultiplied source-over: `color` at `coverage` over the premultiplied pixel (`text.rs:485`).
fn blend(pm: &mut Pixmap, x: i32, y: i32, c: Rgba, cov: f32) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    if x < 0 || y < 0 || x >= w || y >= h || cov <= 0.0 {
        return;
    }
    let idx = (y * w + x) as usize;
    let Some(px) = pm.pixels_mut().get_mut(idx) else { return };
    let a = (c.a() as f32 / 255.0) * cov.min(1.0);
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    let r = (c.r() as f32 * a + px.red() as f32 * inv).round() as u8;
    let g = (c.g() as f32 * a + px.green() as f32 * inv).round() as u8;
    let b = (c.b() as f32 * a + px.blue() as f32 * inv).round() as u8;
    let al = ((a * 255.0) + px.alpha() as f32 * inv).round().min(255.0) as u8;
    if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(r.min(al), g.min(al), b.min(al), al) {
        *px = p;
    }
}

/// A solid paint in `c`, anti-aliased as the renderer's exports draw text (`render/src/lib.rs:416`
/// with antialiasing on).
fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.r(), c.g(), c.b(), c.a());
    p.anti_alias = true;
    p
}

/// Draws one line of `text` in the label font, centred on (`cx`, `cy`) and moved by `offset`
/// (`text.rs:505`).
fn draw_text(pm: &mut Pixmap, text: &str, cx: f32, cy: f32, px_size: f32, color: Rgba, bold: bool, offset: (f32, f32)) {
    let Some(f) = font(bold) else { return };
    let sf = f.as_scaled(em(f, px_size));
    let (glyphs, width) = shape(text, px_size, f, None);
    let x0 = cx - width / 2.0 + offset.0;
    let base = cy + (sf.ascent() + sf.descent()) / 2.0 + offset.1;
    for (f, id, x, _) in glyphs {
        let g = id.with_scale_and_position(em(f, px_size), ab_glyph::point(x0 + x, base));
        if let Some(og) = f.outline_glyph(g) {
            let bb = og.px_bounds();
            og.draw(|gx, gy, cov| blend(pm, bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32, color, cov));
        }
    }
}

/// The slant of synthetic italics (the fonts have no italic face): tan 12° (`text.rs:526`).
const SLANT: f32 = 0.2126;

/// The most pixels of one slanted glyph's coverage gathered before it is drawn (`text.rs:583`).
const MAX_SLANTED_PIXELS: usize = 1 << 22;

/// Draws `og` slanted, its coverage split across the pixels it falls between (`text.rs:589`).
fn slanted_glyph(pm: &mut Pixmap, og: &ab_glyph::OutlinedGlyph, base: f32, color: Rgba) {
    let bb = og.px_bounds();
    let shift = |px_x: f32, px_y: f32| px_x + SLANT * (base - px_y - 0.5);
    let left = shift(bb.min.x, bb.max.y).min(shift(bb.min.x, bb.min.y)).floor();
    let right = shift(bb.max.x, bb.min.y).max(shift(bb.max.x, bb.max.y)).ceil() + 1.0;
    let (w, h) = ((right - left).max(0.0) as usize + 1, bb.height().max(0.0).ceil() as usize + 1);
    let Some(mut acc) = w.checked_mul(h).filter(|n| *n <= MAX_SLANTED_PIXELS).map(|n| vec![0.0f32; n]) else { return };
    og.draw(|gx, gy, cov| {
        let s = shift(bb.min.x + gx as f32, bb.min.y + gy as f32);
        let (ix, frac) = (s.floor(), s - s.floor());
        let col = (ix - left) as usize;
        let at = gy as usize * w + col;
        if let Some(a) = acc.get_mut(at) {
            *a += cov * (1.0 - frac);
        }
        if col + 1 < w
            && let Some(a) = acc.get_mut(at + 1)
        {
            *a += cov * frac;
        }
    });
    for (i, cov) in acc.into_iter().enumerate() {
        if cov > 0.0 {
            let (col, row) = ((i % w) as f32, (i / w) as f32);
            blend(pm, (left + col) as i32, (bb.min.y + row).floor() as i32, color, cov);
        }
    }
}

/// Draws `text` from `left` (pixels), its line centred on `cy`, at `px` in `face` with `spacing`
/// pixels between letters (`text.rs:554`).
fn styled_at(pm: &mut Pixmap, text: &str, left: f32, cy: f32, px: f32, color: Rgba, face: Face, spacing: f32) {
    if !(left.is_finite() && cy.is_finite() && px.is_finite() && spacing.is_finite()) || px <= 0.0 {
        return;
    }
    let Some((f, slanted)) = face_font(face) else { return };
    let sf = f.as_scaled(em(f, px));
    let base = cy + (sf.ascent() + sf.descent()) / 2.0;
    let (glyphs, _) = shape(text, px, f, sans_beside(f, face));
    for (i, (gf, id, x, _)) in glyphs.into_iter().enumerate() {
        let italic = leans(face, f, slanted, gf);
        let g = id.with_scale_and_position(em(gf, px), ab_glyph::point(left + x + spacing * i as f32, base));
        let Some(og) = gf.outline_glyph(g) else { continue };
        let bb = og.px_bounds();
        if italic {
            slanted_glyph(pm, &og, base, color);
            continue;
        }
        og.draw(|gx, gy, cov| {
            // (Floored, so a row just above or left of the pixmap is clipped, not drawn on row 0.)
            blend(pm, (bb.min.x + gx as f32).floor() as i32, (bb.min.y + gy as f32).floor() as i32, color, cov);
        });
    }
}

// ---- the label itself (`text.rs:619`–`:966`) ----

/// Draws a placed label (logical coordinates) with its halo (`text.rs:621`).
fn draw_label(pm: &mut Pixmap, l: &PlacedLabel, ppp: f32) {
    draw_callout(pm, l, ppp);
    // Curved along a line: each character turned into its place, every character's halo first,
    // so none covers the letter before it.
    if !l.glyphs.is_empty() {
        let inks: &[Ink] = if l.halo.is_some() { &[Ink::Halo, Ink::Text] } else { &[Ink::Text] };
        let base = PlacedLabel { text: String::new(), align: 0.0, angle: None, glyphs: Vec::new(), ..l.clone() };
        for &ink in inks {
            for g in &l.glyphs {
                let one = PlacedLabel { text: g.text.clone(), x: l.x + g.dx, y: l.y + g.dy, ..base.clone() };
                draw_turned(pm, &one, g.angle, ppp, ink);
            }
        }
        return;
    }
    match l.angle {
        Some(a) if a.is_finite() && a.abs() > 1e-3 => draw_turned(pm, l, a, ppp, Ink::Both),
        _ => draw_upright(pm, l, ppp, Ink::Both),
    }
}

/// A label's callout line, from its feature to the edge of its text (`text.rs:645`).
fn draw_callout(pm: &mut Pixmap, l: &PlacedLabel, ppp: f32) {
    let Some([x0, y0, x1, y1]) = l.collide.callout else { return };
    let mut pb = PathBuilder::new();
    pb.move_to((l.x + x0) * ppp, (l.y + y0) * ppp);
    pb.line_to((l.x + x1) * ppp, (l.y + y1) * ppp);
    let Some(path) = pb.finish() else { return };
    let width = (rustgis_render::CALLOUT_WIDTH * ppp).max(0.5);
    let inks = l.halo.map(|h| (h, width + 2.0 * ppp)).into_iter().chain([(l.color, width)]);
    for (c, w) in inks {
        let stroke = Stroke { width: w, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
        pm.stroke_path(&path, &paint(c), &stroke, Transform::identity(), None);
    }
}

/// What of a label to draw: its halo, its text, or both (`text.rs:666`).
#[derive(Clone, Copy, PartialEq)]
enum Ink {
    Halo,
    Text,
    Both,
}

/// Draws `l` turned by `angle` (radians, clockwise) about its centre (`text.rs:674`).
fn draw_turned(pm: &mut Pixmap, l: &PlacedLabel, angle: f32, ppp: f32, ink: Ink) {
    let (w, h) = (label_width(l), block_height(&l.text, l.size).max(line_pitch(l.size)));
    // Room for the halo, and for the glyphs reaching past the line.
    let pad = 4.0
        + l.size * 0.5
        + drawn_halo_reach(l).min(MAX_HALO_PX / ppp.max(f32::EPSILON))
        + background_reach(l).into_iter().fold(0.0, f32::max);
    let (pw, ph) = (((w + 2.0 * pad) * ppp).ceil(), ((h + 2.0 * pad) * ppp).ceil());
    if !(pw.is_finite() && ph.is_finite()) || pw > 4096.0 || ph > 4096.0 {
        return;
    }
    let (cx, cy) = (pw / ppp / 2.0, ph / ppp / 2.0);
    let upright = PlacedLabel { x: cx + l.align * w, y: cy, angle: None, framed: false, icon: None, ..l.clone() };
    let (tx, ty) = (l.center_x() * ppp, l.y * ppp);
    let t = Transform::from_translate(-pw / 2.0, -ph / 2.0).post_rotate(angle.to_degrees()).post_translate(tx, ty);
    let Some(mut pic) = Pixmap::new(pw.max(1.0) as u32, ph.max(1.0) as u32) else { return };
    draw_upright(&mut pic, &upright, ppp, ink);
    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
    pm.draw_pixmap(0, 0, pic.as_ref(), &paint, t, None);
}

/// Draws an upright label: its frame when selected, then its halo and text as `ink` says
/// (`text.rs:702`).
fn draw_upright(pm: &mut Pixmap, l: &PlacedLabel, ppp: f32, ink: Ink) {
    let px = l.size * 1.33 * ppp;
    let (cx, cy) = (l.center_x() * ppp, l.y * ppp);
    let widths: Vec<(&str, f32)> = lines(&l.text).map(|line| (line, line_width(l, line, ppp))).collect();
    let (n, line_h) = (widths.len(), line_pitch(l.size) * ppp);
    let w = widths.iter().map(|(_, lw)| *lw).fold(0.0, f32::max);
    if l.framed {
        let [x0, y0, x1, y1] = frame_box(l).map(|v| v * ppp);
        if let Some(r) = Rect::from_ltrb(x0, y0, x1, y1) {
            frame(pm, r, ppp);
        }
    }
    let top = cy - n.saturating_sub(1) as f32 * line_h / 2.0;
    // Each line: its text, its left edge (where the pen starts), its width, its centre y.
    let placed: Vec<(&str, f32, f32, f32)> = widths
        .iter()
        .enumerate()
        .map(|(i, (line, lw))| {
            let free = match l.line_align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (w - lw) / 2.0,
                TextAlign::Right => w - lw,
            };
            (*line, cx - w / 2.0 + free, *lw, top + i as f32 * line_h)
        })
        .collect();
    let (styled, spacing) = (styled(l), l.spacing * 1.33 * ppp);
    let put = |pm: &mut Pixmap, &(line, left, lw, y): &(&str, f32, f32, f32), color: Rgba, (dx, dy): (f32, f32)| {
        if styled {
            styled_at(pm, line, left + dx, y + dy, px, color, label_face(l), spacing);
        } else {
            draw_text(pm, line, left + lw / 2.0, y, px, color, l.bold, (dx, dy));
        }
    };
    // Its background under everything.
    if ink != Ink::Text {
        draw_background(pm, l, ppp);
    }
    // Every line's halo first, so none covers the line above it.
    if let Some(h) = l.halo.filter(|_| ink != Ink::Text) {
        match l.halo_width {
            // As wide as its class sets it: the letters spread that far all round, as QGIS's
            // buffer, in one coat.
            Some(width) => {
                let r = (rustgis_render::halo_reach(width) * 1.33 * ppp).min(MAX_HALO_PX);
                let (pad, x0, x1) = (
                    r + 0.4 * px + 2.0,
                    placed.iter().map(|p| p.1).fold(f32::MAX, f32::min),
                    placed.iter().map(|p| p.1 + p.2).fold(f32::MIN, f32::max),
                );
                let (y0, y1) = (top - line_h, top + n as f32 * line_h);
                let (pw, ph) = (pm.width() as f32, pm.height() as f32);
                let (x0, y0) = ((x0 - pad).max(-pad).floor(), (y0 - pad).max(-pad).floor());
                let (x1, y1) = ((x1 + pad).min(pw + pad).ceil(), (y1 + pad).min(ph + pad).ceil());
                // (None at all for a halo no wider than nothing.)
                if let Some(mut mask) = (r > 0.0 && x1 > x0 && y1 > y0).then(|| Pixmap::new((x1 - x0) as u32, (y1 - y0) as u32)).flatten() {
                    for line in &placed {
                        put(&mut mask, line, Rgba::rgb(0, 0, 0), (-x0, -y0));
                    }
                    let (w, cover) = (mask.width() as usize, spread(&mask, r));
                    for (i, c) in cover.into_iter().enumerate() {
                        blend(pm, x0 as i32 + (i % w) as i32, y0 as i32 + (i / w) as i32, h, c);
                    }
                }
            }
            None => {
                let r = 1.2 * ppp;
                let ring = [
                    (-r, 0.0),
                    (r, 0.0),
                    (0.0, -r),
                    (0.0, r),
                    (-r * 0.7, -r * 0.7),
                    (r * 0.7, r * 0.7),
                    (-r * 0.7, r * 0.7),
                    (r * 0.7, -r * 0.7),
                ];
                for line in &placed {
                    for offset in ring {
                        put(pm, line, h, offset);
                    }
                }
            }
        }
    }
    if ink == Ink::Halo {
        return;
    }
    for line in &placed {
        put(pm, line, l.color, (0.0, 0.0));
    }
}

/// Draws a label's background (logical coordinates, upright) at `ppp` pixels a point
/// (`text.rs:463`).
fn draw_background(pm: &mut Pixmap, l: &PlacedLabel, ppp: f32) {
    let (Some(r), Some(b)) = (background_box(l), l.background.as_ref()) else { return };
    let [x0, y0, x1, y1] = r.map(|v| v * ppp);
    let Some(rect) = Rect::from_ltrb(x0, y0, x1, y1) else { return };
    let path = rounded_rect(rect, b.radius * 1.33 * ppp).unwrap_or_else(|| PathBuilder::from_rect(rect));
    if let Some(f) = b.fill.filter(|f| f.a() > 0) {
        pm.fill_path(&path, &paint(f), FillRule::Winding, Transform::identity(), None);
    }
    if let Some(c) = b.stroke.filter(|c| c.a() > 0 && b.stroke_width > 0.0) {
        let stroke = Stroke { width: b.stroke_width * 1.33 * ppp, ..Stroke::default() };
        pm.stroke_path(&path, &paint(c), &stroke, Transform::identity(), None);
    }
}

/// How far past its letters a label's halo of its own width reaches (`text.rs:844`).
fn drawn_halo_reach(l: &PlacedLabel) -> f32 {
    l.halo.filter(|h| h.a() > 0).and(l.halo_width).map_or(0.0, |w| rustgis_render::halo_reach(w) * 1.33)
}

/// The widest halo drawn on a picture, in pixels (`text.rs:858`).
const MAX_HALO_PX: f32 = 256.0;

/// How much of each of `mask`'s pixels a halo `r` pixels wide covers (`text.rs:865`).
fn spread(mask: &Pixmap, r: f32) -> Vec<f32> {
    const FAR: f64 = 1e20;
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let cover: Vec<f32> = mask.pixels().iter().map(|p| f32::from(p.alpha()) / 255.0).collect();
    let mut d: Vec<f64> = cover.iter().map(|&c| if c >= 0.5 { 0.0 } else { FAR }).collect();
    let mut edt = Edt::default();
    // Down each column, then along each row.
    let mut col = vec![FAR; h];
    for x in 0..w {
        for (y, c) in col.iter_mut().enumerate() {
            *c = d.get(y * w + x).copied().unwrap_or(FAR);
        }
        for (y, v) in edt.run(&col).iter().enumerate() {
            if let Some(slot) = d.get_mut(y * w + x) {
                *slot = *v;
            }
        }
    }
    for y in 0..h {
        if let Some(row) = d.get_mut(y * w..(y + 1) * w) {
            let done = edt.run(row);
            row.copy_from_slice(done);
        }
    }
    cover.iter().zip(&d).map(|(&c, &d2)| c.max((r + 0.5 - d2.sqrt() as f32).clamp(0.0, 1.0))).collect()
}

/// The one-dimensional squared distance transform, its buffers kept between rows
/// (`text.rs:894`).
#[derive(Default)]
struct Edt {
    v: Vec<usize>,
    z: Vec<f64>,
    out: Vec<f64>,
}

impl Edt {
    /// For each `q`, the least `(q - p)² + f[p]` over `p`.
    fn run(&mut self, f: &[f64]) -> &[f64] {
        let n = f.len();
        self.v.clear();
        self.v.resize(n.max(1), 0);
        self.z.clear();
        self.z.resize(n + 1, f64::INFINITY);
        self.out.clear();
        self.out.resize(n, 0.0);
        if n == 0 {
            return &self.out;
        }
        let (v, z) = (&mut self.v, &mut self.z);
        let at = |p: usize| f[p] + (p * p) as f64;
        let mut k = 0;
        z[0] = f64::NEG_INFINITY;
        for q in 1..n {
            let mut s = (at(q) - at(v[k])) / (2.0 * (q - v[k]) as f64);
            // z[0] is -inf, so this stops at k = 0.
            while s <= z[k] {
                k -= 1;
                s = (at(q) - at(v[k])) / (2.0 * (q - v[k]) as f64);
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
        }
        k = 0;
        for q in 0..n {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            let dq = q as f64 - p as f64;
            self.out[q] = dq * dq + f[p];
        }
        &self.out
    }
}

/// The selection's frame: a light fill and an outline in the selection colour (`text.rs:960`).
fn frame(pm: &mut Pixmap, r: Rect, ppp: f32) {
    let path = PathBuilder::from_rect(r);
    pm.fill_path(&path, &paint(rustgis_render::SELECTION.with_alpha(40)), FillRule::Winding, Transform::identity(), None);
    let stroke = Stroke { width: 2.0 * ppp, ..Stroke::default() };
    pm.stroke_path(&path, &paint(rustgis_render::SELECTION), &stroke, Transform::identity(), None);
}

/// A rectangle with its corners rounded by `rad` pixels, each a quarter circle drawn as a cubic
/// (`layout.rs:512`).
fn rounded_rect(r: Rect, rad: f32) -> Option<Path> {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0);
    if rad.is_nan() || rad <= 0.0 {
        return None;
    }
    // The control points' distance that makes a cubic a quarter circle.
    let c = rad * 0.552_284_8;
    let (l, t, rt, b) = (r.left(), r.top(), r.right(), r.bottom());
    let mut pb = PathBuilder::new();
    pb.move_to(l + rad, t);
    pb.line_to(rt - rad, t);
    pb.cubic_to(rt - rad + c, t, rt, t + rad - c, rt, t + rad);
    pb.line_to(rt, b - rad);
    pb.cubic_to(rt, b - rad + c, rt - rad + c, b, rt - rad, b);
    pb.line_to(l + rad, b);
    pb.cubic_to(l + rad - c, b, l, b - rad + c, l, b - rad);
    pb.line_to(l, t + rad);
    pb.cubic_to(l, t + rad - c, l + rad - c, t, l + rad, t);
    pb.close();
    pb.finish()
}

// ---- the layer ----

/// Rasterises the labels for a `width`×`height` logical view at `dpi` device pixels per point.
///
/// `unplaced` (the labels `View Unplaced` shows) are drawn first and `labels` over them, the
/// order `rustgis_render::render_placed` draws them in. Returns premultiplied RGBA at the view's
/// physical size (`width * dpi` × `height * dpi`), or `None` when no label draws anything or the
/// size is degenerate.
pub fn rasterize(
    unplaced: &[PlacedLabel],
    labels: &[PlacedLabel],
    width: u32,
    height: u32,
    dpi: f32,
) -> Option<(u32, u32, Vec<u8>)> {
    let ppp = if dpi.is_finite() && dpi > 0.0 { dpi.clamp(0.1, 8.0) } else { 1.0 };
    let (pw, ph) = (
        (f64::from(width) * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32,
        (f64::from(height) * f64::from(ppp)).round().clamp(1.0, 16_384.0) as u32,
    );
    if labels.is_empty() && unplaced.is_empty() {
        return None;
    }
    let mut pixmap = Pixmap::new(pw, ph)?;
    let mut drawn = 0usize;
    for label in unplaced.iter().chain(labels) {
        // An icon is in the frame itself (see the module note), and nothing else would draw.
        if label.text.is_empty() && label.glyphs.is_empty() {
            continue;
        }
        draw_label(&mut pixmap, label, ppp);
        drawn += 1;
    }
    if drawn == 0 {
        return None;
    }
    Some((pw, ph, pixmap.data().to_vec()))
}

/// Where a label layer drawn for camera `placed` lands in a frame drawn for camera `now`, in
/// device pixels: `[x0, y0, x1, y1]` of the layer's `(width * ppp)` × `(height * ppp)` pixels.
///
/// A flat, north-up camera maps one screen point of `placed` to `anchor + k · (p - centre)`, with
/// `k` the zoom ratio and `anchor` the screen point of `placed`'s centre in `now` — a uniform
/// scale and a translation, exactly what one texture drawn into a rectangle can do. Same turn and
/// pitch are required: a camera that turned between the two frames would need every label turned
/// about the view's centre (its glyphs stay upright), which one quad cannot draw, so this returns
/// `None` and the caller rasterises the layer again.
///
/// `None` too when the ratio is absurd (the layer would be unreadable: the caller re-rasterises)
/// or either camera is tilted.
pub fn layer_rect(placed: &rustgis_map::Camera, now: &rustgis_map::Camera, width: f64, height: f64, ppp: f32) -> Option<[f32; 4]> {
    let pitch = rustgis_map::camera::normalize_pitch(now.pitch);
    let placed_pitch = rustgis_map::camera::normalize_pitch(placed.pitch);
    if pitch != 0.0 || placed_pitch != 0.0 || !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
        return None;
    }
    if !(placed.res.is_finite() && now.res.is_finite()) || placed.res <= 0.0 || now.res <= 0.0 {
        return None;
    }
    // The bare minimum a turned camera may differ by and still draw as one quad: none.
    if rustgis_map::camera::normalize_bearing(placed.bearing - now.bearing).abs() > 1e-9 {
        return None;
    }
    let k = placed.res / now.res;
    if !(0.2..=5.0).contains(&k) {
        return None;
    }
    let (ax, ay) = now.to_screen(placed.center, width, height);
    let (hw, hh) = (width * 0.5, height * 0.5);
    let corners = [(ax - k * hw, ay - k * hh), (ax + k * hw, ay + k * hh)];
    let rect = [corners[0].0 * f64::from(ppp), corners[0].1 * f64::from(ppp), corners[1].0 * f64::from(ppp), corners[1].1 * f64::from(ppp)];
    rect.iter().all(|v| v.is_finite()).then_some(rect.map(|v| v as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    use rustgis_geom::Coord;
    use rustgis_map::Camera;
    use rustgis_render::RenderOutput;

    /// A label in the label font at (`x`, `y`) points.
    fn sample(text: &str, x: f32, y: f32) -> PlacedLabel {
        PlacedLabel {
            text: text.to_string(),
            x,
            y,
            size: 11.0,
            color: Rgba::rgb(20, 20, 20),
            halo: Some(Rgba::rgb(255, 255, 255)),
            halo_width: None,
            bold: false,
            icon: None,
            align: 0.0,
            framed: false,
            italic: false,
            spacing: 0.0,
            font: None,
            line_align: TextAlign::Center,
            angle: None,
            collide: Default::default(),
            glyphs: Vec::new(),
            background: None,
        }
    }

    /// The layer's ink: its bounding box and how many pixels it covers.
    fn ink(rgba: &[u8], w: usize) -> (usize, usize, usize, usize, usize) {
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0);
        for (i, px) in rgba.chunks_exact(4).enumerate() {
            if px[3] == 0 {
                continue;
            }
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            n += 1;
        }
        (x0, y0, x1, y1, n)
    }

    fn frame() -> Camera {
        Camera { center: Coord::new(1_000_000.0, 6_000_000.0), res: 300.0, bearing: 0.0, pitch: 0.0 }
    }

    /// The layer a moving camera is drawn with covers exactly the ground its own frame showed:
    /// a label point lands where the new camera puts its map point, for a pan and for a zoom.
    #[test]
    fn a_moved_camera_moves_the_label_layer_with_it() {
        let (w, h) = (800.0, 500.0);
        let placed = frame();
        assert_eq!(layer_rect(&placed, &placed, w, h, 1.0), Some([0.0, 0.0, 800.0, 500.0]), "an unmoved camera's layer covers the frame");

        let check = |now: &Camera, rect: [f32; 4]| {
            for (px, py) in [(0.0, 0.0), (123.0, 77.0), (799.0, 499.0)] {
                let map = placed.to_map(px, py, w, h);
                let (sx, sy) = now.to_screen(map, w, h);
                // Where the layer's rectangle puts the same point.
                let (qx, qy) = (
                    f64::from(rect[0]) + px / w * f64::from(rect[2] - rect[0]),
                    f64::from(rect[1]) + py / h * f64::from(rect[3] - rect[1]),
                );
                assert!((qx - sx).abs() < 1e-3 && (qy - sy).abs() < 1e-3, "({px},{py}) -> ({qx},{qy}) vs ({sx},{sy})");
            }
        };

        // A 100 px drag right and 40 up: the layer moves with the map.
        let mut panned = placed;
        panned.pan_px(100.0, -40.0);
        let rect = layer_rect(&placed, &panned, w, h, 1.0).expect("a pan can move the layer");
        let [x0, y0, x1, y1] = rect.map(f64::from);
        assert!((x0 - 100.0).abs() < 1e-3 && (y0 + 40.0).abs() < 1e-3);
        assert!((x1 - (w + 100.0)).abs() < 1e-3 && (y1 - (h - 40.0)).abs() < 1e-3);
        check(&panned, rect);

        // A 2x zoom about the centre spreads the layer from it.
        let mut zoomed = placed;
        zoomed.res /= 2.0;
        let rect = layer_rect(&placed, &zoomed, w, h, 1.0).expect("a zoom can move the layer");
        let [x0, y0, x1, y1] = rect.map(f64::from);
        assert!((x0 + w / 2.0).abs() < 1e-3 && (y0 + h / 2.0).abs() < 1e-3, "{rect:?}");
        assert!((x1 - 1.5 * w).abs() < 1e-3 && (y1 - 1.5 * h).abs() < 1e-3, "{rect:?}");
        check(&zoomed, rect);

        // At 125 % the rectangle is in device pixels.
        let rect = layer_rect(&placed, &zoomed, w, h, 1.25).expect("a zoom can move the layer");
        let [x0, _, x1, _] = rect.map(f64::from);
        assert!((x0 + 1.25 * w / 2.0).abs() < 1e-3 && (x1 - 1.25 * 1.5 * w).abs() < 1e-3, "{rect:?}");
    }

    /// A layer whose frame cannot be moved by one quad is reported, not drawn in the wrong place:
    /// a turn, a tilt or a zoom past what one texture holds.
    #[test]
    fn a_turned_tilted_or_wildly_zoomed_camera_cannot_move_the_layer() {
        let placed = frame();
        let (w, h) = (800.0, 500.0);
        assert!(layer_rect(&placed, &Camera { bearing: 20.0, ..placed }, w, h, 1.0).is_none(), "a turn");
        assert!(layer_rect(&placed, &Camera { pitch: 30.0, ..placed }, w, h, 1.0).is_none(), "a tilt");
        assert!(layer_rect(&placed, &Camera { res: placed.res * 10.0, ..placed }, w, h, 1.0).is_none(), "a 10x zoom out");
        assert!(layer_rect(&placed, &Camera { res: placed.res / 10.0, ..placed }, w, h, 1.0).is_none(), "a 10x zoom in");
        // A turned layer may still be moved by a camera turned the same way.
        let turned = Camera { bearing: 20.0, ..placed };
        assert!(layer_rect(&turned, &turned, w, h, 1.0).is_some());
    }

    /// A label turned along a line is drawn turned: its ink is taller than it is wide.
    #[test]
    fn a_turned_label_is_drawn_turned() {
        let (w, h) = (200, 120);
        let flat = sample("Station Road", 100.0, 60.0);
        let (_, _, upright) = rasterize(&[], std::slice::from_ref(&flat), w, h, 1.0).expect("an upright label draws");
        let (x0, y0, x1, y1, _) = ink(&upright, w as usize);
        assert!(x1 - x0 > y1 - y0, "upright text is wider than tall: {}x{}", x1 - x0, y1 - y0);

        let turned = PlacedLabel { angle: Some(std::f32::consts::FRAC_PI_2), ..flat };
        let (_, _, turned) = rasterize(&[], &[turned], w, h, 1.0).expect("a turned label draws");
        let (x0, y0, x1, y1, _) = ink(&turned, w as usize);
        assert!(y1 - y0 > x1 - x0, "a quarter turn makes it taller than wide: {}x{}", x1 - x0, y1 - y0);
    }

    /// A halo of its own width spreads round the letters (the renderer's distance transform, not
    /// eight copies), and covers more of the frame than the thin ring does.
    #[test]
    fn a_wide_halo_spreads_round_the_letters() {
        let (w, h) = (160, 100);
        let thin = sample("Halo", 80.0, 50.0);
        let (_, _, thin_px) = rasterize(&[], std::slice::from_ref(&thin), w, h, 1.0).expect("a label with a thin halo draws");
        let wide = PlacedLabel { halo_width: Some(2.5), ..thin.clone() };
        let (_, _, wide_px) = rasterize(&[], &[wide], w, h, 1.0).expect("a label with a wide halo draws");
        let (.., thin_n) = ink(&thin_px, w as usize);
        let (.., wide_n) = ink(&wide_px, w as usize);
        assert!(wide_n > thin_n, "a 2.5 pt halo covers more than the thin ring: {thin_n} vs {wide_n}");

        // The halo is its own colour, all round the letters: just left of the widest ink, the
        // wide halo is there and white; a label with no halo has nothing there at all.
        let bare = PlacedLabel { halo: None, ..thin.clone() };
        let (_, _, bare_px) = rasterize(&[], std::slice::from_ref(&bare), w, h, 1.0).expect("a label without a halo draws");
        let (_, by0, _, by1, _) = ink(&bare_px, w as usize);
        let (x0, ..) = ink(&wide_px, w as usize);
        let i = (by0 + by1) / 2 * w as usize + x0 + 1;
        let (halo_px, bare_at) = (&wide_px[i * 4..][..4], bare_px[i * 4 + 3]);
        assert!(halo_px[3] > 0, "the halo reaches left of the letters");
        assert!(halo_px[0] > 100 && u16::from(halo_px[0]) + 4 >= u16::from(halo_px[3]), "the halo is its white colour, not the text's dark ink: {halo_px:?}");
        assert_eq!(bare_at, 0, "nothing reaches there without a halo");
    }

    /// A label's background is a filled, rounded box behind the text.
    #[test]
    fn a_background_box_is_filled_behind_the_text() {
        let (w, h) = (160, 100);
        let bare = PlacedLabel { halo: None, ..sample("Name", 80.0, 50.0) };
        let green = Rgba::rgb(0, 200, 0);
        let background = rustgis_map::LabelBackground {
            fill: Some(green),
            stroke: None,
            stroke_width: 0.0,
            padding: [6.0, 4.0],
            radius: 3.0,
        };
        let boxed = PlacedLabel { background: Some(background), ..bare.clone() };
        let (_, _, without) = rasterize(&[], std::slice::from_ref(&bare), w, h, 1.0).expect("a label draws");
        let (_, _, with) = rasterize(&[], &[boxed], w, h, 1.0).expect("a label with a background draws");
        let (.., without_n) = ink(&without, w as usize);
        let (x0, y0, _, y1, with_n) = ink(&with, w as usize);
        assert!(with_n > without_n * 2, "the box covers far more than the text: {without_n} vs {with_n}");
        // Inside its left edge, away from the glyphs (the padding is 8 px): the fill's own colour.
        let i = ((y0 + y1) / 2 * w as usize + x0 + 2) * 4;
        let px = &with[i..][..4];
        assert_eq!((px[0], px[1], px[2], px[3]), (0, 200, 0, 255), "the box's fill at its left edge");
        assert_eq!(&without[i..][..4], &[0, 0, 0, 0], "there is no box without a background");
    }

    /// A curved label is drawn character by character, each turned where the line takes it.
    #[test]
    fn a_curved_label_draws_its_characters() {
        use rustgis_render::LabelGlyph;
        let (w, h) = (200, 100);
        let curved = PlacedLabel {
            text: "Curve".to_string(),
            glyphs: vec![
                LabelGlyph { text: "C".to_string(), dx: -24.0, dy: 0.0, angle: 0.4 },
                LabelGlyph { text: "u".to_string(), dx: -8.0, dy: -2.0, angle: 0.2 },
                LabelGlyph { text: "r".to_string(), dx: 8.0, dy: 0.2, angle: 0.0 },
                LabelGlyph { text: "ve".to_string(), dx: 24.0, dy: 2.0, angle: -0.2 },
            ],
            ..sample("Curve", 100.0, 50.0)
        };
        let (_, _, px) = rasterize(&[], &[curved], w, h, 1.0).expect("a curved label draws");
        let (x0, y0, x1, y1, n) = ink(&px, w as usize);
        assert!(n > 0, "the characters drew nothing");
        assert!(x1 - x0 > 40 && y1 - y0 > 8, "the characters run along the curve: {}x{}", x1 - x0, y1 - y0);
    }

    /// Every label shape this layer draws, on one sheet, written to
    /// `target/rustgis-labels/label-features.png` for looking at (the world sample places none of
    /// them): a plain halo, a turned label, a curved one, a wide (spread) halo, a background box,
    /// a callout, and a selected (framed) text graphic in italics with letter spacing.
    #[test]
    fn the_label_feature_sheet_draws() {
        use rustgis_render::LabelGlyph;
        let (w, h) = (460u32, 300u32);
        let arc = |text: &str, size: f32| -> Vec<LabelGlyph> {
            // Characters laid along a circle of this radius, each turned to the tangent where it
            // stands (`text.rs`'s `line_labels` places a curved label the same way).
            let face = Face { bold: false, italic: false, family: None };
            let radius = 110.0f32;
            let widths: Vec<f32> = text.chars().map(|c| measure_in(&c.to_string(), size, face)).collect();
            let total: f32 = widths.iter().sum();
            let mut pen = -total / 2.0;
            let mut out = Vec::new();
            for (c, width) in text.chars().zip(&widths) {
                let angle = (pen + width / 2.0) / radius;
                out.push(LabelGlyph {
                    text: c.to_string(),
                    dx: radius * angle.sin(),
                    dy: radius * (1.0 - angle.cos()),
                    angle,
                });
                pen += width;
            }
            out
        };
        let bold = PlacedLabel { bold: true, size: 13.0, ..sample("Wide halo, bold", 330.0, 40.0) };
        let labels = vec![
            sample("Plain halo", 110.0, 30.0),
            PlacedLabel { angle: Some(0.5), size: 12.0, ..sample("Turned along a street", 110.0, 95.0) },
            PlacedLabel { text: "Curved along a line".to_string(), glyphs: arc("Curved along a line", 12.0), size: 12.0, ..sample("Curved along a line", 230.0, 150.0) },
            PlacedLabel { halo_width: Some(3.0), ..bold },
            PlacedLabel {
                background: Some(rustgis_map::LabelBackground { fill: Some(Rgba::rgb(255, 255, 255)), stroke: Some(Rgba::rgb(40, 40, 40)), stroke_width: 0.4, padding: [4.0, 2.0], radius: 3.0 }),
                halo: None,
                size: 12.0,
                ..sample("Background box", 340.0, 200.0)
            },
            PlacedLabel {
                collide: rustgis_tiles::Collide { callout: Some([-52.0, -26.0, -30.0, -10.0]), ..Default::default() },
                size: 12.0,
                ..sample("Callout", 120.0, 250.0)
            },
            PlacedLabel {
                framed: true,
                italic: true,
                spacing: 1.0,
                size: 12.0,
                halo: None,
                ..sample("Selected text", 340.0, 250.0)
            },
        ];
        let (pw, ph, rgba) = rasterize(&[], &labels, w, h, 1.0).expect("the feature sheet draws");
        assert_eq!((pw, ph), (w, h));
        // Every label left its own ink where it was placed.
        for (bx, by) in [(110.0, 30.0), (110.0, 95.0), (230.0, 150.0), (330.0, 40.0), (340.0, 200.0), (120.0, 250.0), (340.0, 250.0)] {
            let (x, y) = (bx as usize, by as usize);
            let (rows, cols) = (y.saturating_sub(24)..(y + 24).min(h as usize), x.saturating_sub(60)..(x + 60).min(w as usize));
            let ink = rows.into_iter().any(|row| {
                cols.clone().any(|col| rgba[(row * w as usize + col) * 4 + 3] > 0)
            });
            assert!(ink, "no ink near ({bx}, {by})");
        }
        // Kept for looking at, over white.
        let mut png = Vec::with_capacity(rgba.len());
        for px in rgba.chunks_exact(4) {
            for c in 0..3 {
                png.push((f32::from(px[c]) + 255.0 * (1.0 - f32::from(px[3]) / 255.0)).round().clamp(0.0, 255.0) as u8);
            }
            png.push(255);
        }
        let dir = std::path::Path::new("target/rustgis-labels");
        let _ = std::fs::create_dir_all(dir);
        let _ = image::save_buffer(dir.join("label-features.png"), &png, w, h, image::ColorType::Rgba8);
    }

    // ---- the layer against the CPU/export path (`render_placed`) ----

    /// The world sample's layers, as the GPU path draws them.
    fn world_map() -> rustgis_map::Map {
        let mut m = rustgis_map::Map::new("Map");
        for (name, text) in [
            ("Countries", rustgis_render::sample_data::COUNTRIES),
            ("Rivers", rustgis_render::sample_data::RIVERS),
            ("Lakes", rustgis_render::sample_data::LAKES),
            ("Cities", rustgis_render::sample_data::CITIES),
        ] {
            if let Some(d) = rustgis_render::world_dataset(name, text) {
                m.add_dataset(d);
            }
        }
        if let Some(l) = m.layers.iter_mut().find(|l| l.name == "Cities") {
            l.labels.enabled = true;
        }
        m
    }

    fn world_view(ppp: f64) -> rustgis_render::View {
        rustgis_render::View { width: 640.0, height: 400.0, pixels_per_point: ppp, camera: Camera::world(), sky: false }
    }

    /// Mean per-byte absolute difference, and the fraction of pixels whose worst channel differs
    /// by more than 16.
    fn diff_stats(a: &[u8], b: &[u8]) -> (f64, f64) {
        assert_eq!(a.len(), b.len());
        let mut sum = 0u64;
        let mut over = 0usize;
        for (px, py) in a.chunks(4).zip(b.chunks(4)) {
            let mut worst = 0u8;
            for c in 0..4 {
                let d = px[c].abs_diff(py[c]);
                sum += u64::from(d);
                worst = worst.max(d);
            }
            if worst > 16 {
                over += 1;
            }
        }
        (sum as f64 / a.len() as f64, over as f64 / (a.len() / 4) as f64)
    }

    /// Pixels differing by >16 whose 3×3 neighbourhood in the reference is FLAT: a mismatch away
    /// from an edge is a structural bug, not antialiasing.
    fn structural_outliers(reference: &[u8], ours: &[u8], w: usize, h: usize) -> usize {
        let mut out = 0;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let worst = (0..4).map(|c| reference[i + c].abs_diff(ours[i + c])).max().unwrap_or(0);
                if worst <= 16 {
                    continue;
                }
                let mut lo = [255u8; 4];
                let mut hi = [0u8; 4];
                for dy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                    for dx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                        let j = (dy * w + dx) * 4;
                        for c in 0..4 {
                            lo[c] = lo[c].min(reference[j + c]);
                            hi[c] = hi[c].max(reference[j + c]);
                        }
                    }
                }
                if (0..4).all(|c| hi[c] - lo[c] < 4) {
                    out += 1;
                }
            }
        }
        out
    }

    /// The label layer over the map without labels, which is what the GPU draws it on.
    fn compose(base: &[u8], layer: &[u8]) -> Vec<u8> {
        assert_eq!(base.len(), layer.len());
        base.chunks_exact(4)
            .zip(layer.chunks_exact(4))
            .flat_map(|(b, s)| {
                let sa = f32::from(s[3]) / 255.0;
                (0..3)
                    .map(move |i| (f32::from(s[i]) + f32::from(b[i]) * (1.0 - sa)).round().clamp(0.0, 255.0) as u8)
                    .chain([(sa * 255.0 + f32::from(b[3]) * (1.0 - sa)).round().clamp(0.0, 255.0) as u8])
            })
            .collect()
    }

    /// The layer drawn from the GPU scene's labels, over the CPU frame, is the frame
    /// `rustgis_render::render_placed` draws for the same map and view: the settled GPU frame's
    /// labels are the CPU/export path's labels.
    ///
    /// (The comparison holds the base frame fixed: the layer is composited over the CPU frame
    /// drawn without labels, so what differs is the labels alone.)
    fn assert_labels_match_render_placed(name: &str, ppp: f64, bearing: f64) {
        let view = rustgis_render::View { camera: Camera { bearing, ..Camera::world() }, ..world_view(ppp) };
        let Some(RenderOutput::Gpu(scene)) = rustgis_render::render_backend(&mut world_map(), &view, rustgis_render::Backend::Gpu) else {
            panic!("{name}: the world sample is a GPU scene");
        };
        assert!(!scene.labels.is_empty(), "{name}: the sample's labels were placed");
        let layer = rasterize(&scene.unplaced, &scene.labels, view.width as u32, view.height as u32, view.pixels_per_point as f32)
            .expect("the labels rasterise");
        let base = rustgis_render::render(&mut world_map(), &view).expect("the CPU base frame").pixmap;
        let placed = rustgis_render::render_placed(&mut world_map(), &view).expect("the CPU export").pixmap;
        assert_eq!((layer.0, layer.1), (base.width(), base.height()), "{name}: the layer is the frame's size");
        let ours = compose(base.data(), &layer.2);
        let (mean, over) = diff_stats(placed.data(), &ours);
        let structural = structural_outliers(placed.data(), &ours, base.width() as usize, base.height() as usize);
        // (Shown with `--nocapture`: the numbers the comparison holds.)
        println!("label layer vs render_placed ({name}): mean {mean:.3}/255, {over:.4} pixels > 16, {structural} structural outliers");
        assert!(mean < 1.5 && structural == 0, "{name}: mean {mean:.3}/255, {over:.4} pixels over 16, {structural} structural outliers");
    }

    #[test]
    fn the_label_layer_matches_the_cpu_render() {
        assert_labels_match_render_placed("world-sample", 1.0, 0.0);
    }

    #[test]
    fn the_label_layer_matches_the_cpu_render_at_125_percent() {
        assert_labels_match_render_placed("world-sample-125", 1.25, 0.0);
    }

    #[test]
    fn the_label_layer_matches_the_cpu_render_turned() {
        assert_labels_match_render_placed("turned", 1.0, 35.0);
    }
}

