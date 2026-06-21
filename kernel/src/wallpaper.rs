use crate::desktop;
use crate::display::{self, Framebuffer};
use crate::fat32::{DirEntry, Fat32, MAX_NAME};
use crate::image;
use crate::status_bar;
use crate::system;
use crate::ModuleRange;

const WIN_W: usize = 632;
const WIN_H: usize = 374;
const PAD: usize = 22;
const TILE_W: usize = 170;
const TILE_H: usize = 108;
const BTN_H: usize = 32;
const ROUND: usize = 16;
const TILE_ROUND: usize = 10;
const JPEG_BUF_SIZE: usize = 900 * 1024;
const MAX_PICK_FILES: usize = 16;
const PICK_ROW_H: usize = 22;

static mut JPEG_FILE_BUF: [u8; JPEG_BUF_SIZE] = [0; JPEG_BUF_SIZE];

#[derive(Copy, Clone)]
struct PickEntry {
    name: [u8; MAX_NAME],
    name_len: usize,
    cluster: u32,
    size: u32,
}

impl PickEntry {
    const EMPTY: PickEntry = PickEntry {
        name: [0; MAX_NAME],
        name_len: 0,
        cluster: 0,
        size: 0,
    };
}

pub struct WallpaperWindow {
    visible: bool,
    picker_open: bool,
    solo_mode: bool,
    fs_img: Option<ModuleRange>,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    status: [u8; 72],
    status_len: usize,
    files: [PickEntry; MAX_PICK_FILES],
    file_count: usize,
}

