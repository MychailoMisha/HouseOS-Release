use crate::display::{self, Framebuffer};
use crate::drivers::audio::pc_speaker;
use crate::fat32::{DirEntry, Fat32};
use crate::image;
use crate::system;
use crate::window;
use crate::ModuleRange;

const MAX_AUDIO: usize = 131072;
const MAX_IMAGE_FILE: usize = 6 * 1024 * 1024;
const MAX_IMAGES: usize = 64;
const MAX_TITLE: usize = 48;
const PAD: usize = 12;
const MODE_AUDIO: u8 = 0;
const MODE_IMAGE: u8 = 1;
const MODE_VIDEO: u8 = 2;

static mut IMAGE_INPUT: [u8; MAX_IMAGE_FILE] = [0; MAX_IMAGE_FILE];

#[derive(Copy, Clone)]
struct Mp4Info {
    valid: bool,
    brand: [u8; 8],
    brand_len: usize,
    codec: [u8; 8],
    codec_len: usize,
    width: u32,
    height: u32,
    timescale: u32,
    duration: u32,
    tracks: u8,
    mdat_offset: usize,
    mdat_len: usize,
    jpeg_start: usize,
    jpeg_len: usize,
}

impl Mp4Info {
    const EMPTY: Mp4Info = Mp4Info {
        valid: false,
        brand: [0; 8],
        brand_len: 0,
        codec: [0; 8],
        codec_len: 0,
        width: 0,
        height: 0,
        timescale: 0,
        duration: 0,
        tracks: 0,
        mdat_offset: 0,
        mdat_len: 0,
        jpeg_start: 0,
        jpeg_len: 0,
    };
}

pub struct MediaPlayer {
    visible: bool,
    win_x: usize,
    win_y: usize,
    win_w: usize,
    win_h: usize,
    fs_img: Option<ModuleRange>,
    title: [u8; MAX_TITLE],
    title_len: usize,
    data: [u8; MAX_AUDIO],
    data_len: usize,
    status: [u8; 64],
    status_len: usize,
    mode: u8,
    image_parent_cluster: u32,
    image_entries: [DirEntry; MAX_IMAGES],
    image_count: usize,
    image_index: usize,
    image_loaded: bool,
    video_frame: u32,
    video_playing: bool,
    mp4_info: Mp4Info,
}

