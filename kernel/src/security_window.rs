use crate::display::{self, Framebuffer};
use crate::protection;
use crate::system;
use crate::window;

const PAD: usize = 16;
const BTN_W: usize = 78;
const BTN_H: usize = 24;

pub struct SecurityWindow {
    visible: bool,
    win_x: usize,
    win_y: usize,
    win_w: usize,
    win_h: usize,
}

impl SecurityWindow {
    pub fn new(_fb: Framebuffer) -> Self {
        Self {
            visible: false,
            win_x: 0,
            win_y: 0,
            win_w: 0,
            win_h: 0,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn show(&mut self, fb: &Framebuffer) {
        self.visible = true;
        if self.win_w == 0 || self.win_h == 0 {
            let (x, y, w, h) = calc_rect(fb);
            self.win_x = x;
            self.win_y = y;
            self.win_w = w;
            self.win_h = h;
        }
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        if self.win_w == 0 || self.win_h == 0 {
            calc_rect(fb)
        } else {
            (self.win_x, self.win_y, self.win_w, self.win_h)
        }
    }

    pub fn handle_click(&mut self, fb: &Framebuffer, x: usize, y: usize) -> bool {
        if !self.visible {
            return false;
        }
        let (wx, wy, ww, wh) = self.rect(fb);
        if !window::hit(x, y, (wx, wy, ww, wh)) {
            return false;
        }
        if window::hit(x, y, window::close_rect(wx, wy, ww)) {
            self.hide();
            return true;
        }
        let scan = self.scan_button_rect(fb);
        if window::hit(x, y, scan) {
            protection::run_scan();
            return true;
        }
        let close = self.close_button_rect(fb);
        if window::hit(x, y, close) {
            self.hide();
            return true;
        }
        true
    }

    pub fn redraw(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }

        let report = match protection::last_scan() {
            Some(v) => v,
            None => protection::run_scan(),
        };

        let (x, y, w, h) = self.rect(fb);
        let chrome = window::draw_window(fb, x, y, w, h, b"Security");
        let ui = system::ui_settings();
        let is_dark = ui.dark;
        let accent = ui.accent;

        let bg_top = if is_dark { 0x001A2028 } else { 0x00FFFFFF };
        let bg_bottom = if is_dark { 0x00131820 } else { 0x00F4F8FD };
        fill_vertical_gradient(
            fb,
            chrome.content_x,
            chrome.content_y,
            chrome.content_w,
            chrome.content_h,
            bg_top,
            bg_bottom,
        );

        let mut writer = crate::TextWriter::new(*fb);
        let text = if is_dark { 0x00F3F6FA } else { 0x00172230 };
        let detail = if is_dark { 0x00AEB8C6 } else { 0x00546576 };
        let danger = if is_dark { 0x00FF8A8A } else { 0x00B42332 };
        let ok = if is_dark { 0x007FE7A2 } else { 0x001E7F4D };
        let panel = if is_dark { 0x00222934 } else { 0x00FFFFFF };
        let panel_edge = if is_dark { 0x00455060 } else { 0x00D6E2F0 };

        let panel_x = chrome.content_x + PAD;
        let panel_y = chrome.content_y + PAD;
        let panel_w = chrome.content_w.saturating_sub(PAD * 2);
        let panel_h = 78;
        display::fill_rect(fb, panel_x, panel_y, panel_w, panel_h, panel_edge);
        display::fill_rect(fb, panel_x + 1, panel_y + 1, panel_w.saturating_sub(2), panel_h.saturating_sub(2), panel);
        display::fill_rect(fb, panel_x + 1, panel_y + 1, 6, panel_h.saturating_sub(2), if report.warnings > 0 { danger } else { ok });

        writer.set_color(if report.warnings > 0 { danger } else { ok });
        writer.set_pos(panel_x + 18, panel_y + 14);
        if report.warnings > 0 {
            writer.write_bytes(b"Threat detected");
        } else {
            writer.write_bytes(b"No threats detected");
        }

        writer.set_color(detail);
        writer.set_pos(panel_x + 18, panel_y + 36);
        if report.warnings > 0 {
            writer.write_bytes(b"HouseOS found warnings during the last scan.");
        } else {
            writer.write_bytes(b"Protection is watching memory, devices and drivers.");
        }

        writer.set_color(text);
        writer.set_pos(panel_x, panel_y + panel_h + 16);
        writer.write_bytes(b"Scan report");

        let list_y = panel_y + panel_h + 36;
        for i in 0..report.count {
            let row_y = list_y + i * 18;
            if row_y + 14 >= chrome.content_y + chrome.content_h.saturating_sub(36) {
                break;
            }
            let line = &report.lines[i][..report.lens[i]];
            writer.set_color(report_line_color(line, is_dark, text, detail, danger, ok));
            writer.set_pos(panel_x + 4, row_y);
            writer.write_bytes(line);
        }

        let scan = self.scan_button_rect(fb);
        let close = self.close_button_rect(fb);
        draw_button(fb, &mut writer, scan, b"Scan", accent, 0x00FFFFFF, is_dark);
        draw_button(fb, &mut writer, close, b"Close", if is_dark { 0x00424B58 } else { 0x00DCE5F0 }, text, is_dark);
    }

    fn scan_button_rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        let (x, y, w, h) = self.rect(fb);
        (
            x + w.saturating_sub(PAD + BTN_W * 2 + 8),
            y + h.saturating_sub(PAD + BTN_H),
            BTN_W,
            BTN_H,
        )
    }