impl WallpaperWindow {
    pub fn new(fb: Framebuffer, fs_img: Option<ModuleRange>) -> Self {
        let (x, y, w, h) = calc_rect(&fb);
        Self {
            visible: false,
            picker_open: false,
            solo_mode: false,
            fs_img,
            x,
            y,
            w,
            h,
            status: [0; 72],
            status_len: 0,
            files: [PickEntry::EMPTY; MAX_PICK_FILES],
            file_count: 0,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn solo_mode(&self) -> bool {
        self.visible && self.solo_mode
    }

    pub fn rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        if self.w == 0 || self.h == 0 {
            calc_rect(fb)
        } else {
            (self.x, self.y, self.w, self.h)
        }
    }

    pub fn show(&mut self, fb: &Framebuffer) {
        let (x, y, w, h) = calc_rect(fb);
        self.x = x;
        self.y = y;
        self.w = w;
        self.h = h;
        self.visible = true;
        self.picker_open = false;
        self.solo_mode = true;
        self.set_status(b"Choose built-in or open Explorer.");
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.solo_mode = false;
    }

    pub fn handle_click(&mut self, fb: &Framebuffer, mx: usize, my: usize) -> bool {
        if !self.visible {
            return false;
        }
        let (x, y, w, h) = self.rect(fb);
        if mx < x || my < y || mx >= x + w || my >= y + h {
            return false;
        }

        let close = (x + w.saturating_sub(40), y + 12, 26, 24);
        if hit(mx, my, close) {
            self.hide();
            return true;
        }

        if self.picker_open {
            return self.handle_picker_click(fb, mx, my);
        }

        for i in 0..3 {
            let tx = x + PAD + i * (TILE_W + 20);
            let ty = y + 96;
            if hit(mx, my, (tx, ty, TILE_W, TILE_H + 30)) {
                self.apply_builtin(fb, i);
                return true;
            }
        }

        let add = (x + PAD, y + h.saturating_sub(PAD + BTN_H), 178, BTN_H);
        if hit(mx, my, add) {
            self.open_picker();
            return true;
        }

        let solo = (x + PAD + 158, y + h.saturating_sub(PAD + BTN_H), 156, BTN_H);
        if hit(mx, my, solo) {
            self.solo_mode = !self.solo_mode;
            if self.solo_mode {
                self.set_status(b"Other windows hidden.");
            } else {
                self.set_status(b"Other windows visible.");
            }
            return true;
        }

        true
    }

    pub fn redraw(&self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }
        let (x, y, w, h) = self.rect(fb);
        let ui = system::ui_settings();
        let text = if ui.dark { 0x00F4F7FA } else { 0x001B2430 };
        let muted = if ui.dark { 0x00B9C2CE } else { 0x005A6573 };
        let panel = if ui.dark { 0x00232933 } else { 0x00F7FAFE };
        let edge = if ui.dark { 0x006D7784 } else { 0x00D7E1EC };
        let accent = ui.accent;
        let header = if ui.dark { 0x00313A46 } else { 0x00EAF2FB };

        fill_translucent_round_rect(fb, x + 7, y + 9, w, h, 0x00000000, 36, ROUND);
        fill_translucent_round_rect(fb, x, y, w, h, panel, 86, ROUND);
        fill_translucent_top_round_rect(fb, x, y, w, 50, header, 88, ROUND);
        draw_round_outline(fb, x, y, w, h, ROUND, edge);
        display::fill_rect(fb, x + 1, y + 49, w.saturating_sub(2), 1, edge);

        let mut writer = crate::TextWriter::new(*fb);
        writer.set_color(text);
        writer.set_pos(x + PAD, y + 18);
        writer.write_bytes(b"Wallpaper");

        draw_button(fb, x + w.saturating_sub(40), y + 12, 26, 24, b"X", text, 0x00FFFFFF);

        writer.set_color(muted);
        writer.set_pos(x + PAD, y + 62);
        if self.picker_open {
            writer.write_bytes(b"Explorer - HOUSE_OS/WALLPAPE");
            self.draw_picker(fb, x, y, w, h, text, muted, accent);
            return;
        }
        writer.write_bytes(b"Built-in backgrounds");

        let names: [&[u8]; 3] = [b"BG 1", b"BG 2", b"BG 3"];
        for i in 0..3 {
            let tx = x + PAD + i * (TILE_W + 20);
            let ty = y + 96;
            if !self.draw_builtin_preview(fb, i, tx, ty, TILE_W, TILE_H) {
                draw_preview(fb, tx, ty, TILE_W, TILE_H, fallback_top(i), fallback_bottom(i));
            }
            writer.set_color(text);
            writer.set_pos(tx + 63, ty + TILE_H + 12);
            writer.write_bytes(names[i]);
        }

        let add_y = y + h.saturating_sub(PAD + BTN_H);
        draw_button(fb, x + PAD, add_y, 146, BTN_H, b"Add own", text, accent);
        let solo_label: &[u8] = if self.solo_mode { b"Show apps" } else { b"Hide apps" };
        draw_button(fb, x + PAD + 158, add_y, 156, BTN_H, solo_label, text, 0x00FFFFFF);

        writer.set_color(muted);
        writer.set_pos(x + PAD + 330, add_y + 9);
        if self.status_len > 0 {
            writer.write_bytes(&self.status[..self.status_len]);
        }
    }

    fn draw_builtin_preview(&self, fb: &Framebuffer, index: usize, x: usize, y: usize, w: usize, h: usize) -> bool {
        let names: [&[u8]; 3] = [b"BG1.JPG", b"BG2.JPG", b"BG3.JPG"];
        if index >= names.len() {
            return false;
        }
        self.load_preview(fb, &[b"HOUSE_OS", b"WALLPAPE"], names[index], x, y, w, h)
    }