impl MediaPlayer {
    pub fn new(_fb: Framebuffer, fs_img: Option<ModuleRange>) -> Self {
        let mut title = [0u8; MAX_TITLE];
        let base = b"Media Player";
        title[..base.len()].copy_from_slice(base);
        Self {
            visible: false,
            win_x: 0,
            win_y: 0,
            win_w: 0,
            win_h: 0,
            fs_img,
            title,
            title_len: base.len(),
            data: [0; MAX_AUDIO],
            data_len: 0,
            status: [0; 64],
            status_len: 0,
            mode: MODE_AUDIO,
            image_parent_cluster: 0,
            image_entries: [DirEntry::EMPTY; MAX_IMAGES],
            image_count: 0,
            image_index: 0,
            image_loaded: false,
            video_frame: 0,
            video_playing: false,
            mp4_info: Mp4Info::EMPTY,
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

    pub fn open_empty(&mut self, fb: &Framebuffer) {
        self.show(fb);
        self.set_title(b"Music Player");
        self.mode = MODE_AUDIO;
        self.data_len = 0;
        self.image_loaded = false;
        self.video_playing = false;
        self.mp4_info = Mp4Info::EMPTY;
        self.set_status(b"No music loaded. Press Play for demo sound.");
    }

    pub fn open_photo_viewer(&mut self, fb: &Framebuffer) {
        self.show(fb);
        self.set_title(b"Photo Viewer");
        self.mode = MODE_IMAGE;
        self.data_len = 0;
        self.image_loaded = false;
        self.image_count = 0;
        self.image_index = 0;
        self.video_playing = false;
        self.mp4_info = Mp4Info::EMPTY;
        self.set_status(b"Open JPG/JPEG from Explorer to browse photos.");
    }

    pub fn open_video_player(&mut self, fb: &Framebuffer) {
        self.show(fb);
        self.set_title(b"Video Player");
        self.mode = MODE_VIDEO;
        self.data_len = 0;
        self.image_loaded = false;
        self.video_frame = 0;
        self.video_playing = false;
        self.mp4_info = Mp4Info::EMPTY;
        self.set_status(b"Video preview ready. Press Play.");
    }

    pub fn open_file(&mut self, fb: &Framebuffer, cluster: u32, size: u32, file_name: &[u8]) {
        self.show(fb);
        self.set_title(file_name);
        if is_video_name(file_name) {
            self.mode = MODE_VIDEO;
            self.video_frame = 0;
            self.video_playing = false;
            self.mp4_info = Mp4Info::EMPTY;
        } else {
            self.mode = MODE_AUDIO;
            self.video_playing = false;
            self.mp4_info = Mp4Info::EMPTY;
        }
        self.image_loaded = false;
        self.data_len = 0;
        let fs_img = self.fs_img;
        if let Some(fs) = fs_img.and_then(Fat32::new) {
            let read = fs.read_file(cluster, size as usize, &mut self.data);
            self.data_len = read.min(MAX_AUDIO);
            if self.mode == MODE_VIDEO {
                self.mp4_info = parse_mp4_info(&self.data[..self.data_len]);
                if self.mp4_info.valid {
                    if self.mp4_info.jpeg_len > 0 {
                        self.set_status(b"MP4 decoded: embedded JPEG frame ready.");
                    } else {
                        self.set_status(b"MP4 container decoded. H264/HEVC stream preview.");
                    }
                } else {
                    self.set_status(b"Video file opened. Unknown container.");
                }
            } else if self.data_len == 0 {
                self.set_status(b"Could not read audio");
            } else if wav_info(&self.data[..self.data_len]).is_some() {
                self.set_status(b"PCM WAV loaded");
            } else if mp3_first_frame(&self.data[..self.data_len]).is_some() {
                self.set_status(b"MP3 loaded (frame decoder)");
            } else {
                self.set_status(b"Raw sound loaded");
            }
        } else {
            self.set_status(b"No filesystem image");
        }
    }

    pub fn open_image_file(
        &mut self,
        fb: &Framebuffer,
        parent_cluster: u32,
        cluster: u32,
        size: u32,
        file_name: &[u8],
    ) {
        self.show(fb);
        self.mode = MODE_IMAGE;
        self.image_parent_cluster = parent_cluster;
        self.image_loaded = false;
        self.data_len = 0;
        self.set_title(file_name);
        self.build_image_list(parent_cluster, cluster, size, file_name);
        self.load_image_index(self.image_index);
    }

    pub fn handle_click(&mut self, fb: &Framebuffer, x: usize, y: usize) -> bool {
        if !self.visible {
            return false;
        }
        if self.mode == MODE_IMAGE {
            let prev = self.prev_rect(fb);
            let next = self.next_rect(fb);
            if hit(x, y, prev.0, prev.1, prev.2, prev.3) {
                self.prev_image();
                self.redraw(fb);
                return true;
            }
            if hit(x, y, next.0, next.1, next.2, next.3) {
                self.next_image();
                self.redraw(fb);
                return true;
            }
            return true;
        }
        if self.mode == MODE_VIDEO {
            let play = self.play_rect(fb);
            if hit(x, y, play.0, play.1, play.2, play.3) {
                self.play_video_step();
                self.redraw(fb);
                return true;
            }
            return true;
        }
        let play = self.play_rect(fb);
        if hit(x, y, play.0, play.1, play.2, play.3) {
            self.play_loaded();
            self.redraw(fb);
            return true;
        }
        false
    }

    pub fn redraw(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }
        if self.mode == MODE_IMAGE {
            self.redraw_image(fb);
            return;
        }
        if self.mode == MODE_VIDEO {
            self.redraw_video(fb);
            return;
        }
        let (x, y, w, h) = self.rect(fb);
        let ui = system::ui_settings();
        let is_dark = ui.dark;
        let chrome = window::draw_window(fb, x, y, w, h, b"Music Player");
        fill_vertical_gradient(
            fb,
            chrome.content_x,
            chrome.content_y,
            chrome.content_w,
            chrome.content_h,
            if is_dark { 0x00191B20 } else { 0x00FFFFFF },
            if is_dark { 0x0013161B } else { 0x00F2F7FC },
        );

        let mut writer = crate::TextWriter::new(*fb);
        let text = if is_dark { 0x00F2F5F8 } else { 0x00131A28 };
        let detail = if is_dark { 0x00B7C0CC } else { 0x004D5D72 };

        writer.set_color(text);
        writer.set_pos(chrome.content_x + PAD, chrome.content_y + PAD);
        writer.write_bytes(b"Track: ");
        writer.write_bytes(&self.title[..self.title_len]);

        let mut size_line = [0u8; 32];
        let mut p = 0usize;
        p += write_str(&mut size_line[p..], b"Loaded bytes: ");
        p += write_u32(&mut size_line[p..], self.data_len as u32);
        writer.set_color(detail);
        writer.set_pos(chrome.content_x + PAD, chrome.content_y + PAD + 22);
        writer.write_bytes(&size_line[..p]);

        writer.set_pos(chrome.content_x + PAD, chrome.content_y + PAD + 44);
        writer.write_bytes(&self.status[..self.status_len]);

        let mut volume_line = [0u8; 32];
        let mut vp = 0usize;
        vp += write_str(&mut volume_line[vp..], b"Volume: ");
        vp += write_u32(&mut volume_line[vp..], pc_speaker::get_volume() as u32);
        vp += write_str(&mut volume_line[vp..], b"%");
        writer.set_pos(chrome.content_x + PAD, chrome.content_y + PAD + 66);
        writer.write_bytes(&volume_line[..vp]);

        let play = self.play_rect(fb);
        fill_vertical_gradient(
            fb,
            play.0,
            play.1,
            play.2,
            play.3,
            if is_dark { 0x00375C76 } else { 0x00CDEAFF },
            if is_dark { 0x002B465B } else { 0x009FD6FF },
        );
        writer.set_color(if is_dark { 0x00FFFFFF } else { 0x000F2433 });
        writer.set_pos(play.0 + 18, play.1 + 8);
        writer.write_bytes(b"Play");
    }

    pub fn rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        if self.win_w == 0 || self.win_h == 0 {
            return calc_rect(fb);
        }
        (self.win_x, self.win_y, self.win_w, self.win_h)
    }

