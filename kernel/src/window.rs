use crate::display::{self, Framebuffer};
use crate::system;

pub const HEADER_H: usize = 42;
pub const CORNER_RADIUS: usize = 14;

const BTN_SIZE: usize = 24;
const BTN_GAP: usize = 7;
const BTN_RIGHT_PAD: usize = 12;

#[derive(Copy, Clone)]
pub struct ChromeLayout {
    pub content_x: usize,
    pub content_y: usize,
    pub content_w: usize,
    pub content_h: usize,
    pub close: (usize, usize, usize, usize),
    pub maximize: (usize, usize, usize, usize),
    pub minimize: (usize, usize, usize, usize),
    pub header: (usize, usize, usize, usize),
    pub drag_header: (usize, usize, usize, usize),
}

pub fn draw_window(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    title: &[u8],
) -> ChromeLayout {
    let ui = system::ui_settings();
    let accent = ui.accent;
    let is_dark = ui.dark;

    let surface = if is_dark { 0x001A2230 } else { 0x00F4F8FE };
    let content = if is_dark { 0x00121820 } else { 0x00FFFFFF };
    let header_top = if is_dark { 0x003B485C } else { 0x00FFFFFF };
    let header_bottom = if is_dark { 0x00242E3D } else { 0x00E7F0FB };
    let border = if is_dark { 0x00566174 } else { 0x00AFC2D8 };
    let inner_line = if is_dark { 0x00465366 } else { 0x00FFFFFF };
    let text_primary = if is_dark { 0x00F6F9FC } else { 0x0014202E };
    let text_muted = if is_dark { 0x00B5C1D1 } else { 0x00526276 };
    let shadow = if is_dark { 0x0005070C } else { 0x00899CAF };

    fill_round_rect(fb, x + 8, y + 10, w, h, CORNER_RADIUS, shadow);
    fill_round_rect(fb, x + 4, y + 5, w, h, CORNER_RADIUS, if is_dark { 0x00101720 } else { 0x00D4DEE9 });

    fill_round_rect(fb, x, y, w, h, CORNER_RADIUS, surface);
    fill_round_rect(
        fb,
        x + 1,
        y + HEADER_H,
        w.saturating_sub(2),
        h.saturating_sub(HEADER_H + 1),
        CORNER_RADIUS.saturating_sub(2),
        content,
    );

    fill_header(fb, x, y, w, HEADER_H, header_top, header_bottom);
    display::fill_rect(fb, x + 1, y + HEADER_H - 1, w.saturating_sub(2), 1, border);
    display::fill_rect(fb, x + CORNER_RADIUS, y, w.saturating_sub(CORNER_RADIUS * 2), 1, inner_line);
    display::fill_rect(fb, x + CORNER_RADIUS, y + h.saturating_sub(1), w.saturating_sub(CORNER_RADIUS * 2), 1, border);
    display::fill_rect(fb, x, y + CORNER_RADIUS, 1, h.saturating_sub(CORNER_RADIUS * 2), border);
    display::fill_rect(fb, x + w.saturating_sub(1), y + CORNER_RADIUS, 1, h.saturating_sub(CORNER_RADIUS * 2), border);
    display::fill_rect(fb, x + CORNER_RADIUS, y + 2, w.saturating_sub(CORNER_RADIUS * 2), 2, accent);
    display::fill_rect(fb, x + CORNER_RADIUS + 2, y + 5, w.saturating_sub(CORNER_RADIUS * 2 + 4), 1, inner_line);
    display::fill_rect(fb, x + 16, y + HEADER_H - 6, 52, 2, accent);
    display::fill_rect(fb, x + 18, y + HEADER_H - 3, w.saturating_sub(36), 1, if is_dark { 0x00344150 } else { 0x00DCE8F5 });

    let close = close_rect(x, y, w);
    let maximize = maximize_rect(x, y, w);
    let minimize = minimize_rect(x, y, w);

    draw_control_button(fb, minimize, if is_dark { 0x00524630 } else { 0x00F4D77A }, 0);
    draw_control_button(fb, maximize, if is_dark { 0x00335B42 } else { 0x0077CC91 }, 1);
    draw_control_button(fb, close, if is_dark { 0x00613A43 } else { 0x00F07F8D }, 2);

    let mut writer = crate::TextWriter::new(*fb);
    display::fill_rect(fb, x + 14, y + 12, 3, 18, accent);
    writer.set_color(text_primary);
    writer.set_pos(x + 24, y + 13);
    writer.write_bytes(title);

    let subtitle_x = x + 24 + title.len() * 8 + 10;
    let max_subtitle_x = minimize.0.saturating_sub(92);
    if subtitle_x < max_subtitle_x {
        writer.set_color(text_muted);
        writer.set_pos(subtitle_x, y + 13);
        writer.write_bytes(b"HouseOS");
    }

    ChromeLayout {
        content_x: x + 1,
        content_y: y + HEADER_H,
        content_w: w.saturating_sub(2),
        content_h: h.saturating_sub(HEADER_H + 1),
        close,
        maximize,
        minimize,
        header: (x, y, w, HEADER_H),
        drag_header: drag_header_rect(x, y, w),
    }
}

fn fill_round_rect(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    r: usize,
    color: u32,
) {
    if w == 0 || h == 0 {
        return;
    }
    let r = r.min(w / 2).min(h / 2);
    if r == 0 {
        display::fill_rect(fb, x, y, w, h, color);
        return;
    }
    display::fill_rect(fb, x + r, y, w.saturating_sub(r * 2), h, color);
    display::fill_rect(fb, x, y + r, r, h.saturating_sub(r * 2), color);
    display::fill_rect(fb, x + w.saturating_sub(r), y + r, r, h.saturating_sub(r * 2), color);
    draw_corner(fb, x, y, r, 0, color);
    draw_corner(fb, x + w.saturating_sub(r), y, r, 1, color);
    draw_corner(fb, x, y + h.saturating_sub(r), r, 2, color);
    draw_corner(fb, x + w.saturating_sub(r), y + h.saturating_sub(r), r, 3, color);
}