    fn load_preview(&self, fb: &Framebuffer, dirs: &[&[u8]], name: &[u8], x: usize, y: usize, w: usize, h: usize) -> bool {
        let range = match self.fs_img {
            Some(v) => v,
            None => return false,
        };
        let fs = match Fat32::new(range) {
            Some(v) => v,
            None => return false,
        };
        let mut cluster = fs.root_cluster();
        for dir in dirs {
            cluster = match fs.find_dir(cluster, dir) {
                Some(v) => v,
                None => return false,
            };
        }
        let entry = match fs.find_file(cluster, name) {
            Some(v) => v,
            None => return false,
        };
        let size = entry.size as usize;
        if size == 0 || size > JPEG_BUF_SIZE {
            return false;
        }
        let read = unsafe {
            let buf = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(JPEG_FILE_BUF) as *mut u8,
                JPEG_BUF_SIZE,
            );
            fs.read_file(entry.cluster, size, buf)
        };
        if read == 0 {
            return false;
        }
        let decoded = unsafe {
            let data = core::slice::from_raw_parts(core::ptr::addr_of!(JPEG_FILE_BUF) as *const u8, read);
            image::decode_jpeg(data)
        };
        if !decoded {
            return false;
        }
        draw_bgra_tile(fb, image::get_bgra_ptr(), image::get_bgra_len(), x, y, w, h, TILE_ROUND)
    }

    fn apply_builtin(&mut self, fb: &Framebuffer, index: usize) {
        let names: [&[u8]; 3] = [b"BG1.JPG", b"BG2.JPG", b"BG3.JPG"];
        if self.load_and_apply(fb, &[b"HOUSE_OS", b"WALLPAPE"], names[index]) {
            self.set_status(b"Wallpaper changed.");
        } else {
            self.set_status(b"Wallpaper JPG not found or too big.");
        }
    }

    fn open_picker(&mut self) {
        self.picker_open = true;
        self.load_wallpaper_list();
        if self.file_count == 0 {
            self.set_status(b"No JPG in HOUSE_OS/WALLPAPE.");
        } else {
            self.set_status(b"Click a JPG to apply it.");
        }
    }

    fn load_and_apply(&mut self, fb: &Framebuffer, dirs: &[&[u8]], name: &[u8]) -> bool {
        let range = match self.fs_img {
            Some(v) => v,
            None => return false,
        };
        let fs = match Fat32::new(range) {
            Some(v) => v,
            None => return false,
        };
        let mut cluster = fs.root_cluster();
        for dir in dirs {
            cluster = match fs.find_dir(cluster, dir) {
                Some(v) => v,
                None => return false,
            };
        }
        let entry = match fs.find_file(cluster, name) {
            Some(v) => v,
            None => return false,
        };
        let size = entry.size as usize;
        if size == 0 || size > JPEG_BUF_SIZE {
            return false;
        }
        let read = unsafe {
            let buf = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(JPEG_FILE_BUF) as *mut u8,
                JPEG_BUF_SIZE,
            );
            fs.read_file(entry.cluster, size, buf)
        };
        if read == 0 {
            return false;
        }
        let decoded = unsafe {
            let data = core::slice::from_raw_parts(core::ptr::addr_of!(JPEG_FILE_BUF) as *const u8, read);
            image::decode_jpeg(data)
        };
        if !decoded {
            return false;
        }
        if !display::draw_bgra_image(fb, image::get_bgra_ptr(), image::get_bgra_len()) {
            return false;
        }
        desktop::capture(fb);
        status_bar::refresh_background(fb);
        true
    }

    fn load_entry_and_apply(&mut self, fb: &Framebuffer, entry: PickEntry) -> bool {
        if entry.cluster < 2 || entry.size == 0 || entry.size as usize > JPEG_BUF_SIZE {
            return false;
        }
        let range = match self.fs_img {
            Some(v) => v,
            None => return false,
        };
        let fs = match Fat32::new(range) {
            Some(v) => v,
            None => return false,
        };
        let read = unsafe {
            let buf = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(JPEG_FILE_BUF) as *mut u8,
                JPEG_BUF_SIZE,
            );
            fs.read_file(entry.cluster, entry.size as usize, buf)
        };
        if read == 0 {
            return false;
        }
        let decoded = unsafe {
            let data = core::slice::from_raw_parts(core::ptr::addr_of!(JPEG_FILE_BUF) as *const u8, read);
            image::decode_jpeg(data)
        };
        if !decoded {
            return false;
        }
        if !display::draw_bgra_image(fb, image::get_bgra_ptr(), image::get_bgra_len()) {
            return false;
        }
        desktop::capture(fb);
        status_bar::refresh_background(fb);
        true
    }

    fn load_wallpaper_list(&mut self) {
        self.file_count = 0;
        let range = match self.fs_img {
            Some(v) => v,
            None => return,
        };
        let fs = match Fat32::new(range) {
            Some(v) => v,
            None => return,
        };
        let mut cluster = fs.root_cluster();
        cluster = match fs.find_dir(cluster, b"HOUSE_OS") {
            Some(v) => v,
            None => return,
        };
        cluster = match fs.find_dir(cluster, b"WALLPAPE") {
            Some(v) => v,
            None => return,
        };
        let mut entries = [DirEntry::EMPTY; 64];
        let count = fs.list_dir(cluster, &mut entries);
        for i in 0..count {
            let entry = entries[i];
            if entry.is_dir || !is_jpeg_name(&entry.name[..entry.name_len]) {
                continue;
            }
            if self.file_count >= MAX_PICK_FILES {
                break;
            }
            self.files[self.file_count] = PickEntry {
                name: entry.name,
                name_len: entry.name_len,
                cluster: entry.cluster,
                size: entry.size,
            };
            self.file_count += 1;
        }
    }

    fn handle_picker_click(&mut self, fb: &Framebuffer, mx: usize, my: usize) -> bool {
        let (x, y, w, h) = self.rect(fb);
        let back = (x + PAD, y + h.saturating_sub(PAD + BTN_H), 88, BTN_H);
        if hit(mx, my, back) {
            self.picker_open = false;
            self.set_status(b"Choose built-in or open Explorer.");
            return true;
        }
        let list_x = x + PAD;
        let list_y = y + 70;
        let list_w = w.saturating_sub(PAD * 2);
        let max_rows = (h.saturating_sub(122) / PICK_ROW_H).max(1);
        let rows = self.file_count.min(max_rows);
        for i in 0..rows {
            let row_y = list_y + i * PICK_ROW_H;
            if hit(mx, my, (list_x, row_y, list_w, PICK_ROW_H)) {
                let entry = self.files[i];
                if self.load_entry_and_apply(fb, entry) {
                    self.set_status(b"Wallpaper applied from Explorer.");
                    self.picker_open = false;
                } else {
                    self.set_status(b"Could not decode selected JPG.");
                }
                return true;
            }
        }
        true
    }

    fn draw_picker(&self, fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, text: u32, muted: u32, accent: u32) {
        let list_x = x + PAD;
        let list_y = y + 70;
        let list_w = w.saturating_sub(PAD * 2);
        let max_rows = (h.saturating_sub(122) / PICK_ROW_H).max(1);
        let rows = self.file_count.min(max_rows);
        display::fill_rect(fb, list_x, list_y.saturating_sub(4), list_w, rows.max(1) * PICK_ROW_H + 8, 0x00FFFFFF);
        display::fill_rect(fb, list_x, list_y.saturating_sub(4), list_w, 1, 0x00C8D3E0);
        display::fill_rect(fb, list_x, list_y + rows.max(1) * PICK_ROW_H + 4, list_w, 1, 0x00C8D3E0);
        let mut writer = crate::TextWriter::new(*fb);
        if self.file_count == 0 {
            writer.set_color(muted);
            writer.set_pos(list_x + 10, list_y + 8);
            writer.write_bytes(b"No wallpaper files on disk.");
        } else {
            for i in 0..rows {
                let row_y = list_y + i * PICK_ROW_H;
                let bg = if i % 2 == 0 { 0x00F6FAFE } else { 0x00EEF4FA };
                display::fill_rect(fb, list_x + 1, row_y, list_w.saturating_sub(2), PICK_ROW_H, bg);
                writer.set_color(accent);
                writer.set_pos(list_x + 8, row_y + 7);
                writer.write_bytes(b"#");
                writer.set_color(text);
                writer.set_pos(list_x + 26, row_y + 7);
                writer.write_bytes(&self.files[i].name[..self.files[i].name_len]);
            }
        }
        let add_y = y + h.saturating_sub(PAD + BTN_H);
        draw_button(fb, x + PAD, add_y, 88, BTN_H, b"Back", text, 0x00FFFFFF);
        writer.set_color(muted);
        writer.set_pos(x + PAD + 104, add_y + 9);
        if self.status_len > 0 {
            writer.write_bytes(&self.status[..self.status_len]);
        }
    }

    fn set_status(&mut self, msg: &[u8]) {
        self.status_len = msg.len().min(self.status.len());
        self.status[..self.status_len].copy_from_slice(&msg[..self.status_len]);
    }
}