    pub fn set_pos(&mut self, x: usize, y: usize) {
        self.win_x = x;
        self.win_y = y;
    }

    pub fn set_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        self.win_x = x;
        self.win_y = y;
        self.win_w = w;
        self.win_h = h;
    }

    fn play_rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        let (x, y, _, h) = self.rect(fb);
        (x + PAD + 2, y + h.saturating_sub(54), 86, 30)
    }

    fn prev_rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        let (x, y, _, h) = self.rect(fb);
        (x + PAD + 2, y + h.saturating_sub(54), 86, 30)
    }

    fn next_rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        let (x, y, _, h) = self.rect(fb);
        (x + PAD + 98, y + h.saturating_sub(54), 86, 30)
    }

    fn redraw_image(&mut self, fb: &Framebuffer) {
        let (x, y, w, h) = self.rect(fb);
        let ui = system::ui_settings();
        let is_dark = ui.dark;
        let chrome = window::draw_window(fb, x, y, w, h, b"Photo Viewer");
        fill_vertical_gradient(
            fb,
            chrome.content_x,
            chrome.content_y,
            chrome.content_w,
            chrome.content_h,
            if is_dark { 0x00171920 } else { 0x00F8FBFF },
            if is_dark { 0x00111419 } else { 0x00EAF2FB },
        );

        let text = if is_dark { 0x00F2F5F8 } else { 0x00131A28 };
        let detail = if is_dark { 0x00B7C0CC } else { 0x004D5D72 };
        let mut writer = crate::TextWriter::new(*fb);
        display::fill_rect(
            fb,
            chrome.content_x + PAD,
            chrome.content_y + PAD,
            chrome.content_w.saturating_sub(PAD * 2),
            54,
            if is_dark { 0x00222A34 } else { 0x00E9F2FC },
        );

        writer.set_color(text);
        writer.set_pos(chrome.content_x + PAD + 10, chrome.content_y + PAD + 8);
        writer.write_bytes(b"Photo: ");
        writer.write_bytes(&self.title[..self.title_len]);

        let mut line = [0u8; 64];
        let mut p = 0usize;
        p += write_str(&mut line[p..], b"Photo ");
        p += write_u32(&mut line[p..], self.image_index.saturating_add(1) as u32);
        p += write_str(&mut line[p..], b" / ");
        p += write_u32(&mut line[p..], self.image_count.max(1) as u32);
        writer.set_color(detail);
        writer.set_pos(chrome.content_x + PAD + 10, chrome.content_y + PAD + 28);
        writer.write_bytes(&line[..p]);

        writer.set_pos(chrome.content_x + PAD + 190, chrome.content_y + PAD + 28);
        writer.write_bytes(&self.status[..self.status_len]);

        let image_x = chrome.content_x + PAD;
        let image_y = chrome.content_y + PAD + 66;
        let image_w = chrome.content_w.saturating_sub(PAD * 2);
        let image_h = chrome.content_h.saturating_sub(PAD * 2 + 120);
        display::fill_rect(
            fb,
            image_x,
            image_y,
            image_w,
            image_h,
            if is_dark { 0x000B0D11 } else { 0x00FFFFFF },
        );
        display::fill_rect(
            fb,
            image_x,
            image_y,
            image_w,
            1,
            if is_dark { 0x00303A46 } else { 0x00C8D6E6 },
        );
        display::fill_rect(
            fb,
            image_x,
            image_y + image_h.saturating_sub(1),
            image_w,
            1,
            if is_dark { 0x00303A46 } else { 0x00C8D6E6 },
        );

        if self.image_loaded {
            let ok = display::draw_bgra_image_fit_rect(
                fb,
                image::get_bgra_ptr(),
                image::get_bgra_len(),
                image_x + 4,
                image_y + 4,
                image_w.saturating_sub(8),
                image_h.saturating_sub(8),
            );
            if !ok {
                writer.set_color(detail);
                writer.set_pos(image_x + 18, image_y + 18);
                writer.write_bytes(b"Decoded image cannot be drawn");
            }
        } else {
            writer.set_color(detail);
            writer.set_pos(image_x + 18, image_y + 18);
            writer.write_bytes(b"No image loaded");
        }

        let prev = self.prev_rect(fb);
        let next = self.next_rect(fb);
        draw_button(fb, &mut writer, prev, b"< Prev", is_dark, text);
        draw_button(fb, &mut writer, next, b"Next >", is_dark, text);
    }

    fn redraw_video(&mut self, fb: &Framebuffer) {
        let (x, y, w, h) = self.rect(fb);
        let ui = system::ui_settings();
        let is_dark = ui.dark;
        let chrome = window::draw_window(fb, x, y, w, h, b"Video Player");
        fill_vertical_gradient(
            fb,
            chrome.content_x,
            chrome.content_y,
            chrome.content_w,
            chrome.content_h,
            if is_dark { 0x0013151B } else { 0x00F8FBFF },
            if is_dark { 0x000B0D12 } else { 0x00E6EEF8 },
        );

        let mut writer = crate::TextWriter::new(*fb);
        let text = if is_dark { 0x00F2F5F8 } else { 0x00131A28 };
        let detail = if is_dark { 0x00AAB4C2 } else { 0x005C6A7E };

        writer.set_color(text);
        writer.set_pos(chrome.content_x + PAD, chrome.content_y + PAD);
        writer.write_bytes(b"Video: ");
        writer.write_bytes(&self.title[..self.title_len]);

        writer.set_color(detail);
        writer.set_pos(chrome.content_x + PAD, chrome.content_y + PAD + 22);
        writer.write_bytes(&self.status[..self.status_len]);

        if self.mp4_info.valid {
            let mut info_line = [0u8; 96];
            let mut ip = 0usize;
            ip += write_str(&mut info_line[ip..], b"MP4 ");
            ip += write_str(&mut info_line[ip..], &self.mp4_info.brand[..self.mp4_info.brand_len]);
            if self.mp4_info.codec_len > 0 {
                ip += write_str(&mut info_line[ip..], b" / ");
                ip += write_str(&mut info_line[ip..], &self.mp4_info.codec[..self.mp4_info.codec_len]);
            }
            if self.mp4_info.width > 0 && self.mp4_info.height > 0 {
                ip += write_str(&mut info_line[ip..], b" ");
                ip += write_u32(&mut info_line[ip..], self.mp4_info.width);
                ip += write_str(&mut info_line[ip..], b"x");
                ip += write_u32(&mut info_line[ip..], self.mp4_info.height);
            }
            writer.set_color(detail);
            writer.set_pos(chrome.content_x + PAD, chrome.content_y + PAD + 44);
            writer.write_bytes(&info_line[..ip]);
        }

        let screen_x = chrome.content_x + PAD;
        let screen_y = chrome.content_y + PAD + 70;
        let screen_w = chrome.content_w.saturating_sub(PAD * 2);
        let screen_h = chrome.content_h.saturating_sub(PAD * 2 + 126);
        display::fill_rect(fb, screen_x, screen_y, screen_w, screen_h, 0x0006080D);

        let mut drew_frame = false;
        if self.mp4_info.jpeg_len > 0 {
            let start = self.mp4_info.jpeg_start;
            let end = start.saturating_add(self.mp4_info.jpeg_len);
            if end <= self.data_len && image::decode_jpeg(&self.data[start..end]) {
                drew_frame = display::draw_bgra_image_fit_rect(
                    fb,
                    image::get_bgra_ptr(),
                    image::get_bgra_len(),
                    screen_x + 4,
                    screen_y + 4,
                    screen_w.saturating_sub(8),
                    screen_h.saturating_sub(8),
                );
            }
        }
        if !drew_frame {
            let frame = self.video_frame as usize;
            let band_h = (screen_h / 5).max(12);
            for i in 0..5 {
                let yy = screen_y + 10 + i * band_h;
                let offset = (frame * (i + 3) * 7) % screen_w.max(1);
                let seed = if self.data_len > 0 {
                    self.data[(frame + i * 97) % self.data_len] as u32
                } else {
                    (i as u32) * 37
                };
                let color = match i {
                    0 => 0x00204080 | (seed << 8),
                    1 => 0x00208040 | seed,
                    2 => 0x00604090 | (seed << 16),
                    3 => 0x00805030 | (seed << 8),
                    _ => 0x00207090 | seed,
                } & 0x00FFFFFF;
                display::fill_rect(
                    fb,
                    screen_x + offset / 2,
                    yy,
                    screen_w.saturating_sub(offset / 2 + 14),
                    band_h / 2,
                    color,
                );
            }
            writer.set_color(detail);
            writer.set_pos(screen_x + 16, screen_y + screen_h.saturating_sub(28));
            if self.mp4_info.valid {
                writer.write_bytes(b"Compressed MP4 stream loaded; full H264/HEVC decode pending.");
            } else {
                writer.write_bytes(b"Video bytes loaded; container is not MP4.");
            }
        }
        display::fill_rect(fb, screen_x, screen_y, screen_w, 2, if is_dark { 0x003B4654 } else { 0x00B8C8D8 });
        display::fill_rect(fb, screen_x, screen_y + screen_h.saturating_sub(2), screen_w, 2, if is_dark { 0x003B4654 } else { 0x00B8C8D8 });

        let play = self.play_rect(fb);
        draw_button(fb, &mut writer, play, if self.video_playing { b"Next frame" } else { b"Play" }, is_dark, text);

        let bar_x = play.0 + play.2 + 18;
        let bar_y = play.1 + 10;
        let bar_w = chrome.content_w.saturating_sub((bar_x - chrome.content_x) + PAD);
        display::fill_rect(fb, bar_x, bar_y, bar_w, 8, if is_dark { 0x002B313A } else { 0x00D7E4F3 });
        let fill = ((self.video_frame as usize % 120) * bar_w) / 120;
        display::fill_rect(fb, bar_x, bar_y, fill, 8, ui.accent);
    }

    fn build_image_list(&mut self, parent_cluster: u32, cluster: u32, size: u32, file_name: &[u8]) {
        self.image_entries = [DirEntry::EMPTY; MAX_IMAGES];
        self.image_count = 0;
        self.image_index = 0;
        let fs_img = self.fs_img;
        if let Some(fs) = fs_img.and_then(Fat32::new) {
            let mut dir_buf = [DirEntry::EMPTY; MAX_IMAGES];
            let count = fs.list_dir(parent_cluster, &mut dir_buf);
            for i in 0..count {
                let entry = dir_buf[i];
                if entry.is_dir || !is_image_name(&entry.name[..entry.name_len]) {
                    continue;
                }
                if self.image_count >= MAX_IMAGES {
                    break;
                }
                if entry.cluster == cluster
                    || (entry.size == size && name_eq(&entry.name[..entry.name_len], file_name))
                {
                    self.image_index = self.image_count;
                }
                self.image_entries[self.image_count] = entry;
                self.image_count += 1;
            }
        }
        if self.image_count == 0 {
            let mut fallback = DirEntry::EMPTY;
            fallback.cluster = cluster;
            fallback.size = size;
            fallback.name_len = file_name.len().min(fallback.name.len());
            fallback.name[..fallback.name_len].copy_from_slice(&file_name[..fallback.name_len]);
            self.image_entries[0] = fallback;
            self.image_count = 1;
            self.image_index = 0;
        }
    }

    fn load_image_index(&mut self, index: usize) {
        self.image_loaded = false;
        if self.image_count == 0 || index >= self.image_count {
            self.set_status(b"No image in folder");
            return;
        }
        let entry = self.image_entries[index];
        self.image_index = index;
        self.set_title(&entry.name[..entry.name_len]);
        if entry.size as usize > MAX_IMAGE_FILE {
            self.set_status(b"JPG is too large for editor buffer");
            return;
        }
        let fs_img = self.fs_img;
        if let Some(fs) = fs_img.and_then(Fat32::new) {
            let buf = unsafe { &mut IMAGE_INPUT };
            let read = fs.read_file(entry.cluster, entry.size as usize, &mut buf[..entry.size as usize]);
            if read == 0 {
                self.set_status(b"Could not read JPG");
                return;
            }
            if image::decode_jpeg(&buf[..read]) {
                self.image_loaded = true;
                self.set_status(b"JPG opened. Use Prev/Next to browse.");
            } else {
                self.set_status(b"Unsupported JPG (needs baseline JPEG)");
            }
        } else {
            self.set_status(b"No filesystem image");
        }
    }

    fn prev_image(&mut self) {
        if self.image_count == 0 {
            return;
        }
        let next = if self.image_index == 0 {
            self.image_count - 1
        } else {
            self.image_index - 1
        };
        self.load_image_index(next);
    }

    fn next_image(&mut self) {
        if self.image_count == 0 {
            return;
        }
        let next = (self.image_index + 1) % self.image_count;
        self.load_image_index(next);
    }

    fn play_loaded(&mut self) {
        if self.data_len == 0 {
            pc_speaker::play_demo();
            self.set_status(b"Demo sound played");
            return;
        }
        if let Some(info) = wav_info(&self.data[..self.data_len]) {
            if play_wav_preview(&self.data[..self.data_len], info) {
                self.set_status(b"PCM WAV preview played");
            } else {
                self.set_status(b"Unsupported WAV format");
            }
        } else if play_mp3_preview(&self.data[..self.data_len]) {
            self.set_status(b"MP3 frame decoder played");
        } else {
            pc_speaker::play_bytes(&self.data[..self.data_len]);
            self.set_status(b"Raw sound preview played");
        }
    }

    fn play_video_step(&mut self) {
        self.video_playing = true;
        self.video_frame = self.video_frame.wrapping_add(12);
        pc_speaker::click();
        self.set_status(b"Rendering video preview frame.");
    }

    fn set_title(&mut self, title: &[u8]) {
        self.title.fill(0);
        self.title_len = title.len().min(MAX_TITLE);
        self.title[..self.title_len].copy_from_slice(&title[..self.title_len]);
    }

    fn set_status(&mut self, status: &[u8]) {
        self.status.fill(0);
        self.status_len = status.len().min(self.status.len());
        self.status[..self.status_len].copy_from_slice(&status[..self.status_len]);
    }
}