fn fill_header(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, top: u32, bottom: u32) {
    for row in 0..h {
        let color = lerp_rgb(top, bottom, row as u32, h.saturating_sub(1).max(1) as u32);
        display::fill_rect(fb, x + 1, y + row, w.saturating_sub(2), 1, color);
    }
    draw_corner(fb, x, y, CORNER_RADIUS, 0, top);
    draw_corner(fb, x + w.saturating_sub(CORNER_RADIUS), y, CORNER_RADIUS, 1, top);
}

fn draw_control_button(fb: &Framebuffer, rect: (usize, usize, usize, usize), bg: u32, kind: u8) {
    fill_round_rect(fb, rect.0, rect.1, rect.2, rect.3, 7, bg);
    display::fill_rect(fb, rect.0 + 2, rect.1 + rect.3.saturating_sub(2), rect.2.saturating_sub(4), 1, 0x00485763);
    display::fill_rect(fb, rect.0 + 6, rect.1 + 4, rect.2.saturating_sub(12), 1, 0x00FFFFFF);
    let dark = 0x0023313A;
    let cx = rect.0 + rect.2 / 2;
    let cy = rect.1 + rect.3 / 2;
    match kind {
        0 => display::fill_rect(fb, rect.0 + 7, cy + 3, rect.2.saturating_sub(14), 2, dark),
        1 => {
            display::fill_rect(fb, cx.saturating_sub(5), cy.saturating_sub(5), 10, 2, dark);
            display::fill_rect(fb, cx.saturating_sub(5), cy + 4, 10, 2, dark);
            display::fill_rect(fb, cx.saturating_sub(5), cy.saturating_sub(5), 2, 11, dark);
            display::fill_rect(fb, cx + 4, cy.saturating_sub(5), 2, 11, dark);
        }
        _ => {
            for i in 0..9 {
                display::put_pixel(fb, cx.saturating_sub(4) + i, cy.saturating_sub(4) + i, dark);
                display::put_pixel(fb, cx.saturating_sub(4) + i, cy.saturating_sub(3) + i, dark);
                display::put_pixel(fb, cx + 4 - i, cy.saturating_sub(4) + i, dark);
                display::put_pixel(fb, cx + 4 - i, cy.saturating_sub(3) + i, dark);
            }
        }
    }
}

fn draw_corner(fb: &Framebuffer, x: usize, y: usize, r: usize, corner: u8, color: u32) {
    let r_i = r as isize;
    let r_sq = r_i * r_i;
    for dy in 0..r {
        for dx in 0..r {
            let (px, py) = (dx as isize, dy as isize);
            let inside = match corner {
                0 => (px - r_i).pow(2) + (py - r_i).pow(2) <= r_sq,
                1 => px.pow(2) + (py - r_i).pow(2) <= r_sq,
                2 => (px - r_i).pow(2) + py.pow(2) <= r_sq,
                3 => px.pow(2) + py.pow(2) <= r_sq,
                _ => true,
            };
            if inside {
                display::put_pixel(fb, x + dx, y + dy, color);
            }
        }
    }
}

fn lerp_rgb(a: u32, b: u32, num: u32, den: u32) -> u32 {
    let ar = ((a >> 16) & 0xFF) as u32;
    let ag = ((a >> 8) & 0xFF) as u32;
    let ab = (a & 0xFF) as u32;
    let br = ((b >> 16) & 0xFF) as u32;
    let bg = ((b >> 8) & 0xFF) as u32;
    let bb = (b & 0xFF) as u32;
    let r = (ar * (den - num) + br * num) / den;
    let g = (ag * (den - num) + bg * num) / den;
    let b = (ab * (den - num) + bb * num) / den;
    (r << 16) | (g << 8) | b
}

pub fn close_rect(x: usize, y: usize, w: usize) -> (usize, usize, usize, usize) {
    let cx = x + w.saturating_sub(BTN_RIGHT_PAD + BTN_SIZE);
    let cy = y + (HEADER_H.saturating_sub(BTN_SIZE)) / 2;
    (cx, cy, BTN_SIZE, BTN_SIZE)
}

pub fn maximize_rect(x: usize, y: usize, w: usize) -> (usize, usize, usize, usize) {
    let close = close_rect(x, y, w);
    let cx = close.0.saturating_sub(BTN_SIZE + BTN_GAP);
    (cx, close.1, BTN_SIZE, BTN_SIZE)
}

pub fn minimize_rect(x: usize, y: usize, w: usize) -> (usize, usize, usize, usize) {
    let max = maximize_rect(x, y, w);
    let cx = max.0.saturating_sub(BTN_SIZE + BTN_GAP);
    (cx, max.1, BTN_SIZE, BTN_SIZE)
}

pub fn drag_header_rect(x: usize, y: usize, w: usize) -> (usize, usize, usize, usize) {
    let min_btn = minimize_rect(x, y, w);
    let start_x = x + 12;
    let right = min_btn.0.saturating_sub(6);
    let drag_w = right.saturating_sub(start_x);
    (start_x, y, drag_w, HEADER_H)
}

pub fn hit(px: usize, py: usize, rect: (usize, usize, usize, usize)) -> bool {
    px >= rect.0 && py >= rect.1 && px < rect.0 + rect.2 && py < rect.1 + rect.3
}