fn is_jpeg_name(name: &[u8]) -> bool {
    ends_with_ci(name, b".JPG") || ends_with_ci(name, b".JPEG")
}

fn ends_with_ci(value: &[u8], suffix: &[u8]) -> bool {
    if value.len() < suffix.len() {
        return false;
    }
    let start = value.len() - suffix.len();
    for i in 0..suffix.len() {
        if ascii_lower(value[start + i]) != ascii_lower(suffix[i]) {
            return false;
        }
    }
    true
}

fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

fn calc_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let w = WIN_W.min(fb.width.saturating_sub(32)).max(260);
    let h = WIN_H.min(fb.height.saturating_sub(70)).max(220);
    let x = fb.width.saturating_sub(w) / 2;
    let y = fb.height.saturating_sub(h) / 2;
    (x, y, w, h)
}

fn hit(mx: usize, my: usize, rect: (usize, usize, usize, usize)) -> bool {
    let (x, y, w, h) = rect;
    mx >= x && my >= y && mx < x + w && my < y + h
}

fn draw_preview(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, top: u32, bottom: u32) {
    let end_x = (x + w).min(fb.width);
    let end_y = (y + h).min(fb.height);
    for py in y..end_y {
        let c = lerp_rgb(top, bottom, py.saturating_sub(y) as u32, h.saturating_sub(1).max(1) as u32);
        for px in x..end_x {
            if inside_round_rect_abs(px, py, x, y, w, h, TILE_ROUND) {
                display::put_pixel(fb, px, py, c);
            }
        }
    }
    draw_round_outline(fb, x, y, w, h, TILE_ROUND, 0x00FFFFFF);
}