#[derive(Copy, Clone)]
struct WavInfo {
    data_start: usize,
    data_len: usize,
    audio_format: u16,
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    block_align: u16,
}

fn wav_info(data: &[u8]) -> Option<WavInfo> {
    if data.len() < 16 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let mut i = 12usize;
    let mut audio_format = 0u16;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut block_align = 0u16;
    let mut data_start = 0usize;
    let mut data_len = 0usize;
    while i + 8 <= data.len() {
        let id = &data[i..i + 4];
        let size = read_u32_le(&data[i + 4..i + 8]) as usize;
        let start = i + 8;
        if start + size > data.len() {
            return None;
        }
        if id == b"fmt " && size >= 16 {
            audio_format = read_u16_le(&data[start..start + 2]);
            channels = read_u16_le(&data[start + 2..start + 4]);
            sample_rate = read_u32_le(&data[start + 4..start + 8]);
            block_align = read_u16_le(&data[start + 12..start + 14]);
            bits_per_sample = read_u16_le(&data[start + 14..start + 16]);
        } else if id == b"data" {
            data_start = start;
            data_len = size.min(data.len().saturating_sub(start));
        }
        i = start + size + (size & 1);
    }
    if data_start == 0 || data_len == 0 || audio_format == 0 || channels == 0 || sample_rate == 0 {
        return None;
    }
    Some(WavInfo {
        data_start,
        data_len,
        audio_format,
        channels,
        sample_rate,
        bits_per_sample,
        block_align,
    })
}

