use crate::display::{self, Framebuffer};
use crate::drivers::battery;
use crate::drivers::audio::pc_speaker;
use crate::rtc::RtcTime;
use crate::system;

pub const BAR_H: usize = 32;
const MAX_W: usize = 1920;
const MAX_BACK: usize = MAX_W * BAR_H;
const VOLUME_W: usize = 96;
const VOLUME_H: usize = 20;
const POPUP_W: usize = 174;
const POPUP_H: usize = 52;

static mut STATUS_BACK: [u32; MAX_BACK] = [0; MAX_BACK];
static mut STATUS_W: usize = 0;
static mut STATUS_SAVED: bool = false;

static mut LAST_TIME_HASH: u32 = 0;
static mut CACHED_TIME_STR: [u8; 8] = [0; 8];
static mut CACHED_DATE_STR: [u8; 10] = [0; 10];

pub fn init(fb: &Framebuffer) {
    if fb.width == 0 || fb.height == 0 {
        return;
    }

    battery::init();
    refresh_background(fb);
}

pub fn refresh_background(fb: &Framebuffer) {
    if fb.width == 0 || fb.height == 0 {
        return;
    }
    let w = fb.width.min(MAX_W);
    let y = fb.height.saturating_sub(BAR_H);
    let mut idx = 0usize;
    for row in 0..BAR_H {
        for col in 0..w {
            let px = col;
            let py = y + row;
            unsafe {
                STATUS_BACK[idx] = display::get_pixel(fb, px, py);
            }
            idx += 1;
        }
    }
    unsafe {
        STATUS_W = w;
        STATUS_SAVED = true;
    }
}

pub fn draw(fb: &Framebuffer, now: RtcTime) {
    let settings = system::ui_settings();
    if !settings.status_bar {
        return;
    }

    battery::update();

    let bar_h = BAR_H;
    let y = fb.height.saturating_sub(bar_h);
    let w = fb.width.min(MAX_W);

    // 1. Відновлюємо оригінальний фон (прибираємо накопичення прозорості)
    restore_background(fb, y, w);

    // 2. Малюємо фон із прозорістю 30% (альфа = 70% непрозорості)
    //    Колір залежить від теми: темний або світлий
    let bg_color = if settings.dark { 0x00272727 } else { 0x00FDFEFF };
    let alpha = 0xB3; // 70% opacity → 30% transparency (змініть на 0xCC для меншої прозорості)
    for row in 0..bar_h {
        for col in 0..w {
            let px = col;
            let py = y + row;
            let current = display::get_pixel(fb, px, py);
            let blended = blend_alpha(current, bg_color, alpha);
            display::put_pixel(fb, px, py, blended);
        }
    }

    // 3. Малюємо текст, батарею, годинник
    let text_color = if settings.dark { 0x00F2F5F8 } else { 0x00121B29 };
    let detail_text_color = if settings.dark { 0x00B7C0CC } else { 0x004D5D72 };

    let mut writer = crate::TextWriter::new(*fb);
    let mut right_x = w.saturating_sub(12);

    draw_volume(fb, &mut writer, text_color, detail_text_color);

    if battery::has_battery() {
        right_x = draw_battery(fb, right_x, y, bar_h, &mut writer, text_color, detail_text_color);
    }

    let adjusted_now = system::apply_timezone(now);
    draw_clock_with_date(right_x, y, adjusted_now, &settings, &mut writer, text_color, detail_text_color);
}

pub fn volume_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let bar_y = fb.height.saturating_sub(BAR_H);
    let x = fb.width.saturating_sub(224);
    let y = bar_y + (BAR_H.saturating_sub(VOLUME_H)) / 2;
    (x, y, VOLUME_W, VOLUME_H)
}

pub fn volume_from_point(fb: &Framebuffer, px: usize, py: usize) -> Option<u8> {
    let r = volume_rect(fb);
    if px < r.0 || py < r.1 || px >= r.0 + r.2 || py >= r.1 + r.3 {
        return None;
    }
    let bar_x = r.0 + 32;
    let bar_w = r.2.saturating_sub(58);
    if bar_w == 0 {
        return Some(pc_speaker::get_volume());
    }
    if px <= bar_x {
        return Some(0);
    }
    let rel = px.saturating_sub(bar_x).min(bar_w);
    Some(((rel * 100) / bar_w) as u8)
}

pub fn volume_popup_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let bar_y = fb.height.saturating_sub(BAR_H);
    let x = fb.width.saturating_sub(POPUP_W + 14);
    let y = bar_y.saturating_sub(POPUP_H + 8);
    (x, y, POPUP_W, POPUP_H)
}