fn draw_button(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, label: &[u8], text: u32, color: u32) {
    let fill = if color == 0x00FFFFFF { 0x00EDF3FA } else { color };
    let radius = if h >= 30 { 9 } else { 7 };
    fill_round_rect(fb, x, y, w, h, fill, radius);
    draw_round_outline(fb, x, y, w, h, radius, if color == 0x00FFFFFF { 0x0090A0B2 } else { 0x00FFFFFF });
    let mut writer = crate::TextWriter::new(*fb);
    writer.set_color(if color == 0x00FFFFFF { text } else { 0x00FFFFFF });
    let tx = x + w.saturating_sub(label.len() * 8) / 2;
    let ty = y + h.saturating_sub(8) / 2;
    writer.set_pos(tx, ty);
    writer.write_bytes(label);
}

fn draw_bgra_tile(fb: &Framebuffer, data: *const u8, size: usize, x: usize, y: usize, w: usize, h: usize, radius: usize) -> bool {
    if data.is_null() || size < 8 || w == 0 || h == 0 {
        return false;
    }
    let src_w = read_u32_le_ptr(data) as usize;
    let src_h = read_u32_le_ptr(unsafe { data.add(4) }) as usize;
    if src_w == 0 || src_h == 0 {
        return false;
    }
    let pixel_len = size - 8;
    let expected = src_w.saturating_mul(src_h).saturating_mul(4);
    if pixel_len < expected {
        return false;
    }
    let src_ratio = (src_w as u64) * (h as u64);
    let dst_ratio = (w as u64) * (src_h as u64);
    let mut sample_w = src_w;
    let mut sample_h = src_h;
    let mut crop_x = 0usize;
    let mut crop_y = 0usize;
    if src_ratio > dst_ratio {
        sample_w = ceil_div((src_h as u64) * (w as u64), h as u64) as usize;
        if sample_w == 0 {
            sample_w = 1;
        }
        if sample_w < src_w {
            crop_x = (src_w - sample_w) / 2;
        } else {
            sample_w = src_w;
        }
    } else if dst_ratio > src_ratio {
        sample_h = ceil_div((src_w as u64) * (h as u64), w as u64) as usize;
        if sample_h == 0 {
            sample_h = 1;
        }
        if sample_h < src_h {
            crop_y = (src_h - sample_h) / 2;
        } else {
            sample_h = src_h;
        }
    }
    let step_x = ((sample_w as u32) << 16) / (w as u32);
    let step_y = ((sample_h as u32) << 16) / (h as u32);
    let pixels = unsafe { data.add(8) };
    let mut sy_fp = 0u32;
    for dy in 0..h {
        let py = y + dy;
        if py >= fb.height {
            break;
        }
        let sy = crop_y + (sy_fp >> 16) as usize;
        let row = unsafe { pixels.add(sy * src_w * 4) };
        let mut sx_fp = 0u32;
        for dx in 0..w {
            let px = x + dx;
            if px >= fb.width {
                break;
            }
            if inside_round_rect_abs(px, py, x, y, w, h, radius) {
                let sx = crop_x + (sx_fp >> 16) as usize;
                let p = unsafe { row.add(sx * 4) };
                let b = unsafe { p.read() } as u32;
                let g = unsafe { p.add(1).read() } as u32;
                let r = unsafe { p.add(2).read() } as u32;
                display::put_pixel(fb, px, py, (r << 16) | (g << 8) | b);
            }
            sx_fp = sx_fp.wrapping_add(step_x);
        }
        sy_fp = sy_fp.wrapping_add(step_y);
    }
    draw_round_outline(fb, x, y, w, h, radius, 0x00FFFFFF);
    true
}