    fn close_button_rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        let (x, y, w, h) = self.rect(fb);
        (
            x + w.saturating_sub(PAD + BTN_W),
            y + h.saturating_sub(PAD + BTN_H),
            BTN_W,
            BTN_H,
        )
    }
}

fn draw_button(
    fb: &Framebuffer,
    writer: &mut crate::TextWriter,
    rect: (usize, usize, usize, usize),
    label: &[u8],
    bg: u32,
    fg: u32,
    is_dark: bool,
) {
    display::fill_rect(fb, rect.0, rect.1, rect.2, rect.3, if is_dark { 0x0010151C } else { 0x00C8D4E2 });
    display::fill_rect(fb, rect.0 + 1, rect.1 + 1, rect.2.saturating_sub(2), rect.3.saturating_sub(2), bg);
    writer.set_color(fg);
    writer.set_pos(rect.0 + rect.2.saturating_sub(label.len() * 8) / 2, rect.1 + 7);
    writer.write_bytes(label);
}

fn report_line_color(line: &[u8], is_dark: bool, text: u32, detail: u32, danger: u32, ok: u32) -> u32 {
    if starts_with(line, b"Result: OK") {
        return ok;
    }
    if starts_with(line, b"Result: WARN") || starts_with(line, b"NET:") || starts_with(line, b"USB controllers: 0") {
        return danger;
    }
    if starts_with(line, b"HouseOS") || starts_with(line, b"Scan") {
        return if is_dark { 0x008AD8FF } else { 0x002B6CB0 };
    }
    if line.is_empty() {
        return detail;
    }
    text
}

fn starts_with(buf: &[u8], prefix: &[u8]) -> bool {
    if buf.len() < prefix.len() {
        return false;
    }
    for i in 0..prefix.len() {
        if buf[i] != prefix[i] {
            return false;
        }
    }
    true
}

fn fill_vertical_gradient(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    top: u32,
    bottom: u32,
) {
    for row in 0..h {
        let color = lerp_rgb(top, bottom, row as u32, h.saturating_sub(1).max(1) as u32);
        display::fill_rect(fb, x, y + row, w, 1, color);
    }
}

fn lerp_rgb(a: u32, b: u32, num: u32, den: u32) -> u32 {
    let ar = (a >> 16) & 0xFF;
    let ag = (a >> 8) & 0xFF;
    let ab = a & 0xFF;
    let br = (b >> 16) & 0xFF;
    let bg = (b >> 8) & 0xFF;
    let bb = b & 0xFF;
    let r = (ar * (den - num) + br * num) / den;
    let g = (ag * (den - num) + bg * num) / den;
    let blue = (ab * (den - num) + bb * num) / den;
    (r << 16) | (g << 8) | blue
}

fn calc_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let w = (fb.width * 52 / 100).max(440).min(fb.width.saturating_sub(40));
    let h = (fb.height * 46 / 100).max(300).min(fb.height.saturating_sub(80));
    ((fb.width - w) / 2, (fb.height - h) / 2, w, h)
}