pub fn draw_volume_popup(fb: &Framebuffer) {
    let ui = system::ui_settings();
    let (x, y, w, h) = volume_popup_rect(fb);
    let volume = pc_speaker::get_volume() as usize;
    let text = if ui.dark { 0x00F6F8FB } else { 0x00141C28 };
    let muted = if ui.dark { 0x00B9C3CF } else { 0x00556373 };
    let bg = if ui.dark { 0x00202631 } else { 0x00F8FBFF };
    let edge = if ui.dark { 0x00647282 } else { 0x00B6C7DA };

    fill_translucent_round_rect(fb, x + 5, y + 6, w, h, 0x00000000, 36, 12);
    fill_translucent_round_rect(fb, x, y, w, h, bg, 91, 12);
    draw_round_outline(fb, x, y, w, h, 12, edge);
    display::fill_rect(fb, x + 13, y + 34, w.saturating_sub(26), 6, if ui.dark { 0x00151A22 } else { 0x00D4DFEC });
    let fill_w = (w.saturating_sub(26) * volume) / 100;
    if fill_w > 0 {
        display::fill_rect(fb, x + 13, y + 34, fill_w, 6, ui.accent);
    }

    let mut writer = crate::TextWriter::new(*fb);
    writer.set_color(text);
    writer.set_pos(x + 14, y + 12);
    writer.write_bytes(if volume == 0 { b"Muted" } else { b"Volume" });

    let mut buf = [0u8; 4];
    let len = write_percent(&mut buf, volume as u8);
    writer.set_color(muted);
    writer.set_pos(x + w.saturating_sub(14 + len * 8), y + 12);
    writer.write_bytes(&buf[..len]);
}

fn draw_volume(
    fb: &Framebuffer,
    writer: &mut crate::TextWriter,
    text_color: u32,
    detail_text_color: u32,
) {
    let (x, y, w, h) = volume_rect(fb);
    let volume = pc_speaker::get_volume() as usize;
    let ui = system::ui_settings();
    let bg = if ui.dark { 0x00303943 } else { 0x00E5EEF8 };
    let border = if ui.dark { 0x00505C6A } else { 0x00A8BCD2 };
    display::fill_rect(fb, x, y, w, h, bg);
    display::fill_rect(fb, x, y, w, 1, border);
    display::fill_rect(fb, x, y + h.saturating_sub(1), w, 1, border);
    display::fill_rect(fb, x, y, 1, h, border);
    display::fill_rect(fb, x + w.saturating_sub(1), y, 1, h, border);

    writer.set_color(text_color);
    writer.set_pos(x + 5, y + 6);
    writer.write_bytes(b"Vol");

    let bar_x = x + 32;
    let bar_y = y + 8;
    let bar_w = w.saturating_sub(58);
    display::fill_rect(fb, bar_x, bar_y, bar_w, 5, if ui.dark { 0x001B2027 } else { 0x00C8D6E6 });
    let fill_w = (bar_w * volume) / 100;
    if fill_w > 0 {
        display::fill_rect(fb, bar_x, bar_y, fill_w, 5, ui.accent);
    }

    let mut buf = [0u8; 4];
    let len = write_percent(&mut buf, volume as u8);
    writer.set_color(detail_text_color);
    writer.set_pos(x + w.saturating_sub(len * 8 + 4), y + 6);
    writer.write_bytes(&buf[..len]);
}

fn fill_translucent_round_rect(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, color: u32, alpha_percent: u32, radius: usize) {
    let end_x = (x + w).min(fb.width);
    let end_y = (y + h).min(fb.height);
    for py in y..end_y {
        for px in x..end_x {
            if inside_round_rect(px.saturating_sub(x), py.saturating_sub(y), w, h, radius) {
                let bg = display::get_pixel(fb, px, py);
                display::put_pixel(fb, px, py, blend_percent(bg, color, alpha_percent.min(100)));
            }
        }
    }
}

fn draw_round_outline(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, radius: usize, color: u32) {
    let end_x = (x + w).min(fb.width);
    let end_y = (y + h).min(fb.height);
    for py in y..end_y {
        for px in x..end_x {
            let outer = inside_round_rect(px.saturating_sub(x), py.saturating_sub(y), w, h, radius);
            let inner = px > x && py > y
                && inside_round_rect(px - x - 1, py - y - 1, w.saturating_sub(2), h.saturating_sub(2), radius.saturating_sub(1));
            if outer && !inner {
                display::put_pixel(fb, px, py, color);
            }
        }
    }
}

