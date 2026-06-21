use crate::display::{self, Framebuffer};
use crate::optimizer;
use crate::system;

const TILE_W: usize = 116;
const TILE_H: usize = 72;
const TILE_GAP: usize = 10;
const PAD: usize = 16;
const BAR_H: usize = 26;
const CORNER_RADIUS: usize = 16;
const HEADER_H: usize = 24;
const APP_COLS: usize = 3;
const SYSTEM_ROW_H: usize = 34;
const SYSTEM_GAP: usize = 6;
const SEPARATOR_GAP: usize = 12;

#[derive(Copy, Clone)]
pub enum StartAction {
    OpenConsole,
    OpenExplorer,
    OpenClipboard,
    OpenNotepad,
    OpenBrowser,
    OpenMusicPlayer,
    OpenPhotoViewer,
    OpenVideoPlayer,
    OpenBin,
    OpenCalculator,
    OpenSecurity,
    ChangeWallpaper,
    ToggleTheme,
    Reboot,
    Shutdown,
}

pub struct StartMenu {
    visible: bool,
    win_x: usize,
    win_y: usize,
    win_w: usize,
    win_h: usize,
}

impl StartMenu {
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
        let (x, y, w, h) = calc_rect(fb);
        self.win_x = x;
        self.win_y = y;
        self.win_w = w;
        self.win_h = h;
        mark_dirty(x, y, w, h);
    }

    pub fn hide(&mut self, fb: &Framebuffer) {
        let (x, y, w, h) = self.rect(fb);
        mark_dirty(x, y, w, h);
        self.visible = false;
    }

    pub fn handle_click(&mut self, fb: &Framebuffer, x: usize, y: usize) -> Option<StartAction> {
        if !self.visible {
            return None;
        }
        let (wx, wy, ww, _) = self.rect(fb);
        let grid_x = wx + PAD;
        let grid_y = wy + PAD + HEADER_H;
        let items = app_items();
        for (i, item) in items.iter().enumerate() {
            let col = i % APP_COLS;
            let row = i / APP_COLS;
            let rx = grid_x + col * (TILE_W + TILE_GAP);
            let ry = grid_y + row * (TILE_H + TILE_GAP);
            if x >= rx && x < rx + TILE_W && y >= ry && y < ry + TILE_H && x < wx + ww {
                return Some(item.action);
            }
        }

        let rows = grid_rows(items.len());
        let mut sy = system_start_y(wy, rows);
        for item in system_items().iter() {
            if x >= grid_x
                && x < grid_x + (ww - PAD * 2)
                && y >= sy
                && y < sy + SYSTEM_ROW_H
            {
                return Some(item.action);
            }
            sy += SYSTEM_ROW_H + SYSTEM_GAP;
        }
        None
    }

    pub fn refresh(&self, fb: &Framebuffer) {
        if self.visible {
            self.redraw(fb);
        }
    }

    fn redraw(&self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }

        let (x, y, w, h) = self.rect(fb);
        let ui = system::ui_settings();
        let accent = ui.accent;
        let is_dark = ui.dark;

        draw_rounded_rect(fb, x + 2, y + 2, w, h, CORNER_RADIUS, 0x00101010);
        draw_rounded_rect(
            fb,
            x,
            y,
            w,
            h,
            CORNER_RADIUS,
            if is_dark { 0x001E1E1E } else { 0x00FFFFFF },
        );
        draw_rounded_rect_outline(
            fb,
            x,
            y,
            w,
            h,
            CORNER_RADIUS,
            1,
            if is_dark { 0x003A3A3A } else { 0x00E0E0E0 },
        );

        let mut writer = crate::TextWriter::new(*fb);
        let text_color = if is_dark { 0x00F2F5F8 } else { 0x00172233 };
        let secondary_text = if is_dark { 0x00AAB4C2 } else { 0x005C6A7E };

        writer.set_color(text_color);
        writer.set_pos(x + PAD + 2, y + PAD);
        writer.write_bytes(b"HouseOS Start");

        writer.set_color(secondary_text);
        writer.set_pos(x + w.saturating_sub(PAD + 96), y + PAD);
        writer.write_bytes(b"3 x apps");

        let grid_x = x + PAD;
        let grid_y = y + PAD + HEADER_H;
        let items = app_items();
        for (i, item) in items.iter().enumerate() {
            let col = i % APP_COLS;
            let row = i / APP_COLS;
            let rx = grid_x + col * (TILE_W + TILE_GAP);
            let ry = grid_y + row * (TILE_H + TILE_GAP);
            draw_tile(
                fb,
                &mut writer,
                rx,
                ry,
                item.label,
                item.icon,
                i,
                is_dark,
                accent,
                text_color,
                secondary_text,
            );
        }

        let rows = grid_rows(items.len());
        let sep_y = separator_y(y, rows);
        display::fill_rect(
            fb,
            grid_x,
            sep_y,
            w.saturating_sub(PAD * 2),
            1,
            if is_dark { 0x00384250 } else { 0x00D8E1EC },
        );

        let mut sy = system_start_y(y, rows);
        for (i, item) in system_items().iter().enumerate() {
            draw_system_row(
                fb,
                &mut writer,
                grid_x,
                sy,
                w.saturating_sub(PAD * 2),
                item.label,
                item.icon,
                i,
                is_dark,
                accent,
                text_color,
                secondary_text,
            );
            sy += SYSTEM_ROW_H + SYSTEM_GAP;
        }
    }

    pub fn rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        if self.win_w == 0 {
            calc_rect(fb)
        } else {
            (self.win_x, self.win_y, self.win_w, self.win_h)
        }
    }
}

