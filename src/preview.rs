//! GUI print preview.
//!
//! Renders the same `rows` bitmap that will be sent to the printer into an
//! on-screen window (via `egui`/`eframe`), and asks the user to confirm
//! before printing. Requires the `preview` feature (enabled by default).
//!
//! The preview window has a "View" menu to zoom the UI in/out; the chosen
//! zoom level is persisted to a small config file (see [`config`]) and
//! restored the next time the preview is shown. The window size itself is
//! not persisted (it always opens at a fixed default size, centered on the
//! monitor).

use crate::tepra::TapeWidth;

/// Outcome of showing the preview window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewResult {
    /// User clicked "Print" (or pressed Enter).
    Print,
    /// User clicked "Cancel", pressed Escape, or closed the window.
    Cancel,
}

/// Persisted preview preferences (currently just the UI zoom level).
mod config {
    use std::io::Write as _;

    const DEFAULT_ZOOM: f32 = 1.0;
    const MIN_ZOOM: f32 = 0.5;
    const MAX_ZOOM: f32 = 3.0;

    /// Default window size, used every time the preview is opened (window
    /// size is intentionally not persisted; see the module docs).
    pub const DEFAULT_WIDTH: f32 = 800.0;
    pub const DEFAULT_HEIGHT: f32 = 400.0;

    /// Path to `tpsr3700/preview.conf` under the user's config directory
    /// (e.g. `~/.config/tpsr3700/preview.conf` on Linux, `~/Library/
    /// Application Support/tpsr3700/preview.conf` on macOS), or `None` if
    /// the OS config directory can't be determined.
    fn config_path() -> Option<std::path::PathBuf> {
        let mut dir = dirs::config_dir()?;
        dir.push("tpsr3700");
        dir.push("preview.conf");
        Some(dir)
    }

    /// Loads the saved zoom level, falling back to [`DEFAULT_ZOOM`] if the
    /// config file is missing, unreadable, or contains an invalid value.
    pub fn load_zoom() -> f32 {
        let Some(path) = config_path() else {
            return DEFAULT_ZOOM;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            return DEFAULT_ZOOM;
        };
        for line in content.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("zoom") {
                let value = value.trim().trim_start_matches('=').trim();
                if let Ok(zoom) = value.parse::<f32>() {
                    return zoom.clamp(MIN_ZOOM, MAX_ZOOM);
                }
            }
        }
        DEFAULT_ZOOM
    }

    /// Saves `zoom` so it is restored the next time the preview is shown.
    /// Silently does nothing if the config directory can't be determined
    /// or written to (persistence is a convenience, not essential).
    pub fn save_zoom(zoom: f32) {
        let Some(path) = config_path() else { return };
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        if let Ok(mut file) = std::fs::File::create(&path) {
            let _ = writeln!(file, "# tpsr3700 print preview settings");
            let _ = writeln!(file, "zoom = {zoom:.2}");
        }
    }

    pub const fn min() -> f32 {
        MIN_ZOOM
    }

    pub const fn max() -> f32 {
        MAX_ZOOM
    }
}