fn fill_round_rect(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, color: u32, radius: usize) {
    let end_x = (x + w).min(fb.width);
    let end_y = (y + h).min(fb.height);
    for py in y..end_y {
        for px in x..end_x {
            if inside_round_rect_abs(px, py, x, y, w, h, radius) {
                display::put_pixel(fb, px, py, color);
            }
        }
    }
}

fn fill_translucent_round_rect(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, color: u32, alpha: u32, radius: usize) {
    let end_x = (x + w).min(fb.width);
    let end_y = (y + h).min(fb.height);
    for py in y..end_y {
        for px in x..end_x {
            if inside_round_rect_abs(px, py, x, y, w, h, radius) {
                let bg = display::get_pixel(fb, px, py);
                display::put_pixel(fb, px, py, blend(bg, color, alpha.min(100)));
            }
        }
    }
}

fn fill_translucent_top_round_rect(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, color: u32, alpha: u32, radius: usize) {
    let end_x = (x + w).min(fb.width);
    let end_y = (y + h).min(fb.height);
    for py in y..end_y {
        for px in x..end_x {
            if inside_top_round_rect_abs(px, py, x, y, w, h, radius) {
                let bg = display::get_pixel(fb, px, py);
                display::put_pixel(fb, px, py, blend(bg, color, alpha.min(100)));
            }
        }
    }
}

