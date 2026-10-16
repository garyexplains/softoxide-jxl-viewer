//! Canvas coordinate helpers + crop drawing/interaction.

use eframe::egui;
use egui::{Color32, Painter, Rect, Response, Vec2};

use crate::app::{App, Tool};

#[inline]
pub fn screen_to_px(p: egui::Pos2, img_rect: Rect, img_size: Vec2) -> Vec2 {
    let t = (p - img_rect.min) / img_rect.size();
    Vec2::new(t.x * img_size.x, t.y * img_size.y)
}

#[inline]
pub fn px_to_screen(px: Vec2, img_rect: Rect, img_size: Vec2) -> egui::Pos2 {
    let rel = Vec2::new(px.x / img_size.x, px.y / img_size.y);
    img_rect.min + rel * img_rect.size()
}

pub fn crop_rect_screen(r_px: crate::raster::RasterRect, img_rect: Rect, img_size: Vec2) -> Rect {
    let a = px_to_screen(Vec2::new(r_px.x as f32, r_px.y as f32), img_rect, img_size);
    let b = px_to_screen(
        Vec2::new((r_px.x + r_px.width) as f32, (r_px.y + r_px.height) as f32),
        img_rect,
        img_size,
    );
    Rect::from_two_pos(a, b)
}

fn rect_from_px_points(a: Vec2, b: Vec2) -> Rect {
    Rect::from_two_pos(egui::pos2(a.x, a.y), egui::pos2(b.x, b.y))
}

pub fn draw_crop(app: &mut App, painter: &Painter, img_rect: Rect, img_size: Vec2) {
    // Current selection rect in image pixels.
    let px_rect: Option<Rect> = if let Some(drag) = &app.crop_drag {
        Some(rect_from_px_points(drag.start_px, drag.current_px))
    } else {
        app.crop_rect.map(|r| {
            Rect::from_min_max(
                egui::pos2(r.x as f32, r.y as f32),
                egui::pos2((r.x + r.width) as f32, (r.y + r.height) as f32),
            )
        })
    };
    let Some(r) = px_rect else { return };
    // Convert to screen and draw an isolating overlay.
    let a = px_to_screen(egui::vec2(r.min.x, r.min.y), img_rect, img_size);
    let b = px_to_screen(egui::vec2(r.max.x, r.max.y), img_rect, img_size);
    let rect = Rect::from_two_pos(a, b);
    let overlay = Color32::from_rgba_unmultiplied(0, 0, 0, 110);

    let img_l = img_rect.min.x;
    let img_r = img_rect.max.x;
    let img_b = img_rect.max.y;

    painter.rect_filled(
        Rect::from_min_max(img_rect.min, egui::pos2(img_r, rect.min.y)),
        0.0,
        overlay,
    );
    painter.rect_filled(
        Rect::from_min_max(
            egui::pos2(img_l, rect.min.y),
            egui::pos2(rect.min.x, rect.max.y),
        ),
        0.0,
        overlay,
    );
    painter.rect_filled(
        Rect::from_min_max(
            egui::pos2(rect.max.x, rect.min.y),
            egui::pos2(img_r, rect.max.y),
        ),
        0.0,
        overlay,
    );
    painter.rect_filled(
        Rect::from_min_max(egui::pos2(img_l, rect.max.y), egui::pos2(img_r, img_b)),
        0.0,
        overlay,
    );

    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(2.0, Color32::from_rgb(255, 200, 0)),
        egui::StrokeKind::Outside,
    );
    for c in [
        rect.min,
        rect.max,
        egui::pos2(rect.min.x, rect.max.y),
        egui::pos2(rect.max.x, rect.min.y),
    ] {
        painter.circle_filled(c, 4.0, Color32::from_rgb(255, 200, 0));
    }

    // live dims label above the rect
    let dims = format!(
        "{}×{}",
        (r.max.x - r.min.x).abs() as i32,
        (r.max.y - r.min.y).abs() as i32
    );
    painter.text(
        egui::pos2(rect.center().x, (rect.min.y - 14.0).max(img_rect.min.y)),
        egui::Align2::CENTER_CENTER,
        dims,
        egui::FontId::proportional(11.0),
        Color32::from_rgb(255, 200, 0),
    );
}

pub fn crop_input(
    app: &mut App,
    response: &Response,
    img_rect: Rect,
    img_size: Vec2,
    ctx: &egui::Context,
) {
    let _ = app.tool == Tool::Crop; // marker; the caller gates by tool

    if response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            if img_rect.contains(pos) {
                let px = screen_to_px(pos, img_rect, img_size);
                app.crop_drag = Some(crate::app::CropDrag {
                    start_px: px,
                    current_px: px,
                });
            }
        }
    }

    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            if let Some(d) = &mut app.crop_drag {
                d.current_px = screen_to_px(pos, img_rect, img_size);
            }
        }
    }

    if response.drag_stopped_by(egui::PointerButton::Primary) {
        if let Some(d) = app.crop_drag.take() {
            let r = Rect::from_two_pos(
                egui::pos2(d.start_px.x, d.start_px.y),
                egui::pos2(d.current_px.x, d.current_px.y),
            );
            if r.width().max(r.height()) > 2.0 {
                let x0 = r.min.x.floor().max(0.0) as u32;
                let y0 = r.min.y.floor().max(0.0) as u32;
                let x1 = r.max.x.ceil().min(img_size.x) as u32;
                let y1 = r.max.y.ceil().min(img_size.y) as u32;
                app.crop_rect = Some(crate::raster::RasterRect {
                    x: x0,
                    y: y0,
                    width: (x1 - x0).max(1),
                    height: (y1 - y0).max(1),
                });
            }
            ctx.request_repaint();
        }
    }
}