/// Shows a preview window rendering `rows` (the same bitmap rows that
/// [`crate::tepra::builder::build_print_job`] would encode) for `tape`, and
/// blocks until the user confirms or cancels.
///
/// `rows` is a sequence of feed-direction raster rows, each a bit-packed
/// row `tape.bitmap_bits()` dots wide (MSB-first), exactly as produced by
/// [`crate::font::FontRenderer::render_text`] /
/// [`crate::font::FontRenderer::render_text_multiline`].
///
/// The UI zoom level is loaded from (and, if changed via the "View" menu,
/// saved back to) a small per-user config file; see [`config`].
///
/// `top_margin_bits` / `bottom_margin_bits` are the non-printing margins at
/// the top/bottom edges of the tape (same meaning as the `--top-margin` /
/// `--bottom-margin` CLI options). `left_margin_rows` / `right_margin_rows`
/// are the blank feed-direction rows already included at the start/end of
/// `rows` (same meaning as the `margin_rows` returned by
/// [`crate::font::FontRenderer::render_text`] and the `--right-margin-mm`
/// CLI option converted to rows via [`crate::font::mm_to_rows`]). The area
/// outside the printable band on any edge is shaded gray in the preview to
/// distinguish it from the printable area.
pub fn show_preview(
    rows: &[Vec<u8>],
    tape: TapeWidth,
    label_text: &str,
    top_margin_bits: usize,
    bottom_margin_bits: usize,
    left_margin_rows: u32,
    right_margin_rows: u32,
) -> anyhow::Result<PreviewResult> {
    let total_bits = tape.bitmap_bits();
    let feed_len = rows.len().max(1);
    let head_offset = bottom_margin_bits;
    let head_bits = total_bits.saturating_sub(top_margin_bits + bottom_margin_bits);
    let head_end = head_offset + head_bits;
    let left_margin_rows = (left_margin_rows as usize).min(feed_len);
    let right_margin_rows = (right_margin_rows as usize).min(feed_len);
    let content_end = feed_len.saturating_sub(right_margin_rows);

    // Build an RGBA image: feed direction along X (so the label reads
    // left-to-right like the physical tape), tape width along Y.
    // Bit 0 is the bottom edge of the tape (see `TapeWidth::head_offset`'s
    // doc comment: `head_offset = bottom_margin_bits`), so flip vertically
    // when mapping bit index to image row (image row 0 = top of the tape).
    const MARGIN_GRAY: u8 = 224;
    let img_w = feed_len;
    let img_h = total_bits;
    let mut pixels = vec![255u8; img_w * img_h * 4]; // white background, opaque

    for y in 0..img_h {
        let bit = total_bits - 1 - y;
        for x in 0..img_w {
            // Gray out anything outside the printable band on any edge:
            // top/bottom (non-printing margin bits) or left/right (feed
            // margin rows).
            let in_top_bottom_band = bit >= head_offset && bit < head_end;
            let in_left_right_band = x >= left_margin_rows && x < content_end;
            if !in_top_bottom_band || !in_left_right_band {
                let idx = (y * img_w + x) * 4;
                pixels[idx] = MARGIN_GRAY;
                pixels[idx + 1] = MARGIN_GRAY;
                pixels[idx + 2] = MARGIN_GRAY;
            }
        }
    }

    for (x, row) in rows.iter().enumerate() {
        for bit in 0..total_bits {
            let byte_i = bit / 8;
            let bit_j = bit % 8;
            let inked = byte_i < row.len() && (row[byte_i] >> (7 - bit_j)) & 1 == 1;
            if inked {
                let y = total_bits - 1 - bit;
                let idx = (y * img_w + x) * 4;
                pixels[idx] = 0;
                pixels[idx + 1] = 0;
                pixels[idx + 2] = 0;
                // alpha stays 255
            }
        }
    }

    let color_image = egui::ColorImage::from_rgba_unmultiplied([img_w, img_h], &pixels);

    let result = std::rc::Rc::new(std::cell::Cell::new(PreviewResult::Cancel));
    let result_clone = result.clone();
    let title = label_text.to_string();
    let initial_zoom = config::load_zoom();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([config::DEFAULT_WIDTH, config::DEFAULT_HEIGHT])
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "Print preview",
        native_options,
        Box::new(move |cc| {
            cc.egui_ctx.set_zoom_factor(initial_zoom);

            // The default egui fonts don't cover Japanese (or most non-Latin
            // scripts), so labels/tooltips containing the label text would
            // otherwise render as tofu boxes. Add the same bundled font
            // used for printing (BIZ UDPGothic) as a fallback so any text
            // that can be printed can also be displayed in the preview UI.
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "bundled_bizudp_gothic".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(
                    crate::font::DEFAULT_FONT_BYTES,
                )),
            );
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("bundled_bizudp_gothic".to_owned());
            }
            cc.egui_ctx.set_fonts(fonts);

            let texture = cc.egui_ctx.load_texture(
                "preview-bitmap",
                color_image,
                egui::TextureOptions::NEAREST,
            );
            Ok(Box::new(PreviewApp {
                texture,
                img_w,
                img_h,
                tape,
                title: title.clone(),
                result: result_clone.clone(),
                done: false,
                centered: false,
                zoom: initial_zoom,
                saved_zoom: initial_zoom,
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("failed to open preview window: {e}"))?;

    Ok(result.get())
}

struct PreviewApp {
    texture: egui::TextureHandle,
    img_w: usize,
    img_h: usize,
    tape: TapeWidth,
    title: String,
    result: std::rc::Rc<std::cell::Cell<PreviewResult>>,
    done: bool,
    /// Whether the window has already been centered on the monitor once
    /// (see the centering logic in `ui`).
    centered: bool,
    /// Current UI zoom factor (1.0 = egui default size).
    zoom: f32,
    /// Zoom value last written to the config file, to avoid redundant
    /// disk writes while the user drags/clicks repeatedly.
    saved_zoom: f32,
}

impl PreviewApp {
    fn set_zoom(&mut self, ctx: &egui::Context, zoom: f32) {
        self.zoom = zoom.clamp(config::min(), config::max());
        ctx.set_zoom_factor(self.zoom);
    }
}

impl eframe::App for PreviewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Center the window on the monitor once, on the first frame where
        // the outer size/monitor info is actually available. The window
        // always opens at a fixed size, so one shot is enough.
        if !self.centered {
            if let Some(cmd) = egui::ViewportCommand::center_on_screen(&ctx) {
                ctx.send_viewport_cmd(cmd);
                self.centered = true;
            } else {
                ctx.request_repaint();
            }
        }

        // Enter = Print, Escape = Cancel.
        let mut close_with = None;
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Enter) {
                close_with = Some(PreviewResult::Print);
            } else if i.key_pressed(egui::Key::Escape) {
                close_with = Some(PreviewResult::Cancel);
            }
        });

        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("View", |ui| {
                    if ui.button("Zoom In").clicked() {
                        self.set_zoom(&ctx, self.zoom + 0.1);
                        ui.close();
                    }
                    if ui.button("Zoom Out").clicked() {
                        self.set_zoom(&ctx, self.zoom - 0.1);
                        ui.close();
                    }
                    if ui.button("Reset Zoom (100%)").clicked() {
                        self.set_zoom(&ctx, 1.0);
                        ui.close();
                    }
                    ui.separator();
                    ui.label(format!("Current zoom: {:.0}%", self.zoom * 100.0));
                });
            });
        });

        egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(4.0);
            // Keep the header a single line even when the printed text
            // contains literal newlines, so the header's height stays
            // predictable. The full, unmodified text is available as a
            // tooltip.
            let single_line_title = self.title.replace('\n', " / ");
            ui.add(
                egui::Label::new(format!(
                    "\"{}\" — {}mm tape, {} dots wide × {} dots long",
                    single_line_title,
                    self.tape.width_mm(),
                    self.img_h,
                    self.img_w
                ))
                .truncate(),
            )
            .on_hover_text(&self.title);
            ui.add_space(4.0);
        });

        egui::Panel::bottom("buttons").show(ui, |ui| {
            ui.add_space(6.0);
            let height = ui.spacing().interact_size.y;
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), height),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .button(egui::RichText::new("Print (Enter)").strong())
                        .clicked()
                    {
                        close_with = Some(PreviewResult::Print);
                    }
                    if ui.button("Cancel (Esc)").clicked() {
                        close_with = Some(PreviewResult::Cancel);
                    }
                },
            );
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                let available = ui.available_width();
                let scale = (available / self.img_w as f32).clamp(0.5, 4.0);
                let size = egui::vec2(
                    self.img_w as f32 * scale,
                    self.img_h as f32 * scale,
                );
                ui.image((self.texture.id(), size));
            });
        });

        // Persist the zoom level if it changed since the last save (e.g.
        // via the View menu), so it's restored next time.
        if (self.zoom - self.saved_zoom).abs() > f32::EPSILON {
            config::save_zoom(self.zoom);
            self.saved_zoom = self.zoom;
        }

        if let Some(r) = close_with {
            self.result.set(r);
            self.done = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // If the user closes the window via the OS chrome, `done` stays
        // false but the window closes anyway; result already defaults to
        // Cancel from `show_preview`.
        let _ = self.done;
    }
}