fn play_wav_preview(data: &[u8], info: WavInfo) -> bool {
    if info.audio_format != 1 {
        return false;
    }
    let end = info.data_start.saturating_add(info.data_len).min(data.len());
    if info.data_start >= end {
        return false;
    }
    let pcm = &data[info.data_start..end];
    let frames = (info.sample_rate / 120).max(1) as usize;
    let block = info.block_align.max(1) as usize;
    let step = frames.saturating_mul(block).max(block);
    if info.bits_per_sample == 8 {
        pc_speaker::play_pcm_u8(pcm, step);
        true
    } else if info.bits_per_sample == 16 {
        pc_speaker::play_pcm_i16_le(pcm, step);
        true
    } else {
        false
    }
}

fn read_u16_le(buf: &[u8]) -> u16 {
    if buf.len() < 2 {
        0
    } else {
        (buf[0] as u16) | ((buf[1] as u16) << 8)
    }
}

fn read_u32_le(buf: &[u8]) -> u32 {
    if buf.len() < 4 {
        0
    } else {
        (buf[0] as u32) | ((buf[1] as u32) << 8) | ((buf[2] as u32) << 16) | ((buf[3] as u32) << 24)
    }
}

fn read_u32_be(buf: &[u8]) -> u32 {
    if buf.len() < 4 {
        0
    } else {
        ((buf[0] as u32) << 24) | ((buf[1] as u32) << 16) | ((buf[2] as u32) << 8) | (buf[3] as u32)
    }
}