fn inside_round_rect(lx: usize, ly: usize, w: usize, h: usize, radius: usize) -> bool {
    if lx >= w || ly >= h || w == 0 || h == 0 {
        return false;
    }
    let r = radius.min(w / 2).min(h / 2);
    if r == 0 || (lx >= r && lx < w.saturating_sub(r)) || (ly >= r && ly < h.saturating_sub(r)) {
        return true;
    }
    let cx = if lx < r { r } else { w.saturating_sub(r + 1) };
    let cy = if ly < r { r } else { h.saturating_sub(r + 1) };
    let dx = if lx > cx { lx - cx } else { cx - lx };
    let dy = if ly > cy { ly - cy } else { cy - ly };
    dx * dx + dy * dy <= r * r
}

fn blend_percent(bg: u32, fg: u32, alpha: u32) -> u32 {
    let inv = 100 - alpha;
    let br = (bg >> 16) & 0xFF;
    let bgc = (bg >> 8) & 0xFF;
    let bb = bg & 0xFF;
    let fr = (fg >> 16) & 0xFF;
    let fg_g = (fg >> 8) & 0xFF;
    let fb = fg & 0xFF;
    let r = (br * inv + fr * alpha) / 100;
    let g = (bgc * inv + fg_g * alpha) / 100;
    let b = (bb * inv + fb * alpha) / 100;
    (r << 16) | (g << 8) | b
}

/// Відновлює оригінальний фон статус-бару зі збереженого буфера
fn restore_background(fb: &Framebuffer, y: usize, w: usize) {
    unsafe {
        if !STATUS_SAVED || STATUS_W == 0 {
            return;
        }
        let w_restore = STATUS_W.min(w);
        let mut idx = 0usize;
        for row in 0..BAR_H {
            for col in 0..w_restore {
                let px = col;
                let py = y + row;
                let rgb = STATUS_BACK[idx];
                display::put_pixel(fb, px, py, rgb);
                idx += 1;
            }
        }
    }
}

/// Змішує два кольори (фон і передній план) з прозорістю alpha (0..255)
/// alpha – непрозорість переднього плану (0 = повністю прозорий, 255 = повністю непрозорий)
fn blend_alpha(bg: u32, fg: u32, alpha: u8) -> u32 {
    let a = alpha as u32;
    let inv_a = 255 - a;
    let br = (bg >> 16) & 0xFF;
    let bg_ = (bg >> 8) & 0xFF;
    let bb = bg & 0xFF;
    let fr = (fg >> 16) & 0xFF;
    let fg_ = (fg >> 8) & 0xFF;
    let fb = fg & 0xFF;
    let r = (br * inv_a + fr * a) / 255;
    let g = (bg_ * inv_a + fg_ * a) / 255;
    let b = (bb * inv_a + fb * a) / 255;
    (r << 16) | (g << 8) | b
}

fn draw_battery(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    h: usize,
    writer: &mut crate::TextWriter,
    text_color: u32,
    detail_text_color: u32,
) -> usize {
    let level = battery::get_level();

    let icon_w = 24usize;
    let icon_h = 10usize;
    let icon_x = x.saturating_sub(94);
    let icon_y = y + (h.saturating_sub(icon_h)) / 2 + 1;

    // Корпус батареї
    let body_border = if level > 20 { detail_text_color } else { 0x00E84A5F };
    display::fill_rect(fb, icon_x, icon_y, icon_w, 1, body_border);
    display::fill_rect(fb, icon_x, icon_y + icon_h - 1, icon_w, 1, body_border);
    display::fill_rect(fb, icon_x, icon_y, 1, icon_h, body_border);
    display::fill_rect(fb, icon_x + icon_w - 1, icon_y, 1, icon_h, body_border);
    display::fill_rect(fb, icon_x + icon_w, icon_y + 3, 2, icon_h - 6, body_border);

    // Заливка рівня
    let fill_w = ((icon_w - 4) * level as usize) / 100;
    let fill_color = if level > 50 {
        0x0051B56B
    } else if level > 20 {
        0x00D39B39
    } else {
        0x00D14B55
    };
    if fill_w > 0 {
        display::fill_rect(
            fb,
            icon_x + 2,
            icon_y + 2,
            fill_w.min(icon_w - 4),
            icon_h - 4,
            fill_color,
        );
    }

    // Текст із відсотками
    writer.set_color(text_color);
    writer.set_pos(icon_x + icon_w + 8, y + h / 2 - 4);
    let mut buf = [0u8; 4];
    let idx = if level >= 100 {
        buf[0] = b'1';
        buf[1] = b'0';
        buf[2] = b'0';
        3
    } else if level >= 10 {
        buf[0] = b'0' + (level / 10) as u8;
        buf[1] = b'0' + (level % 10) as u8;
        2
    } else {
        buf[0] = b'0' + level as u8;
        1
    };
    buf[idx] = b'%';
    writer.write_bytes(&buf[..=idx]);

    icon_x.saturating_sub(8)
}