fn mark_dirty(x: usize, y: usize, w: usize, h: usize) {
    if let Some(opt) = optimizer::get_optimizer() {
        opt.add_dirty_rect(x, y, w, h);
    }
}

#[derive(Copy, Clone)]
struct StartItem {
    label: &'static [u8],
    icon: &'static [u8],
    action: StartAction,
}

fn app_items() -> [StartItem; 12] {
    [
        StartItem { label: b"Browser", icon: b"W", action: StartAction::OpenBrowser },
        StartItem { label: b"Explorer", icon: b"#", action: StartAction::OpenExplorer },
        StartItem { label: b"Office", icon: b"O", action: StartAction::OpenNotepad },
        StartItem { label: b"Music", icon: b"M", action: StartAction::OpenMusicPlayer },
        StartItem { label: b"Photo", icon: b"P", action: StartAction::OpenPhotoViewer },
        StartItem { label: b"Video", icon: b"V", action: StartAction::OpenVideoPlayer },
        StartItem { label: b"Console", icon: b">", action: StartAction::OpenConsole },
        StartItem { label: b"Calculator", icon: b"C", action: StartAction::OpenCalculator },
        StartItem { label: b"Clipboard", icon: b"@", action: StartAction::OpenClipboard },
        StartItem { label: b"Wallpaper", icon: b"B", action: StartAction::ChangeWallpaper },
        StartItem { label: b"Recycle", icon: b"%", action: StartAction::OpenBin },
        StartItem { label: b"Security", icon: b"!", action: StartAction::OpenSecurity },
    ]
}

fn system_items() -> [StartItem; 3] {
    [
        StartItem { label: b"Theme", icon: b"T", action: StartAction::ToggleTheme },
        StartItem { label: b"Restart", icon: b"R", action: StartAction::Reboot },
        StartItem { label: b"Shutdown", icon: b"S", action: StartAction::Shutdown },
    ]
}

fn grid_rows(count: usize) -> usize {
    (count + APP_COLS - 1) / APP_COLS
}

fn separator_y(win_y: usize, app_rows: usize) -> usize {
    win_y + PAD + HEADER_H + app_rows * TILE_H + app_rows.saturating_sub(1) * TILE_GAP + SEPARATOR_GAP
}

fn system_start_y(win_y: usize, app_rows: usize) -> usize {
    separator_y(win_y, app_rows) + SEPARATOR_GAP
}

fn draw_tile(
    fb: &Framebuffer,
    writer: &mut crate::TextWriter,
    x: usize,
    y: usize,
    label: &[u8],
    icon: &[u8],
    index: usize,
    is_dark: bool,
    accent: u32,
    text: u32,
    detail: u32,
) {
    let tint = match index % 6 {
        0 => 0x003D6FA3,
        1 => 0x002D8A65,
        2 => 0x00704D9B,
        3 => 0x00905B3A,
        4 => 0x002E7A9A,
        _ => accent,
    };
    let bg = if is_dark {
        blend_rgb(tint, 0x0013161B, 42)
    } else {
        blend_rgb(tint, 0x00FFFFFF, 78)
    };
    let top = if is_dark { blend_rgb(bg, 0x00FFFFFF, 8) } else { 0x00FFFFFF };
    draw_rounded_rect(fb, x, y, TILE_W, TILE_H, 10, bg);
    display::fill_rect(fb, x + 8, y + 8, TILE_W.saturating_sub(16), 1, top);
    display::fill_rect(fb, x + 10, y + TILE_H.saturating_sub(10), TILE_W.saturating_sub(20), 1, if is_dark { 0x002A313A } else { 0x00C9D8E8 });

    writer.set_color(if is_dark { 0x00FFFFFF } else { tint });
    writer.set_pos(x + 14, y + 12);
    writer.write_bytes(icon);

    writer.set_color(text);
    writer.set_pos(x + 14, y + 38);
    let max = (TILE_W - 24) / 8;
    let len = label.len().min(max);
    writer.write_bytes(&label[..len]);

    writer.set_color(detail);
    writer.set_pos(x + 14, y + 54);
    writer.write_bytes(b"Open");
}