fn read_u64_be(buf: &[u8]) -> u64 {
    if buf.len() < 8 {
        0
    } else {
        ((buf[0] as u64) << 56)
            | ((buf[1] as u64) << 48)
            | ((buf[2] as u64) << 40)
            | ((buf[3] as u64) << 32)
            | ((buf[4] as u64) << 24)
            | ((buf[5] as u64) << 16)
            | ((buf[6] as u64) << 8)
            | (buf[7] as u64)
    }
}

fn parse_mp4_info(data: &[u8]) -> Mp4Info {
    let mut info = Mp4Info::EMPTY;
    parse_mp4_boxes(data, 0, 0, &mut info);
    info
}

fn parse_mp4_boxes(data: &[u8], base_offset: usize, depth: usize, info: &mut Mp4Info) {
    if depth > 6 {
        return;
    }
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size32 = read_u32_be(&data[pos..pos + 4]) as usize;
        let typ = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        let mut header = 8usize;
        let box_size = if size32 == 1 {
            if pos + 16 > data.len() {
                break;
            }
            header = 16;
            read_u64_be(&data[pos + 8..pos + 16]) as usize
        } else if size32 == 0 {
            data.len().saturating_sub(pos)
        } else {
            size32
        };
        if box_size < header {
            break;
        }
        let full_end = pos.saturating_add(box_size);
        let payload_start = pos + header;
        let payload_end = full_end.min(data.len());
        if payload_start > payload_end {
            break;
        }
        let payload = &data[payload_start..payload_end];

        if typ == *b"ftyp" {
            info.valid = true;
            info.brand_len = payload.len().min(4).min(info.brand.len());
            info.brand[..info.brand_len].copy_from_slice(&payload[..info.brand_len]);
        } else if typ == *b"mvhd" {
            parse_mp4_mvhd(payload, info);
        } else if typ == *b"tkhd" {
            parse_mp4_tkhd(payload, info);
        } else if typ == *b"stsd" {
            parse_mp4_stsd(payload, info);
        } else if typ == *b"mdat" {
            info.mdat_offset = base_offset + payload_start;
            info.mdat_len = payload.len();
            if let Some((start, len)) = find_jpeg_frame(payload) {
                info.jpeg_start = base_offset + payload_start + start;
                info.jpeg_len = len;
            }
        } else if is_mp4_container(typ) {
            parse_mp4_boxes(payload, base_offset + payload_start, depth + 1, info);
        }

        if full_end > data.len() || box_size == 0 {
            break;
        }
        pos = full_end;
    }
}

fn is_mp4_container(typ: [u8; 4]) -> bool {
    typ == *b"moov"
        || typ == *b"trak"
        || typ == *b"mdia"
        || typ == *b"minf"
        || typ == *b"stbl"
        || typ == *b"edts"
}

fn parse_mp4_mvhd(payload: &[u8], info: &mut Mp4Info) {
    if payload.len() < 20 {
        return;
    }
    let version = payload[0];
    if version == 1 {
        if payload.len() >= 32 {
            info.timescale = read_u32_be(&payload[20..24]);
            info.duration = read_u64_be(&payload[24..32]).min(u32::MAX as u64) as u32;
        }
    } else {
        info.timescale = read_u32_be(&payload[12..16]);
        info.duration = read_u32_be(&payload[16..20]);
    }
}

