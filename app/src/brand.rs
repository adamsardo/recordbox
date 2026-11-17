//! The recordbox logo: one SVG asset (mark + wordmark in a single colour, `assets/brand/`),
//! drawn wherever the app shows its name as a logo, and the mark as the app icon.

use eframe::egui;
use egui::{Color32, Rect, Ui};

/// Mark and wordmark, `fill="currentColor"`: the colour is set when drawing.
pub const LOGO_SVG: &str = include_str!("../assets/brand/recordbox-logo.svg");
/// The mark on its own (white square, dark line and fader), for the app icon.
pub const MARK_SVG: &str = include_str!("../assets/brand/recordbox-mark.svg");
/// Width / height of the logo's view box (616 x 83 units).
pub const LOGO_ASPECT: f32 = 616.0 / 83.0;

fn hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}

/// The logo in colour `c` as an image of the given height (pt), its width following.
pub fn logo_image(c: Color32, height: f32) -> egui::Image<'static> {
    let col = hex(c);
    let svg = LOGO_SVG.replace("currentColor", &col);
    egui::Image::from_bytes(format!("bytes://recordbox-logo-{col}.svg"), svg.into_bytes()).fit_to_exact_size(egui::vec2(height * LOGO_ASPECT, height))
}

/// Draws the logo with its view box's top-left corner at `rect.min`, `rect.height()` tall.
pub fn paint_logo(ui: &Ui, rect: Rect, c: Color32) {
    let r = Rect::from_min_size(rect.min, egui::vec2(rect.height() * LOGO_ASPECT, rect.height()));
    logo_image(c, rect.height()).paint_at(ui, r);
}

/// The mark rendered to RGBA pixels (`size` x `size`), for the window and Dock icon.
pub fn mark_icon(size: u32) -> Option<egui::IconData> {
    let opt = egui::SizeHint::Size(size, size);
    let img = egui_extras::image::load_svg_bytes_with_size(MARK_SVG.as_bytes(), Some(opt)).ok()?;
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    Some(egui::IconData { rgba, width: img.size[0] as u32, height: img.size[1] as u32 })
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    #[test]
    fn logo_and_mark_render() {
        let img = egui_extras::image::load_svg_bytes_with_size(super::LOGO_SVG.replace("currentColor", "#FFFFFF").as_bytes(), Some(egui::SizeHint::Size(616, 83))).unwrap();
        assert_eq!(img.size, [616, 83]);
        // The mark's square is drawn, and its fader is cut out of it (transparent).
        let at = |x: usize, y: usize| img.pixels[y * 616 + x];
        assert!(at(30 - 19, 300 - 278).a() > 200, "square drawn");
        // Fader centre: mark unit 60 -> logo unit 58.5 + 10 x 75/72, less the view box's left edge.
        let fader = (58.5 + 10.0 * 75.0 / 72.0 - 19.0) as usize;
        assert!(at(fader, (321.3 - 278.0) as usize).a() < 40, "fader cut out at x {fader}");
        // Letters drawn ("d" stem near unit 434).
        assert!((0..83).any(|y| at(434 - 19, y).a() > 200));
        let m = super::mark_icon(256).unwrap();
        assert_eq!((m.width, m.height), (256, 256));
    }
}