fn draw_system_row(
    fb: &Framebuffer,
    writer: &mut crate::TextWriter,
    x: usize,
    y: usize,
    w: usize,
    label: &[u8],
    icon: &[u8],
    index: usize,
    is_dark: bool,
    accent: u32,
    text: u32,
    detail: u32,
) {
    let base = if is_dark { 0x0021262D } else { 0x00F4F7FB };
    let stripe = match index {
        0 => accent,
        1 => 0x00D18A24,
        _ => 0x00C74848,
    };
    draw_rounded_rect(fb, x, y, w, SYSTEM_ROW_H, 8, base);
    display::fill_rect(fb, x + 8, y + 7, 3, SYSTEM_ROW_H.saturating_sub(14), stripe);

    writer.set_color(stripe);
    writer.set_pos(x + 22, y + 9);
    writer.write_bytes(icon);

    writer.set_color(text);
    writer.set_pos(x + 42, y + 9);
    writer.write_bytes(label);

    writer.set_color(detail);
    writer.set_pos(x + w.saturating_sub(58), y + 9);
    writer.write_bytes(b"Run");
}

fn draw_rounded_rect(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    radius: usize,
    color: u32,
) {
    for dy in 0..h {
        for dx in 0..w {
            if is_inside_rounded(dx, dy, w, h, radius) {
                display::put_pixel(fb, x + dx, y + dy, color);
            }
        }
    }
}

fn draw_rounded_rect_outline(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    radius: usize,
    thickness: usize,
    color: u32,
) {
    for dy in 0..h {
        for dx in 0..w {
            let inside = is_inside_rounded(dx, dy, w, h, radius);
            let inside_inner = is_inside_rounded(
                dx + thickness,
                dy + thickness,
                w.saturating_sub(2 * thickness),
                h.saturating_sub(2 * thickness),
                radius.saturating_sub(thickness),
            );
            if inside && !inside_inner {
                display::put_pixel(fb, x + dx, y + dy, color);
            }
        }
    }
}

fn is_inside_rounded(dx: usize, dy: usize, w: usize, h: usize, r: usize) -> bool {
    if dx < r && dy < r {
        return (dx as isize - r as isize).pow(2) + (dy as isize - r as isize).pow(2)
            <= (r * r) as isize;
    }
    if dx >= w.saturating_sub(r) && dy < r {
        return (dx as isize - (w - r) as isize).pow(2) + (dy as isize - r as isize).pow(2)
            <= (r * r) as isize;
    }
    if dx < r && dy >= h.saturating_sub(r) {
        return (dx as isize - r as isize).pow(2) + (dy as isize - (h - r) as isize).pow(2)
            <= (r * r) as isize;
    }
    if dx >= w.saturating_sub(r) && dy >= h.saturating_sub(r) {
        return (dx as isize - (w - r) as isize).pow(2) + (dy as isize - (h - r) as isize).pow(2)
            <= (r * r) as isize;
    }
    true
}

fn calc_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let w = PAD * 2 + TILE_W * 3 + TILE_GAP * 2;
    let app_rows = grid_rows(app_items().len());
    let system_rows = system_items().len();
    let h = PAD * 2
        + HEADER_H
        + app_rows * TILE_H
        + app_rows.saturating_sub(1) * TILE_GAP
        + SEPARATOR_GAP * 2
        + system_rows * SYSTEM_ROW_H
        + system_rows.saturating_sub(1) * SYSTEM_GAP;
    let x = 20;
    let y = fb.height.saturating_sub(h + BAR_H + 20);
    (x, y, w, h)
}

fn blend_rgb(a: u32, b: u32, pct_b: u32) -> u32 {
    let pct_b = pct_b.min(100);
    let pct_a = 100 - pct_b;
    let ar = (a >> 16) & 0xFF;
    let ag = (a >> 8) & 0xFF;
    let ab = a & 0xFF;
    let br = (b >> 16) & 0xFF;
    let bg = (b >> 8) & 0xFF;
    let bb = b & 0xFF;
    let r = (ar * pct_a + br * pct_b) / 100;
    let g = (ag * pct_a + bg * pct_b) / 100;
    let bl = (ab * pct_a + bb * pct_b) / 100;
    (r << 16) | (g << 8) | bl
}