fn parse_mp4_tkhd(payload: &[u8], info: &mut Mp4Info) {
    if payload.len() < 84 {
        return;
    }
    let version = payload[0];
    let wh = if version == 1 { 88 } else { 76 };
    if wh + 8 <= payload.len() {
        let width = read_u32_be(&payload[wh..wh + 4]) >> 16;
        let height = read_u32_be(&payload[wh + 4..wh + 8]) >> 16;
        if width > 0 && height > 0 {
            info.width = width;
            info.height = height;
            info.tracks = info.tracks.saturating_add(1);
        }
    }
}

fn parse_mp4_stsd(payload: &[u8], info: &mut Mp4Info) {
    if payload.len() < 16 {
        return;
    }
    let entry_count = read_u32_be(&payload[4..8]);
    if entry_count == 0 {
        return;
    }
    info.codec_len = 4;
    info.codec[..4].copy_from_slice(&payload[12..16]);
}

fn find_jpeg_frame(data: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0usize;
    while i + 1 < data.len() {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {
            let start = i;
            i += 2;
            while i + 1 < data.len() {
                if data[i] == 0xFF && data[i + 1] == 0xD9 {
                    return Some((start, i + 2 - start));
                }
                i += 1;
            }
            return None;
        }
        i += 1;
    }
    None
}