fn draw_round_outline(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, radius: usize, color: u32) {
    if w < 2 || h < 2 {
        return;
    }
    let end_x = (x + w).min(fb.width);
    let end_y = (y + h).min(fb.height);
    for py in y..end_y {
        for px in x..end_x {
            let outer = inside_round_rect_abs(px, py, x, y, w, h, radius);
            let inner = inside_round_rect_abs(px, py, x + 1, y + 1, w.saturating_sub(2), h.saturating_sub(2), radius.saturating_sub(1));
            if outer && !inner {
                display::put_pixel(fb, px, py, color);
            }
        }
    }
}

fn inside_round_rect_abs(px: usize, py: usize, x: usize, y: usize, w: usize, h: usize, radius: usize) -> bool {
    if px < x || py < y || px >= x + w || py >= y + h || w == 0 || h == 0 {
        return false;
    }
    inside_round_rect_local(px - x, py - y, w, h, radius)
}

fn inside_top_round_rect_abs(px: usize, py: usize, x: usize, y: usize, w: usize, h: usize, radius: usize) -> bool {
    if px < x || py < y || px >= x + w || py >= y + h || w == 0 || h == 0 {
        return false;
    }
    let lx = px - x;
    let ly = py - y;
    let r = radius.min(w / 2).min(h / 2);
    if r == 0 || ly >= r || (lx >= r && lx < w.saturating_sub(r)) {
        return true;
    }
    let cx = if lx < r { r } else { w.saturating_sub(r + 1) };
    let dx = abs_diff(lx, cx);
    let dy = abs_diff(ly, r);
    dx * dx + dy * dy <= r * r
}

fn inside_round_rect_local(lx: usize, ly: usize, w: usize, h: usize, radius: usize) -> bool {
    let r = radius.min(w / 2).min(h / 2);
    if r == 0 || (lx >= r && lx < w.saturating_sub(r)) || (ly >= r && ly < h.saturating_sub(r)) {
        return true;
    }
    let cx = if lx < r { r } else { w.saturating_sub(r + 1) };
    let cy = if ly < r { r } else { h.saturating_sub(r + 1) };
    let dx = abs_diff(lx, cx);
    let dy = abs_diff(ly, cy);
    dx * dx + dy * dy <= r * r
}

fn abs_diff(a: usize, b: usize) -> usize {
    if a > b { a - b } else { b - a }
}

fn read_u32_le_ptr(ptr: *const u8) -> u32 {
    unsafe {
        (ptr.read() as u32)
            | ((ptr.add(1).read() as u32) << 8)
            | ((ptr.add(2).read() as u32) << 16)
            | ((ptr.add(3).read() as u32) << 24)
    }
}

fn ceil_div(value: u64, den: u64) -> u64 {
    if den == 0 {
        0
    } else {
        (value + den - 1) / den
    }
}

fn fallback_top(index: usize) -> u32 {
    match index {
        0 => 0x0088C7F0,
        1 => 0x00D8E5F4,
        _ => 0x00E8D8F0,
    }
}

fn fallback_bottom(index: usize) -> u32 {
    match index {
        0 => 0x001D5F8B,
        1 => 0x006F8FB8,
        _ => 0x007A4D92,
    }
}

fn blend(bg: u32, fg: u32, alpha: u32) -> u32 {
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

fn lerp_rgb(a: u32, b: u32, num: u32, den: u32) -> u32 {
    let ar = (a >> 16) & 0xFF;
    let ag = (a >> 8) & 0xFF;
    let ab = a & 0xFF;
    let br = (b >> 16) & 0xFF;
    let bg = (b >> 8) & 0xFF;
    let bb = b & 0xFF;
    let r = (ar * (den - num) + br * num) / den;
    let g = (ag * (den - num) + bg * num) / den;
    let bl = (ab * (den - num) + bb * num) / den;
    (r << 16) | (g << 8) | bl
}