fn draw_clock_with_date(
    x: usize,
    y: usize,
    now: RtcTime,
    settings: &system::UiSettings,
    writer: &mut crate::TextWriter,
    text_color: u32,
    detail_text_color: u32,
) {
    let clock_x = x.saturating_sub(92);

    let time_hash = (now.hour as u32) << 16 | (now.min as u32) << 8 | (now.sec as u32);
    let use_cached = unsafe { time_hash == LAST_TIME_HASH };

    let time_str: &[u8] = if use_cached {
        unsafe { &CACHED_TIME_STR }
    } else {
        unsafe {
            format_time(&mut CACHED_TIME_STR, now, settings.clock_24h);
            LAST_TIME_HASH = time_hash;
            &CACHED_TIME_STR
        }
    };

    let date_str: &[u8] = if use_cached {
        unsafe { &CACHED_DATE_STR }
    } else {
        unsafe {
            format_date(&mut CACHED_DATE_STR, now);
            &CACHED_DATE_STR
        }
    };

    writer.set_color(text_color);
    let time_len = if settings.clock_24h { 5 } else { 7 };
    let time_w = time_len * 8;
    writer.set_pos(clock_x + (92 - time_w) / 2, y + 4);
    writer.write_bytes(&time_str[..time_len]);

    writer.set_color(detail_text_color);
    let date_len = date_str.iter().take_while(|&&b| b != 0).count();
    let date_w = date_len * 8;
    writer.set_pos(clock_x + (92 - date_w) / 2, y + 16);
    writer.write_bytes(&date_str[..date_len]);
}

fn format_time(buf: &mut [u8; 8], now: RtcTime, clock_24h: bool) {
    let mut hour = now.hour;

    if !clock_24h {
        if hour == 0 {
            hour = 12;
        } else if hour >= 12 {
            if hour > 12 {
                hour -= 12;
            }
            buf[5] = b'P';
            buf[6] = b'M';
        } else {
            buf[5] = b'A';
            buf[6] = b'M';
        }
        buf[7] = 0;
    } else {
        buf[5] = 0;
        buf[6] = 0;
        buf[7] = 0;
    }

    buf[0] = b'0' + (hour / 10) as u8;
    buf[1] = b'0' + (hour % 10) as u8;
    buf[2] = b':';
    buf[3] = b'0' + (now.min / 10) as u8;
    buf[4] = b'0' + (now.min % 10) as u8;
}

fn format_date(buf: &mut [u8; 10], now: RtcTime) {
    buf[0] = b'0' + (now.day / 10) as u8;
    buf[1] = b'0' + (now.day % 10) as u8;
    buf[2] = b'.';
    buf[3] = b'0' + (now.month / 10) as u8;
    buf[4] = b'0' + (now.month % 10) as u8;
    buf[5] = b'.';
    let year = now.year;
    buf[6] = b'0' + ((year / 10) % 10) as u8;
    buf[7] = b'0' + (year % 10) as u8;
    buf[8] = 0;
    buf[9] = 0;
}

fn write_percent(buf: &mut [u8; 4], value: u8) -> usize {
    let value = value.min(100);
    if value == 100 {
        buf[0] = b'1';
        buf[1] = b'0';
        buf[2] = b'0';
        buf[3] = b'%';
        4
    } else if value >= 10 {
        buf[0] = b'0' + value / 10;
        buf[1] = b'0' + value % 10;
        buf[2] = b'%';
        3
    } else {
        buf[0] = b'0' + value;
        buf[1] = b'%';
        2
    }
}

pub fn hide(fb: &Framebuffer) {
    unsafe {
        if !STATUS_SAVED || STATUS_W == 0 {
            return;
        }
    }
    let y = fb.height.saturating_sub(BAR_H);
    let w = unsafe { STATUS_W }.min(fb.width);
    let mut idx = 0usize;
    for row in 0..BAR_H {
        for col in 0..w {
            let px = col;
            let py = y + row;
            let rgb = unsafe { STATUS_BACK[idx] };
            display::put_pixel(fb, px, py, rgb);
            idx += 1;
        }
    }
}