fn mp3_first_frame(data: &[u8]) -> Option<usize> {
    let mut i = skip_id3v2(data);
    while i + 4 <= data.len() {
        if mp3_header_info(&data[i..i + 4]).is_some() {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn play_mp3_preview(data: &[u8]) -> bool {
    let mut i = skip_id3v2(data);
    let mut tones = 0usize;
    while i + 4 <= data.len() && tones < 180 {
        if let Some(info) = mp3_header_info(&data[i..i + 4]) {
            let frame_len = info.frame_len.max(4);
            let frame_end = i.saturating_add(frame_len).min(data.len());
            if frame_end > i + 4 {
                tones += play_mp3_frame_energy(&data[i + 4..frame_end], info, 180usize.saturating_sub(tones));
            }
            i = frame_end;
        } else {
            i += 1;
        }
    }
    pc_speaker::silence();
    tones > 0
}

fn play_mp3_frame_energy(frame: &[u8], info: Mp3Info, budget: usize) -> usize {
    if frame.is_empty() || budget == 0 {
        return 0;
    }
    let chunks = budget.min(4).min(frame.len().max(1));
    let chunk_len = (frame.len() / chunks).max(1);
    let mut played = 0usize;
    let mut offset = 0usize;
    while offset < frame.len() && played < chunks {
        let end = offset.saturating_add(chunk_len).min(frame.len());
        let mut sum = 0u32;
        let mut variance = 0u32;
        let mut prev = frame[offset] as i32;
        let mut count = 0u32;
        let mut j = offset;
        while j < end {
            let sample = frame[j] as i32;
            sum = sum.wrapping_add(sample as u32);
            let delta = sample - prev;
            variance = variance.wrapping_add(if delta < 0 { -delta } else { delta } as u32);
            prev = sample;
            count += 1;
            j += 1;
        }
        let avg = if count == 0 { 0 } else { sum / count };
        let motion = if count == 0 { 0 } else { variance / count };
        let stereo_shift = (info.channel_mode as u32) * 37;
        let layer_shift = (info.layer as u32) * 53;
        let freq = 170
            + ((avg * 3)
                + (motion * 11)
                + (info.bitrate_kbps as u32)
                + (info.sample_rate as u32 / 120)
                + stereo_shift
                + layer_shift)
                % 1900;
        pc_speaker::play_tone(freq.min(2400), 10_000);
        offset = end;
        played += 1;
    }
    played
}

#[derive(Copy, Clone)]
struct Mp3Info {
    bitrate_kbps: u16,
    sample_rate: u16,
    layer: u8,
    channel_mode: u8,
    frame_len: usize,
}

fn mp3_header_info(h: &[u8]) -> Option<Mp3Info> {
    if h.len() < 4 || h[0] != 0xFF || (h[1] & 0xE0) != 0xE0 {
        return None;
    }
    let version_bits = (h[1] >> 3) & 0x03;
    let layer_bits = (h[1] >> 1) & 0x03;
    let bitrate_idx = (h[2] >> 4) & 0x0F;
    let rate_idx = (h[2] >> 2) & 0x03;
    if version_bits == 0x01 || layer_bits == 0 || bitrate_idx == 0 || bitrate_idx == 0x0F || rate_idx == 0x03 {
        return None;
    }

    let layer = 4 - layer_bits;
    let sample_rate = mp3_sample_rate(version_bits, rate_idx)?;
    let bitrate = mp3_bitrate(version_bits, layer_bits, bitrate_idx)?;
    let padding = ((h[2] >> 1) & 0x01) as usize;
    let frame_len = if layer_bits == 0x03 {
        ((12 * bitrate as usize * 1000 / sample_rate as usize) + padding) * 4
    } else if version_bits == 0x03 {
        (144 * bitrate as usize * 1000 / sample_rate as usize) + padding
    } else {
        (72 * bitrate as usize * 1000 / sample_rate as usize) + padding
    };
    if frame_len < 4 {
        return None;
    }
    Some(Mp3Info {
        bitrate_kbps: bitrate,
        sample_rate,
        layer,
        channel_mode: (h[3] >> 6) & 0x03,
        frame_len,
    })
}

fn mp3_sample_rate(version_bits: u8, idx: u8) -> Option<u16> {
    let base = match idx {
        0 => 44100,
        1 => 48000,
        2 => 32000,
        _ => return None,
    };
    let rate = match version_bits {
        0x03 => base,
        0x02 => base / 2,
        0x00 => base / 4,
        _ => return None,
    };
    Some(rate as u16)
}

fn mp3_bitrate(version_bits: u8, layer_bits: u8, idx: u8) -> Option<u16> {
    let i = idx as usize;
    let table = if version_bits == 0x03 {
        match layer_bits {
            0x03 => [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0],
            0x02 => [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0],
            0x01 => [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0],
            _ => return None,
        }
    } else {
        match layer_bits {
            0x03 => [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0],
            0x02 | 0x01 => [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0],
            _ => return None,
        }
    };
    let bitrate = table[i];
    if bitrate == 0 { None } else { Some(bitrate as u16) }
}

fn skip_id3v2(data: &[u8]) -> usize {
    if data.len() < 10 || &data[0..3] != b"ID3" {
        return 0;
    }
    let size = ((data[6] as usize & 0x7F) << 21)
        | ((data[7] as usize & 0x7F) << 14)
        | ((data[8] as usize & 0x7F) << 7)
        | (data[9] as usize & 0x7F);
    (10 + size).min(data.len())
}

fn draw_button(
    fb: &Framebuffer,
    writer: &mut crate::TextWriter,
    rect: (usize, usize, usize, usize),
    label: &[u8],
    is_dark: bool,
    text_color: u32,
) {
    fill_vertical_gradient(
        fb,
        rect.0,
        rect.1,
        rect.2,
        rect.3,
        if is_dark { 0x003A4E62 } else { 0x00EEF5FD },
        if is_dark { 0x002B3B4C } else { 0x00D4E2F0 },
    );
    writer.set_color(text_color);
    let text_w = label.len() * 8;
    writer.set_pos(rect.0 + rect.2.saturating_sub(text_w) / 2, rect.1 + 8);
    writer.write_bytes(label);
}

fn is_image_name(name: &[u8]) -> bool {
    if name.len() < 4 {
        return false;
    }
    let mut dot = usize::MAX;
    for i in 0..name.len() {
        if name[i] == b'.' {
            dot = i;
        }
    }
    if dot == usize::MAX || dot + 1 >= name.len() {
        return false;
    }
    let ext = &name[dot + 1..];
    name_eq(ext, b"JPG") || name_eq(ext, b"JPEG")
}

fn is_video_name(name: &[u8]) -> bool {
    if name.len() < 4 {
        return false;
    }
    let mut dot = usize::MAX;
    for i in 0..name.len() {
        if name[i] == b'.' {
            dot = i;
        }
    }
    if dot == usize::MAX || dot + 1 >= name.len() {
        return false;
    }
    let ext = &name[dot + 1..];
    name_eq(ext, b"MP4")
        || name_eq(ext, b"AVI")
        || name_eq(ext, b"MOV")
        || name_eq(ext, b"WEBM")
        || name_eq(ext, b"MPG")
        || name_eq(ext, b"MPEG")
}

fn name_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        if ascii_lower(a[i]) != ascii_lower(b[i]) {
            return false;
        }
    }
    true
}

fn ascii_lower(b: u8) -> u8 {
    if b'A' <= b && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

fn calc_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let w = (fb.width * 3 / 4).min(760).max(420);
    let h = (fb.height * 3 / 4).min(560).max(300);
    ((fb.width.saturating_sub(w)) / 2, (fb.height.saturating_sub(h)) / 2, w, h)
}

fn hit(px: usize, py: usize, x: usize, y: usize, w: usize, h: usize) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

fn write_str(buf: &mut [u8], s: &[u8]) -> usize {
    let mut n = 0usize;
    while n < s.len() && n < buf.len() {
        buf[n] = s[n];
        n += 1;
    }
    n
}

fn write_u32(buf: &mut [u8], mut val: u32) -> usize {
    if buf.is_empty() {
        return 0;
    }
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut len = 0usize;
    while val > 0 && len < tmp.len() {
        tmp[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    let mut out = 0usize;
    while len > 0 && out < buf.len() {
        len -= 1;
        buf[out] = tmp[len];
        out += 1;
    }
    out
}

fn fill_vertical_gradient(fb: &Framebuffer, x: usize, y: usize, w: usize, h: usize, top: u32, bottom: u32) {
    if w == 0 || h == 0 {
        return;
    }
    if h == 1 {
        display::fill_rect(fb, x, y, w, 1, top);
        return;
    }
    let den = (h - 1) as u32;
    for row in 0..h {
        let c = lerp_rgb(top, bottom, row as u32, den);
        display::fill_rect(fb, x, y + row, w, 1, c);
    }
}

fn lerp_rgb(a: u32, b: u32, num: u32, den: u32) -> u32 {
    if den == 0 {
        return a;
    }
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
