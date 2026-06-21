use crate::display::{self, Framebuffer};
use crate::drivers::storage::{ata_pio, exfat, ntfs};
use crate::drivers::usb::{self, UsbControllerKind};
use crate::fat32::{DirEntry, Fat32, MAX_NAME};
use crate::system;
use crate::window;

const LINE_HEIGHT: usize = 18;
const PAD: usize = 12;
const TOOLBAR_H: usize = 30;
const SCROLL_W: usize = 14;
const SIDEBAR_GAP: usize = 14;
const MAX_ENTRIES: usize = 512;
const MAX_PATH: usize = 12;
const MAX_RECYCLE: usize = 96;
const ENTRY_BACK: u8 = 0;
const ENTRY_DIR: u8 = 1;
const ENTRY_FILE: u8 = 2;

const SRC_HOUSE: u8 = 0;
const SRC_PARTITION: u8 = 1;
const SRC_NTFS: u8 = 2;
const SRC_DEVICE: u8 = 3;
const SRC_EXFAT: u8 = 4;

const FULL_PARTITIONS: u8 = 0;
const FULL_NTFS: u8 = 1;
const FULL_EXFAT: u8 = 2;

const REC_NONE: u8 = 0;
const REC_BIN: u8 = 1;
const REC_PURGED: u8 = 2;

#[derive(Copy, Clone, PartialEq)]
enum ExplorerView {
    Root,
    Bin,
}

#[derive(Copy, Clone, PartialEq)]
pub enum ExplorerAction {
    None,
    OpenTextFile {
        name: [u8; MAX_NAME],
        name_len: usize,
        cluster: u32,
        size: u32,
    },
    OpenMediaFile {
        name: [u8; MAX_NAME],
        name_len: usize,
        cluster: u32,
        size: u32,
    },
    OpenImageFile {
        name: [u8; MAX_NAME],
        name_len: usize,
        parent_cluster: u32,
        cluster: u32,
        size: u32,
    },
}

#[derive(Copy, Clone)]
struct FileEntry {
    name: [u8; MAX_NAME],
    name_len: usize,
    kind: u8,
    cluster: u32,
    size: u32,
    parent_cluster: u32,
    recycle_idx: usize,
    source: u8,
    partition_idx: usize,
    ntfs_ref: u64,
}

impl FileEntry {
    const EMPTY: FileEntry = FileEntry {
        name: [0u8; MAX_NAME],
        name_len: 0,
        kind: ENTRY_FILE,
        cluster: 0,
        size: 0,
        parent_cluster: 0,
        recycle_idx: usize::MAX,
        source: SRC_HOUSE,
        partition_idx: usize::MAX,
        ntfs_ref: 0,
    };

    fn back() -> Self {
        let mut e = Self::EMPTY;
        e.kind = ENTRY_BACK;
        e.name[0] = b'<';
        e.name[1] = b'-';
        e.name[2] = b' ';
        e.name[3] = b'B';
        e.name[4] = b'a';
        e.name[5] = b'c';
        e.name[6] = b'k';
        e.name_len = 7;
        e
    }
}

#[derive(Copy, Clone)]
struct PathItem {
    name: [u8; MAX_NAME],
    len: usize,
}

impl PathItem {
    const EMPTY: PathItem = PathItem {
        name: [0u8; MAX_NAME],
        len: 0,
    };
}

#[derive(Copy, Clone)]
struct RecycleEntry {
    state: u8,
    name: [u8; MAX_NAME],
    name_len: usize,
    kind: u8,
    cluster: u32,
    size: u32,
    parent_cluster: u32,
}

impl RecycleEntry {
    const EMPTY: RecycleEntry = RecycleEntry {
        state: REC_NONE,
        name: [0u8; MAX_NAME],
        name_len: 0,
        kind: ENTRY_FILE,
        cluster: 0,
        size: 0,
        parent_cluster: 0,
    };
}

struct Layout {
    content_x: usize,
    content_y: usize,
    content_w: usize,
    content_h: usize,
    toolbar_y: usize,
    body_y: usize,
    body_h: usize,
    sidebar_w: usize,
    list_x: usize,
    list_header_y: usize,
    list_start_y: usize,
    list_w: usize,
    list_h: usize,
    max_lines: usize,
    back_btn: (usize, usize, usize, usize),
    refresh_btn: (usize, usize, usize, usize),
    copy_btn: (usize, usize, usize, usize),
    paste_btn: (usize, usize, usize, usize),
    action_btn: (usize, usize, usize, usize),
    purge_btn: (usize, usize, usize, usize),
}

pub struct Explorer {
    visible: bool,
    win_x: usize,
    win_y: usize,
    win_w: usize,
    win_h: usize,
    view: ExplorerView,
    fs_img: Option<crate::ModuleRange>,
    current_cluster: u32,
    path: [PathItem; MAX_PATH],
    cluster_stack: [u32; MAX_PATH],
    depth: usize,
    entries: [FileEntry; MAX_ENTRIES],
    entry_count: usize,
    visible_count: usize,
    scroll_offset: usize,
    entries_dirty: bool,
    selected_entry: Option<usize>,
    recycle: [RecycleEntry; MAX_RECYCLE],
    action: ExplorerAction,
    full_disk: bool,
    full_mode: u8,
    full_part_idx: usize,
    ntfs_current_ref: u64,
    ntfs_stack: [u64; MAX_PATH],
    clipboard_valid: bool,
    clipboard_entry: FileEntry,
    copy_status: [u8; 96],
    copy_status_len: usize,
}

impl Explorer {
    pub fn new(_fb: Framebuffer, fs_img: Option<crate::ModuleRange>) -> Self {
        Self {
            visible: false,
            win_x: 0,
            win_y: 0,
            win_w: 0,
            win_h: 0,
            view: ExplorerView::Root,
            fs_img,
            current_cluster: 2,
            path: [PathItem::EMPTY; MAX_PATH],
            cluster_stack: [0u32; MAX_PATH],
            depth: 0,
            entries: [FileEntry::EMPTY; MAX_ENTRIES],
            entry_count: 0,
            visible_count: 0,
            scroll_offset: 0,
            entries_dirty: true,
            selected_entry: None,
            recycle: [RecycleEntry::EMPTY; MAX_RECYCLE],
            action: ExplorerAction::None,
            full_disk: false,
            full_mode: FULL_PARTITIONS,
            full_part_idx: usize::MAX,
            ntfs_current_ref: ntfs::ROOT_FILE_REF,
            ntfs_stack: [0; MAX_PATH],
            clipboard_valid: false,
            clipboard_entry: FileEntry::EMPTY,
            copy_status: [0; 96],
            copy_status_len: 0,
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
        self.view = ExplorerView::Root;
        self.reset_path();
        self.entries_dirty = true;
        self.selected_entry = None;
        self.action = ExplorerAction::None;
    }

    pub fn show_bin(&mut self, fb: &Framebuffer) {
        self.visible = true;
        if self.win_w == 0 || self.win_h == 0 {
            let (x, y, w, h) = calc_rect(fb);
            self.win_x = x;
            self.win_y = y;
            self.win_w = w;
            self.win_h = h;
        }
        self.view = ExplorerView::Bin;
        self.entries_dirty = true;
        self.selected_entry = None;
        self.action = ExplorerAction::None;
    }

    pub fn hide(&mut self, _fb: &Framebuffer) {
        self.visible = false;
    }

    pub fn take_action(&mut self) -> ExplorerAction {
        core::mem::replace(&mut self.action, ExplorerAction::None)
    }

    pub fn handle_click(&mut self, fb: &Framebuffer, x: usize, y: usize) -> bool {
        if !self.visible {
            return false;
        }

        let layout = self.layout(fb, self.content_rect(fb));

        let sidebar_x = layout.content_x + 1;
        let sidebar_y = layout.body_y + PAD;
        let sidebar_h = layout.body_h.saturating_sub(PAD * 2);
        if hit(x, y, sidebar_x, sidebar_y, layout.sidebar_w, sidebar_h) {
            self.handle_sidebar_click(fb, &layout, x, y);
            return true;
        }

        if hit(x, y, layout.back_btn.0, layout.back_btn.1, layout.back_btn.2, layout.back_btn.3) {
            if self.view == ExplorerView::Root {
                if self.full_disk {
                    self.full_disk_back();
                    self.redraw(fb);
                } else if self.depth > 0 {
                    self.depth -= 1;
                    self.current_cluster = self.cluster_stack[self.depth];
                    self.entries_dirty = true;
                    self.selected_entry = None;
                    self.redraw(fb);
                }
            } else {
                self.view = ExplorerView::Root;
                self.entries_dirty = true;
                self.selected_entry = None;
                self.redraw(fb);
            }
            return true;
        }

        if hit(
            x,
            y,
            layout.refresh_btn.0,
            layout.refresh_btn.1,
            layout.refresh_btn.2,
            layout.refresh_btn.3,
        ) {
            self.rescan_devices();
            self.entries_dirty = true;
            self.selected_entry = None;
            self.redraw(fb);
            return true;
        }

        if hit(
            x,
            y,
            layout.copy_btn.0,
            layout.copy_btn.1,
            layout.copy_btn.2,
            layout.copy_btn.3,
        ) {
            self.copy_selected();
            self.redraw(fb);
            return true;
        }

        if hit(
            x,
            y,
            layout.paste_btn.0,
            layout.paste_btn.1,
            layout.paste_btn.2,
            layout.paste_btn.3,
        ) {
            self.paste_clipboard(fb);
            return true;
        }

        if hit(
            x,
            y,
            layout.action_btn.0,
            layout.action_btn.1,
            layout.action_btn.2,
            layout.action_btn.3,
        ) {
            if self.view == ExplorerView::Root {
                if let Some(idx) = self.selected_entry {
                    if idx < self.entry_count {
                        let e = self.entries[idx];
                        if e.kind != ENTRY_BACK {
                            if self.recycle_move(e) {
                                self.entries_dirty = true;
                                self.selected_entry = None;
                                self.redraw(fb);
                            }
                        }
                    }
                }
            } else if let Some(idx) = self.selected_entry {
                if idx < self.entry_count {
                    let e = self.entries[idx];
                    if e.recycle_idx < MAX_RECYCLE {
                        self.recycle[e.recycle_idx].state = REC_NONE;
                        self.entries_dirty = true;
                        self.selected_entry = None;
                        self.redraw(fb);
                    }
                }
            }
            return true;
        }

        if self.view == ExplorerView::Root
            && hit(
                x,
                y,
                layout.purge_btn.0,
                layout.purge_btn.1,
                layout.purge_btn.2,
                layout.purge_btn.3,
            )
        {
            self.full_disk = !self.full_disk;
            if self.full_disk {
                self.rescan_devices();
            }
            self.reset_path();
            self.selected_entry = None;
            self.redraw(fb);
            return true;
        }

        if self.view == ExplorerView::Bin
            && hit(
                x,
                y,
                layout.purge_btn.0,
                layout.purge_btn.1,
                layout.purge_btn.2,
                layout.purge_btn.3,
            )
        {
            if let Some(idx) = self.selected_entry {
                if idx < self.entry_count {
                    let e = self.entries[idx];
                    if e.recycle_idx < MAX_RECYCLE {
                        let deleted = if let Some(range) = self.fs_img {
                            unsafe {
                                crate::fat32::mark_deleted(
                                    range,
                                    e.parent_cluster,
                                    &e.name[..e.name_len],
                                )
                            }
                        } else {
                            false
                        };
                        if deleted {
                            self.recycle[e.recycle_idx].state = REC_PURGED;
                        } else {
                            self.recycle[e.recycle_idx].state = REC_NONE;
                        }
                        self.entries_dirty = true;
                        self.selected_entry = None;
                        self.redraw(fb);
                    }
                }
            }
            return true;
        }

        let scroll_x = layout.list_x + layout.list_w + 4;
        let scroll_y = layout.list_start_y;
        let scroll_h = layout.list_h;
        if x >= scroll_x && x < scroll_x + SCROLL_W && y >= scroll_y && y < scroll_y + scroll_h {
            let max_scroll = self.entry_count.saturating_sub(layout.max_lines);
            if max_scroll > 0 {
                let track_h = scroll_h.saturating_sub(2 * SCROLL_W);
                if track_h > 0 {
                    let ratio = (y.saturating_sub(scroll_y + SCROLL_W)) as f32 / track_h as f32;
                    self.scroll_offset = (ratio * max_scroll as f32) as usize;
                    if self.scroll_offset > max_scroll {
                        self.scroll_offset = max_scroll;
                    }
                    self.selected_entry = None;
                    self.redraw(fb);
                }
            }
            return true;
        }

        if x >= layout.list_x
            && x < layout.list_x + layout.list_w
            && y >= layout.list_start_y
            && y < layout.list_start_y + layout.list_h
        {
            let row = (y - layout.list_start_y) / LINE_HEIGHT;
            if row < self.visible_count {
                let idx = self.scroll_offset + row;
                if idx < self.entry_count {
                    let was_selected = self.selected_entry == Some(idx);
                    let entry = self.entries[idx];
                    self.selected_entry = Some(idx);
                    self.redraw(fb);

                    if self.view == ExplorerView::Root {
                        if self.full_disk {
                            self.handle_full_disk_entry(fb, entry);
                        } else if entry.kind == ENTRY_BACK {
                            if self.depth > 0 {
                                self.depth -= 1;
                                self.current_cluster = self.cluster_stack[self.depth];
                                self.entries_dirty = true;
                                self.selected_entry = None;
                                self.redraw(fb);
                            }
                        } else if entry.kind == ENTRY_DIR && entry.cluster >= 2 {
                            if self.depth < MAX_PATH {
                                self.cluster_stack[self.depth] = self.current_cluster;
                                self.path[self.depth] = PathItem {
                                    name: entry.name,
                                    len: entry.name_len,
                                };
                                self.depth += 1;
                                self.current_cluster = entry.cluster;
                                self.entries_dirty = true;
                                self.selected_entry = None;
                                self.redraw(fb);
                            }
                        } else if entry.kind == ENTRY_FILE && was_selected {
                            if is_image_file(entry.name, entry.name_len) {
                                self.action = ExplorerAction::OpenImageFile {
                                    name: entry.name,
                                    name_len: entry.name_len,
                                    parent_cluster: entry.parent_cluster,
                                    cluster: entry.cluster,
                                    size: entry.size,
                                };
                            } else if is_text_file(entry.name, entry.name_len) {
                                self.action = ExplorerAction::OpenTextFile {
                                    name: entry.name,
                                    name_len: entry.name_len,
                                    cluster: entry.cluster,
                                    size: entry.size,
                                };
                            } else if is_media_file(entry.name, entry.name_len) {
                                self.action = ExplorerAction::OpenMediaFile {
                                    name: entry.name,
                                    name_len: entry.name_len,
                                    cluster: entry.cluster,
                                    size: entry.size,
                                };
                            }
                        }
                    }
                    return true;
                }
            }
        }

        false
    }

    pub fn redraw(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }

        let (x, y, w, h) = self.rect(fb);
        let ui = system::ui_settings();
        let accent = ui.accent;
        let is_dark = ui.dark;

        let title: &[u8] = if self.view == ExplorerView::Bin {
            b"Recycle Bin"
        } else {
            b"File Explorer"
        };
        let chrome = window::draw_window(fb, x, y, w, h, title);

        fill_vertical_gradient(
            fb,
            chrome.content_x,
            chrome.content_y,
            chrome.content_w,
            chrome.content_h,
            if is_dark { 0x001E1E1E } else { 0x00FFFFFF },
            if is_dark { 0x00181818 } else { 0x00F7FAFF },
        );

        let layout = self.layout(
            fb,
            (
                chrome.content_x,
                chrome.content_y,
                chrome.content_w,
                chrome.content_h,
            ),
        );

        fill_vertical_gradient(
            fb,
            layout.content_x,
            layout.toolbar_y,
            layout.content_w,
            TOOLBAR_H,
            if is_dark { 0x00313131 } else { 0x00F8FBFF },
            if is_dark { 0x002A2A2A } else { 0x00EDF3FC },
        );
        display::fill_rect(
            fb,
            layout.content_x,
            layout.toolbar_y + TOOLBAR_H.saturating_sub(1),
            layout.content_w,
            1,
            if is_dark { 0x00444444 } else { 0x00D6E2F0 },
        );

        let mut writer = crate::TextWriter::new(*fb);
        let text_color = if is_dark { 0x00F3F5F8 } else { 0x00121B29 };
        let detail = if is_dark { 0x00B7C0CC } else { 0x004D5D72 };

        draw_button(
            fb,
            &mut writer,
            layout.back_btn,
            b"Back",
            is_dark,
            text_color,
        );
        draw_button(
            fb,
            &mut writer,
            layout.refresh_btn,
            b"Refresh",
            is_dark,
            text_color,
        );
        draw_button(
            fb,
            &mut writer,
            layout.copy_btn,
            b"Copy",
            is_dark,
            text_color,
        );
        draw_button(
            fb,
            &mut writer,
            layout.paste_btn,
            b"Paste",
            is_dark,
            text_color,
        );

        let action_label: &[u8] = if self.view == ExplorerView::Bin {
            b"Restore"
        } else {
            b"Delete"
        };
        draw_button(
            fb,
            &mut writer,
            layout.action_btn,
            action_label,
            is_dark,
            text_color,
        );

        if self.view == ExplorerView::Bin {
            draw_button(
                fb,
                &mut writer,
                layout.purge_btn,
                b"Delete forever",
                is_dark,
                text_color,
            );
        } else {
            let mode_label: &[u8] = if self.full_disk { b"Standard" } else { b"Full disk" };
            draw_button(
                fb,
                &mut writer,
                layout.purge_btn,
                mode_label,
                is_dark,
                text_color,
            );
        }

        writer.set_color(detail);
        writer.set_pos(layout.purge_btn.0 + layout.purge_btn.2 + PAD, layout.toolbar_y + 8);
        if self.view == ExplorerView::Bin {
            writer.write_bytes(b"Items in recycle bin");
        } else {
            if self.full_disk {
                writer.write_bytes(b"Full disk\\");
            } else {
                writer.write_bytes(b"HouseOS (H:)\\");
            }
            for i in 0..self.depth {
                let item = self.path[i];
                if item.len > 0 {
                    writer.write_bytes(&item.name[..item.len]);
                    if i + 1 < self.depth {
                        writer.write_bytes(b"\\");
                    }
                }
            }
        }

        fill_vertical_gradient(
            fb,
            layout.content_x,
            layout.body_y,
            layout.sidebar_w,
            layout.body_h,
            if is_dark { 0x00252525 } else { 0x00F9FBFF },
            if is_dark { 0x00212121 } else { 0x00EEF4FC },
        );

        writer.set_color(detail);
        self.draw_sidebar(fb, &mut writer, &layout, accent, text_color, detail);

        let type_x = layout.list_x + 6;
        let name_x = layout.list_x + 76;
        let size_x = layout.list_x + layout.list_w.saturating_sub(72);

        writer.set_color(detail);
        writer.set_pos(type_x, layout.list_header_y);
        writer.write_bytes(b"Type");
        writer.set_pos(name_x, layout.list_header_y);
        writer.write_bytes(b"Name");
        writer.set_pos(size_x, layout.list_header_y);
        writer.write_bytes(b"Size MiB");

        if self.entries_dirty {
            self.rebuild_entries(layout.max_lines);
            self.entries_dirty = false;
            self.scroll_offset = 0;
            self.selected_entry = None;
        } else {
            let max_scroll = self.entry_count.saturating_sub(layout.max_lines);
            if self.scroll_offset > max_scroll {
                self.scroll_offset = max_scroll;
            }
            self.visible_count = (self.entry_count.saturating_sub(self.scroll_offset)).min(layout.max_lines);
        }

        for i in 0..self.visible_count {
            let entry_idx = self.scroll_offset + i;
            if entry_idx >= self.entry_count {
                break;
            }
            let entry = self.entries[entry_idx];
            let row_y = layout.list_start_y + i * LINE_HEIGHT;

            let row_bg = if Some(entry_idx) == self.selected_entry {
                if is_dark { 0x40284A7A } else { 0x40D7E9FF }
            } else if (i & 1) == 1 {
                if is_dark { 0x002E2E2E } else { 0x00F6FAFF }
            } else {
                0
            };
            if row_bg != 0 {
                display::fill_rect(fb, layout.list_x, row_y, layout.list_w, LINE_HEIGHT, row_bg);
            }

            writer.set_color(if entry.kind == ENTRY_DIR { accent } else { detail });
            writer.set_pos(type_x, row_y + 3);
            writer.write_bytes(entry_type_label(entry));

            writer.set_color(if entry.kind == ENTRY_DIR { accent } else { text_color });
            writer.set_pos(name_x, row_y + 3);
            let max_name_chars = (size_x.saturating_sub(name_x + 8) / 8).max(8);
            if entry.kind == ENTRY_BACK {
                writer.write_bytes(b"Back");
            } else if entry.kind == ENTRY_DIR {
                draw_entry_name(&mut writer, &entry.name[..entry.name_len], max_name_chars, Some(b'/'));
            } else {
                draw_entry_name(&mut writer, &entry.name[..entry.name_len], max_name_chars, None);
            }

            if entry.kind != ENTRY_DIR && entry.kind != ENTRY_BACK && entry.source != SRC_DEVICE && entry.size > 0 {
                let mut buf = [0u8; 12];
                let len = write_u32(&mut buf, entry.size);
                if len > 0 {
                    writer.set_color(detail);
                    let sx = layout.list_x + layout.list_w.saturating_sub(len * 8 + 8);
                    writer.set_pos(sx, row_y + 3);
                    writer.write_bytes(&buf[..len]);
                }
            }
        }

        let max_scroll = self.entry_count.saturating_sub(layout.max_lines);
        if max_scroll > 0 {
            let scroll_x = layout.list_x + layout.list_w + 4;
            let scroll_y = layout.list_start_y;
            let scroll_h = layout.list_h;
            let thumb_h = ((layout.max_lines as f32 / self.entry_count as f32) * scroll_h as f32) as usize;
            let thumb_h = thumb_h.max(16).min(scroll_h.max(16));
            let thumb_y = scroll_y
                + ((self.scroll_offset as f32 / max_scroll as f32)
                    * (scroll_h.saturating_sub(thumb_h)) as f32) as usize;
            draw_pretty_scrollbar(fb, scroll_x, scroll_y, SCROLL_W, scroll_h, thumb_y, thumb_h, is_dark, accent);
        }

        if self.entry_count == 0 {
            writer.set_color(detail);
            writer.set_pos(layout.list_x, layout.list_start_y + 4);
            if self.view == ExplorerView::Bin {
                writer.write_bytes(b"Recycle Bin is empty.");
            } else {
                writer.write_bytes(b"(empty folder)");
            }
        }

        if self.copy_status_len > 0 {
            draw_copy_status_panel(
                fb,
                &mut writer,
                &layout,
                &self.copy_status[..self.copy_status_len],
                is_dark,
                accent,
                text_color,
                detail,
            );
        }
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

    pub fn scroll_up(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }
        if self.entries_dirty {
            self.redraw(fb);
        }
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
            self.selected_entry = None;
            self.redraw(fb);
        }
    }

    pub fn scroll_down(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }
        if self.entries_dirty {
            self.redraw(fb);
        }
        let layout = self.layout(fb, self.content_rect(fb));
        let max_scroll = self.entry_count.saturating_sub(layout.max_lines);
        if self.scroll_offset < max_scroll {
            self.scroll_offset += 1;
            self.selected_entry = None;
            self.redraw(fb);
        }
    }

    fn rescan_devices(&mut self) {
        ata_pio::rescan();
        usb::rescan();
        self.entries_dirty = true;
        self.selected_entry = None;
        self.scroll_offset = 0;
    }

    fn copy_selected(&mut self) {
        let idx = match self.selected_entry {
            Some(v) => v,
            None => {
                self.set_copy_status(b"Copy: select a file first");
                return;
            }
        };
        if idx >= self.entry_count {
            self.set_copy_status(b"Copy: invalid selection");
            return;
        }
        let entry = self.entries[idx];
        if self.view != ExplorerView::Root || self.full_disk || entry.source != SRC_HOUSE || entry.kind != ENTRY_FILE {
            self.set_copy_status(b"Copy: only HouseOS files are writable now");
            return;
        }
        self.clipboard_entry = entry;
        self.clipboard_valid = true;
        let mut msg = [0u8; 96];
        let mut len = 0usize;
        len += copy_bytes(&mut msg[len..], b"Copy ready: ");
        len += copy_bytes(&mut msg[len..], &entry.name[..entry.name_len]);
        self.set_copy_status(&msg[..len]);
    }

    fn paste_clipboard(&mut self, fb: &Framebuffer) {
        if !self.clipboard_valid {
            self.set_copy_status(b"Paste: clipboard is empty");
            self.redraw(fb);
            return;
        }
        if self.view != ExplorerView::Root || self.full_disk {
            self.set_copy_status(b"Paste: open a HouseOS folder first");
            self.redraw(fb);
            return;
        }
        let entry = self.clipboard_entry;
        let ok = if let Some(range) = self.fs_img {
            unsafe {
                crate::fat32::copy_file_to_dir(
                    range,
                    entry.parent_cluster,
                    &entry.name[..entry.name_len],
                    self.current_cluster,
                )
            }
        } else {
            false
        };
        if ok {
            self.entries_dirty = true;
            self.selected_entry = None;
            self.set_copy_complete_status(entry.size);
        } else {
            self.set_copy_status(b"Paste failed: no space or unsupported name");
        }
        self.redraw(fb);
    }

    fn set_copy_complete_status(&mut self, bytes: u32) {
        let mut msg = [0u8; 96];
        let mut len = 0usize;
        len += copy_bytes(&mut msg[len..], b"Copy complete: ");
        len += write_size_label(&mut msg[len..], bytes as u64);
        len += copy_bytes(&mut msg[len..], b" / ");
        len += write_size_label(&mut msg[len..], bytes as u64);
        len += copy_bytes(&mut msg[len..], b" 100% Speed: local FS");
        self.set_copy_status(&msg[..len]);
    }

    fn set_copy_status(&mut self, text: &[u8]) {
        self.copy_status.fill(0);
        self.copy_status_len = text.len().min(self.copy_status.len());
        self.copy_status[..self.copy_status_len].copy_from_slice(&text[..self.copy_status_len]);
    }

    fn handle_sidebar_click(&mut self, fb: &Framebuffer, layout: &Layout, x: usize, y: usize) {
        let sidebar_x = layout.content_x + PAD;
        let sidebar_w = layout.sidebar_w.saturating_sub(PAD * 2);
        let places_y = layout.body_y + PAD;
        let house_y = places_y + LINE_HEIGHT + 4;
        let bin_y = house_y + LINE_HEIGHT;
        if hit(x, y, sidebar_x, house_y, sidebar_w, LINE_HEIGHT) {
            self.full_disk = false;
            self.view = ExplorerView::Root;
            self.reset_path();
            self.selected_entry = None;
            self.redraw(fb);
            return;
        }
        if hit(x, y, sidebar_x, bin_y, sidebar_w, LINE_HEIGHT) {
            self.full_disk = false;
            self.view = ExplorerView::Bin;
            self.entries_dirty = true;
            self.selected_entry = None;
            self.redraw(fb);
            return;
        }

        let devices_y = bin_y + LINE_HEIGHT + SIDEBAR_GAP;
        let full_y = devices_y + LINE_HEIGHT + 4;
        if hit(x, y, sidebar_x, full_y, sidebar_w, LINE_HEIGHT) {
            self.open_full_root(fb);
            return;
        }

        let mut row_y = full_y + LINE_HEIGHT;
        let parts = ata_pio::partitions();
        for i in 0..parts.len() {
            if row_y + LINE_HEIGHT > layout.body_y + layout.body_h.saturating_sub(PAD) {
                return;
            }
            if hit(x, y, sidebar_x, row_y, sidebar_w, LINE_HEIGHT) {
                self.open_partition_from_sidebar(fb, i);
                return;
            }
            row_y += LINE_HEIGHT;
        }

        let controllers = usb::controllers();
        for _ in 0..controllers.len() {
            if row_y + LINE_HEIGHT > layout.body_y + layout.body_h.saturating_sub(PAD) {
                return;
            }
            if hit(x, y, sidebar_x, row_y, sidebar_w, LINE_HEIGHT) {
                self.open_full_root(fb);
                return;
            }
            row_y += LINE_HEIGHT;
        }
    }

    fn open_full_root(&mut self, fb: &Framebuffer) {
        self.view = ExplorerView::Root;
        self.full_disk = true;
        self.full_mode = FULL_PARTITIONS;
        self.full_part_idx = usize::MAX;
        self.depth = 0;
        self.ntfs_current_ref = ntfs::ROOT_FILE_REF;
        self.entries_dirty = true;
        self.selected_entry = None;
        self.redraw(fb);
    }

    fn open_partition_from_sidebar(&mut self, fb: &Framebuffer, part_index: usize) {
        self.view = ExplorerView::Root;
        self.full_disk = true;
        self.full_part_idx = part_index;
        self.selected_entry = None;
        self.scroll_offset = 0;
        if ntfs::NtfsVolume::open(part_index).is_some() {
            self.full_mode = FULL_NTFS;
            self.ntfs_current_ref = ntfs::ROOT_FILE_REF;
            self.depth = 1;
            let mut label = [0u8; MAX_NAME];
            let len = partition_label(part_index, &mut label);
            self.path[0] = PathItem { name: label, len };
            self.entries_dirty = true;
        } else if let Some(vol) = exfat::ExfatVolume::open(part_index) {
            self.full_mode = FULL_EXFAT;
            self.current_cluster = vol.root_cluster();
            self.depth = 1;
            let mut label = [0u8; MAX_NAME];
            let len = partition_label(part_index, &mut label);
            self.path[0] = PathItem { name: label, len };
            self.entries_dirty = true;
        } else {
            self.full_mode = FULL_PARTITIONS;
            self.depth = 0;
            self.entries_dirty = true;
        }
        self.redraw(fb);
    }

    fn draw_sidebar(
        &self,
        fb: &Framebuffer,
        writer: &mut crate::TextWriter,
        layout: &Layout,
        accent: u32,
        text_color: u32,
        detail: u32,
    ) {
        let sidebar_x = layout.content_x + PAD;
        let sidebar_w = layout.sidebar_w.saturating_sub(PAD * 2);
        let places_y = layout.body_y + PAD;
        writer.set_color(detail);
        writer.set_pos(sidebar_x, places_y);
        writer.write_bytes(b"Places");

        let house_y = places_y + LINE_HEIGHT + 4;
        draw_sidebar_item(
            fb,
            writer,
            sidebar_x,
            house_y,
            sidebar_w,
            b"HouseOS Disk",
            self.view == ExplorerView::Root && !self.full_disk,
            accent,
            text_color,
        );
        draw_sidebar_item(
            fb,
            writer,
            sidebar_x,
            house_y + LINE_HEIGHT,
            sidebar_w,
            b"Recycle Bin",
            self.view == ExplorerView::Bin,
            accent,
            text_color,
        );

        let devices_y = house_y + LINE_HEIGHT * 2 + SIDEBAR_GAP;
        writer.set_color(detail);
        writer.set_pos(sidebar_x, devices_y);
        writer.write_bytes(b"Devices");

        let full_y = devices_y + LINE_HEIGHT + 4;
        draw_sidebar_item(
            fb,
            writer,
            sidebar_x,
            full_y,
            sidebar_w,
            b"This PC / Full Disk",
            self.view == ExplorerView::Root && self.full_disk && self.full_mode == FULL_PARTITIONS,
            accent,
            text_color,
        );

        let mut row_y = full_y + LINE_HEIGHT;
        let bottom = layout.body_y + layout.body_h.saturating_sub(PAD);
        let parts = ata_pio::partitions();
        for i in 0..parts.len() {
            if row_y + LINE_HEIGHT > bottom {
                return;
            }
            let mut label = [0u8; MAX_NAME];
            let len = partition_label(i, &mut label);
            draw_sidebar_item(
                fb,
                writer,
                sidebar_x,
                row_y,
                sidebar_w,
                &label[..len],
                self.full_disk
                    && (self.full_mode == FULL_NTFS || self.full_mode == FULL_EXFAT)
                    && self.full_part_idx == i,
                accent,
                text_color,
            );
            row_y += LINE_HEIGHT;
        }

        let controllers = usb::controllers();
        for i in 0..controllers.len() {
            if row_y + LINE_HEIGHT > bottom {
                return;
            }
            let mut label = [0u8; MAX_NAME];
            let len = usb_sidebar_label(i, &mut label);
            draw_sidebar_item(
                fb,
                writer,
                sidebar_x,
                row_y,
                sidebar_w,
                &label[..len],
                false,
                accent,
                text_color,
            );
            row_y += LINE_HEIGHT;
        }

        if parts.is_empty() && controllers.is_empty() && row_y + LINE_HEIGHT <= bottom {
            writer.set_color(detail);
            writer.set_pos(sidebar_x, row_y + 2);
            writer.write_bytes(b"No devices");
        }
    }

    fn reset_path(&mut self) {
        self.depth = 0;
        self.current_cluster = 2;
        self.entries_dirty = true;
        self.full_mode = FULL_PARTITIONS;
        self.full_part_idx = usize::MAX;
        self.ntfs_current_ref = ntfs::ROOT_FILE_REF;
        self.ntfs_stack = [0; MAX_PATH];
        if self.full_disk {
            return;
        }
        let fs_img = self.fs_img;
        if let Some(fs) = fs_img.and_then(Fat32::new) {
            let root = fs.root_cluster();
            self.current_cluster = root;
            if !self.full_disk {
                if let Some(house) = fs.find_dir(root, b"HOUSE_OS") {
                    self.current_cluster = house;
                }
            }
        }
    }

    fn recycle_move(&mut self, entry: FileEntry) -> bool {
        for i in 0..MAX_RECYCLE {
            let rec = self.recycle[i];
            if rec.state == REC_NONE {
                continue;
            }
            if rec.parent_cluster == entry.parent_cluster
                && rec.cluster == entry.cluster
                && rec.name_len == entry.name_len
                && names_equal(rec.name, entry.name, rec.name_len)
            {
                if rec.state == REC_PURGED {
                    return false;
                }
                self.recycle[i].state = REC_BIN;
                return true;
            }
        }

        for i in 0..MAX_RECYCLE {
            if self.recycle[i].state == REC_NONE {
                self.recycle[i] = RecycleEntry {
                    state: REC_BIN,
                    name: entry.name,
                    name_len: entry.name_len,
                    kind: entry.kind,
                    cluster: entry.cluster,
                    size: entry.size,
                    parent_cluster: entry.parent_cluster,
                };
                return true;
            }
        }
        false
    }

    fn is_hidden(&self, parent_cluster: u32, name: [u8; MAX_NAME], name_len: usize, cluster: u32) -> bool {
        for i in 0..MAX_RECYCLE {
            let rec = self.recycle[i];
            if rec.state == REC_NONE {
                continue;
            }
            if rec.parent_cluster == parent_cluster
                && rec.cluster == cluster
                && rec.name_len == name_len
                && names_equal(rec.name, name, name_len)
            {
                return true;
            }
        }
        false
    }

    fn rebuild_entries(&mut self, max_lines: usize) {
        self.entry_count = 0;
        self.visible_count = 0;

        if self.view == ExplorerView::Bin {
            for i in 0..MAX_RECYCLE {
                let rec = self.recycle[i];
                if rec.state != REC_BIN {
                    continue;
                }
                if self.entry_count >= MAX_ENTRIES {
                    break;
                }
                self.entries[self.entry_count] = FileEntry {
                    name: rec.name,
                    name_len: rec.name_len,
                    kind: rec.kind,
                    cluster: rec.cluster,
                    size: rec.size,
                    parent_cluster: rec.parent_cluster,
                    recycle_idx: i,
                    source: SRC_HOUSE,
                    partition_idx: usize::MAX,
                    ntfs_ref: 0,
                };
                self.entry_count += 1;
            }
            self.visible_count = self.entry_count.min(max_lines);
            return;
        }

        if self.full_disk {
            self.rebuild_full_disk_entries(max_lines);
            return;
        }

        if self.depth > 0 && self.entry_count < MAX_ENTRIES {
            self.entries[0] = FileEntry::back();
            self.entry_count = 1;
        }

        let fs_img = self.fs_img;
        if let Some(fs) = fs_img.and_then(Fat32::new) {
            let mut dir_buf = [DirEntry::EMPTY; MAX_ENTRIES];
            let count = fs.list_dir(self.current_cluster, &mut dir_buf);
            for i in 0..count {
                if self.entry_count >= MAX_ENTRIES {
                    break;
                }
                let d = dir_buf[i];
                if self.is_hidden(self.current_cluster, d.name, d.name_len, d.cluster) {
                    continue;
                }
                self.entries[self.entry_count] = FileEntry {
                    name: d.name,
                    name_len: d.name_len,
                    kind: if d.is_dir { ENTRY_DIR } else { ENTRY_FILE },
                    cluster: d.cluster,
                    size: d.size,
                    parent_cluster: self.current_cluster,
                    recycle_idx: usize::MAX,
                    source: SRC_HOUSE,
                    partition_idx: usize::MAX,
                    ntfs_ref: 0,
                };
                self.entry_count += 1;
            }
        }

        self.visible_count = self.entry_count.min(max_lines);
    }

    fn full_disk_back(&mut self) {
        if self.full_mode == FULL_NTFS {
            if self.depth > 1 {
                self.depth -= 1;
                self.ntfs_current_ref = self.ntfs_stack[self.depth];
            } else {
                self.depth = 0;
                self.full_mode = FULL_PARTITIONS;
                self.full_part_idx = usize::MAX;
                self.ntfs_current_ref = ntfs::ROOT_FILE_REF;
            }
            self.entries_dirty = true;
            self.selected_entry = None;
        } else if self.full_mode == FULL_EXFAT {
            if self.depth > 1 {
                self.depth -= 1;
                self.current_cluster = self.cluster_stack[self.depth];
            } else {
                self.depth = 0;
                self.full_mode = FULL_PARTITIONS;
                self.full_part_idx = usize::MAX;
                self.current_cluster = 2;
            }
            self.entries_dirty = true;
            self.selected_entry = None;
        }
    }

    fn handle_full_disk_entry(&mut self, fb: &Framebuffer, entry: FileEntry) {
        if entry.kind == ENTRY_BACK {
            self.full_disk_back();
            self.redraw(fb);
            return;
        }
        if entry.source == SRC_PARTITION {
            if entry.partition_idx != usize::MAX && ntfs::NtfsVolume::open(entry.partition_idx).is_some() {
                self.full_mode = FULL_NTFS;
                self.full_part_idx = entry.partition_idx;
                self.ntfs_current_ref = ntfs::ROOT_FILE_REF;
                self.depth = 1;
                self.path[0] = PathItem {
                    name: entry.name,
                    len: entry.name_len,
                };
                self.entries_dirty = true;
                self.selected_entry = None;
                self.redraw(fb);
                return;
            }
            if entry.partition_idx != usize::MAX {
                if let Some(vol) = exfat::ExfatVolume::open(entry.partition_idx) {
                    self.full_mode = FULL_EXFAT;
                    self.full_part_idx = entry.partition_idx;
                    self.current_cluster = vol.root_cluster();
                    self.depth = 1;
                    self.path[0] = PathItem {
                        name: entry.name,
                        len: entry.name_len,
                    };
                    self.entries_dirty = true;
                    self.selected_entry = None;
                    self.redraw(fb);
                }
            }
            return;
        }
        if entry.source == SRC_NTFS && entry.kind == ENTRY_DIR {
            if self.depth < MAX_PATH {
                self.ntfs_stack[self.depth] = self.ntfs_current_ref;
                self.path[self.depth] = PathItem {
                    name: entry.name,
                    len: entry.name_len,
                };
                self.depth += 1;
                self.ntfs_current_ref = entry.ntfs_ref;
                self.entries_dirty = true;
                self.selected_entry = None;
                self.redraw(fb);
            }
        } else if entry.source == SRC_EXFAT && entry.kind == ENTRY_DIR && entry.cluster >= 2 {
            if self.depth < MAX_PATH {
                self.cluster_stack[self.depth] = self.current_cluster;
                self.path[self.depth] = PathItem {
                    name: entry.name,
                    len: entry.name_len,
                };
                self.depth += 1;
                self.current_cluster = entry.cluster;
                self.entries_dirty = true;
                self.selected_entry = None;
                self.redraw(fb);
            }
        }
    }

    fn rebuild_full_disk_entries(&mut self, max_lines: usize) {
        self.entry_count = 0;
        self.visible_count = 0;

        if self.full_mode == FULL_PARTITIONS {
            self.add_device_entries();
            let parts = ata_pio::partitions();
            if parts.is_empty() && self.entry_count == 0 {
                self.add_info_entry(b"No ATA/GPT disks found");
                self.add_info_entry(b"Run run.ps1 as Administrator for Windows disk");
                self.add_info_entry(b"Host disks are attached read-only");
                self.visible_count = self.entry_count.min(max_lines);
                return;
            }
            if parts.is_empty() {
                self.add_info_entry(b"No partitions found on detected disks");
                self.visible_count = self.entry_count.min(max_lines);
                return;
            }
            for i in 0..parts.len() {
                if self.entry_count >= MAX_ENTRIES {
                    break;
                }
                let p = parts[i];
                let mut name = [0u8; MAX_NAME];
                let mut len = partition_label(i, &mut name);
                let is_ntfs = ntfs::NtfsVolume::open(i).is_some();
                let is_exfat = !is_ntfs && exfat::ExfatVolume::open(i).is_some();
                if is_ntfs {
                    len += copy_bytes(&mut name[len..], b" NTFS");
                    let mut volume_name = [0u8; MAX_NAME];
                    let volume_len = partition_volume_label(i, &mut volume_name);
                    if volume_len > 0 {
                        len += copy_bytes(&mut name[len..], b" ");
                        len += copy_bytes(&mut name[len..], &volume_name[..volume_len]);
                    }
                } else if is_exfat {
                    len += copy_bytes(&mut name[len..], b" exFAT");
                    let mut volume_name = [0u8; MAX_NAME];
                    let volume_len = partition_volume_label(i, &mut volume_name);
                    if volume_len > 0 {
                        len += copy_bytes(&mut name[len..], b" ");
                        len += copy_bytes(&mut name[len..], &volume_name[..volume_len]);
                    }
                }
                self.entries[self.entry_count] = FileEntry {
                    name,
                    name_len: len.min(MAX_NAME),
                    kind: if is_ntfs || is_exfat { ENTRY_DIR } else { ENTRY_FILE },
                    cluster: 0,
                    size: (p.sectors / 2048).min(u32::MAX as u64) as u32,
                    parent_cluster: 0,
                    recycle_idx: usize::MAX,
                    source: SRC_PARTITION,
                    partition_idx: i,
                    ntfs_ref: 0,
                };
                self.entry_count += 1;
            }
            self.visible_count = self.entry_count.min(max_lines);
            return;
        }

        if self.full_mode == FULL_NTFS {
            if self.depth > 0 && self.entry_count < MAX_ENTRIES {
                self.entries[self.entry_count] = FileEntry::back();
                self.entry_count += 1;
            }
            if let Some(vol) = ntfs::NtfsVolume::open(self.full_part_idx) {
                let mut ntfs_entries = [ntfs::NtfsEntry::EMPTY; MAX_ENTRIES];
                let count = vol.list_dir(self.ntfs_current_ref, &mut ntfs_entries);
                for i in 0..count {
                    if self.entry_count >= MAX_ENTRIES {
                        break;
                    }
                    let n = ntfs_entries[i];
                    let mut name = [0u8; MAX_NAME];
                    let name_len = n.name_len.min(MAX_NAME);
                    name[..name_len].copy_from_slice(&n.name[..name_len]);
                    self.entries[self.entry_count] = FileEntry {
                        name,
                        name_len,
                        kind: if n.is_dir { ENTRY_DIR } else { ENTRY_FILE },
                        cluster: 0,
                        size: n.size.min(u32::MAX as u64) as u32,
                        parent_cluster: 0,
                        recycle_idx: usize::MAX,
                        source: SRC_NTFS,
                        partition_idx: self.full_part_idx,
                        ntfs_ref: n.file_ref,
                    };
                    self.entry_count += 1;
                }
            } else {
                self.add_info_entry(b"NTFS reader cannot open this partition");
            }
            self.visible_count = self.entry_count.min(max_lines);
            return;
        }

        if self.full_mode == FULL_EXFAT {
            if self.depth > 0 && self.entry_count < MAX_ENTRIES {
                self.entries[self.entry_count] = FileEntry::back();
                self.entry_count += 1;
            }
            if let Some(vol) = exfat::ExfatVolume::open(self.full_part_idx) {
                let mut exfat_entries = [exfat::ExfatEntry::EMPTY; MAX_ENTRIES];
                let count = vol.list_dir(self.current_cluster, &mut exfat_entries);
                for i in 0..count {
                    if self.entry_count >= MAX_ENTRIES {
                        break;
                    }
                    let n = exfat_entries[i];
                    let mut name = [0u8; MAX_NAME];
                    let name_len = n.name_len.min(MAX_NAME);
                    name[..name_len].copy_from_slice(&n.name[..name_len]);
                    self.entries[self.entry_count] = FileEntry {
                        name,
                        name_len,
                        kind: if n.is_dir { ENTRY_DIR } else { ENTRY_FILE },
                        cluster: n.first_cluster,
                        size: n.size.min(u32::MAX as u64) as u32,
                        parent_cluster: self.current_cluster,
                        recycle_idx: usize::MAX,
                        source: SRC_EXFAT,
                        partition_idx: self.full_part_idx,
                        ntfs_ref: 0,
                    };
                    self.entry_count += 1;
                }
            } else {
                self.add_info_entry(b"exFAT reader cannot open this partition");
            }
            self.visible_count = self.entry_count.min(max_lines);
        }
    }

    fn add_info_entry(&mut self, text: &[u8]) {
        if self.entry_count >= MAX_ENTRIES {
            return;
        }
        let mut name = [0u8; MAX_NAME];
        let len = copy_bytes(&mut name, text);
        self.entries[self.entry_count] = FileEntry {
            name,
            name_len: len,
            kind: ENTRY_FILE,
            cluster: 0,
            size: 0,
            parent_cluster: 0,
            recycle_idx: usize::MAX,
            source: SRC_PARTITION,
            partition_idx: usize::MAX,
            ntfs_ref: 0,
        };
        self.entry_count += 1;
    }

    fn add_device_entries(&mut self) {
        let drives = ata_pio::drives();
        for i in 0..drives.len() {
            if self.entry_count >= MAX_ENTRIES {
                return;
            }
            let d = drives[i];
            let mib = (d.sectors / 2048).min(u32::MAX as u64) as u32;
            let mut name = [0u8; MAX_NAME];
            let mut len = 0usize;
            len += copy_bytes(&mut name[len..], b"ATA Disk ");
            len += write_u32(&mut name[len..], i as u32);
            len += copy_bytes(&mut name[len..], if d.slave { b" Slave " } else { b" Master " });
            if mib >= 1024 {
                len += write_u32(&mut name[len..], mib / 1024);
                len += copy_bytes(&mut name[len..], b" GiB");
            } else {
                len += write_u32(&mut name[len..], mib);
                len += copy_bytes(&mut name[len..], b" MiB");
            }
            if d.supports_lba48 {
                len += copy_bytes(&mut name[len..], b" LBA48");
            }
            self.entries[self.entry_count] = FileEntry {
                name,
                name_len: len.min(MAX_NAME),
                kind: ENTRY_FILE,
                cluster: 0,
                size: mib,
                parent_cluster: 0,
                recycle_idx: usize::MAX,
                source: SRC_DEVICE,
                partition_idx: usize::MAX,
                ntfs_ref: 0,
            };
            self.entry_count += 1;
        }

        let controllers = usb::controllers();
        for i in 0..controllers.len() {
            if self.entry_count >= MAX_ENTRIES {
                return;
            }
            let c = controllers[i];
            let mut name = [0u8; MAX_NAME];
            let mut len = 0usize;
            len += copy_bytes(&mut name[len..], b"USB ");
            len += copy_bytes(&mut name[len..], usb_kind_label(c.kind));
            len += copy_bytes(&mut name[len..], b" controller ");
            len += write_u32(&mut name[len..], c.bus as u32);
            len += copy_bytes(&mut name[len..], b":");
            len += write_u32(&mut name[len..], c.dev as u32);
            len += copy_bytes(&mut name[len..], b".");
            len += write_u32(&mut name[len..], c.func as u32);
            self.entries[self.entry_count] = FileEntry {
                name,
                name_len: len.min(MAX_NAME),
                kind: ENTRY_FILE,
                cluster: 0,
                size: 0,
                parent_cluster: 0,
                recycle_idx: usize::MAX,
                source: SRC_DEVICE,
                partition_idx: usize::MAX,
                ntfs_ref: 0,
            };
            self.entry_count += 1;
        }
    }

    fn layout(&self, _fb: &Framebuffer, content: (usize, usize, usize, usize)) -> Layout {
        let content_x = content.0;
        let content_y = content.1;
        let content_w = content.2;
        let content_h = content.3;
        let toolbar_y = content_y;
        let body_y = content_y + TOOLBAR_H;
        let body_h = content_h.saturating_sub(TOOLBAR_H);
        let sidebar_w = (content_w / 4).max(120).min(content_w.saturating_sub(200));
        let list_x = content_x + sidebar_w + PAD;
        let list_header_y = body_y + PAD;
        let list_start_y = list_header_y + LINE_HEIGHT;
        let list_w = content_w.saturating_sub(sidebar_w + PAD * 2 + SCROLL_W + 4);
        let list_h = body_h.saturating_sub(PAD * 2 + LINE_HEIGHT);
        let max_lines = if LINE_HEIGHT == 0 { 0 } else { list_h / LINE_HEIGHT };

        let btn_h = TOOLBAR_H.saturating_sub(6);
        let back_btn = (content_x + PAD, toolbar_y + 3, 54, btn_h);
        let refresh_btn = (back_btn.0 + back_btn.2 + 6, toolbar_y + 3, 72, btn_h);
        let copy_btn = (refresh_btn.0 + refresh_btn.2 + 6, toolbar_y + 3, 58, btn_h);
        let paste_btn = (copy_btn.0 + copy_btn.2 + 6, toolbar_y + 3, 58, btn_h);
        let action_btn = (paste_btn.0 + paste_btn.2 + 6, toolbar_y + 3, 64, btn_h);
        let purge_btn = (action_btn.0 + action_btn.2 + 6, toolbar_y + 3, 106, btn_h);

        Layout {
            content_x,
            content_y,
            content_w,
            content_h,
            toolbar_y,
            body_y,
            body_h,
            sidebar_w,
            list_x,
            list_header_y,
            list_start_y,
            list_w,
            list_h,
            max_lines,
            back_btn,
            refresh_btn,
            copy_btn,
            paste_btn,
            action_btn,
            purge_btn,
        }
    }

    fn content_rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        let (x, y, w, h) = self.rect(fb);
        let content_x = x + 2;
        let content_y = y + window::HEADER_H + 2;
        let content_w = w.saturating_sub(4);
        let content_h = h.saturating_sub(window::HEADER_H + 4);
        (content_x, content_y, content_w, content_h)
    }
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
        if is_dark { 0x00494848 } else { 0x00EAF0F8 },
        if is_dark { 0x003F3F3F } else { 0x00D8E2EE },
    );
    writer.set_color(text_color);
    let text_w = label.len() * 8;
    let tx = rect.0 + rect.2.saturating_sub(text_w) / 2;
    writer.set_pos(tx, rect.1 + 6);
    writer.write_bytes(label);
}

fn draw_pretty_scrollbar(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    thumb_y: usize,
    thumb_h: usize,
    is_dark: bool,
    accent: u32,
) {
    let track = if is_dark { 0x0020262E } else { 0x00E7EEF7 };
    let edge = if is_dark { 0x00424B58 } else { 0x00C8D5E4 };
    let thumb = blend_rgb(accent, if is_dark { 0x00FFFFFF } else { 0x00000000 }, if is_dark { 28 } else { 12 });
    let shine = if is_dark { 0x00677A8D } else { 0x00FFFFFF };

    display::fill_rect(fb, x, y, w, h, edge);
    if w > 2 && h > 2 {
        display::fill_rect(fb, x + 1, y + 1, w - 2, h - 2, track);
    }

    let ty = thumb_y.max(y).min(y + h.saturating_sub(1));
    let th = thumb_h.min(y + h - ty).max(4);
    if w > 4 && th > 4 {
        display::fill_rect(fb, x + 2, ty + 2, w - 4, th - 4, thumb);
        display::fill_rect(fb, x + 3, ty + 3, w - 6, 1, shine);
    } else {
        display::fill_rect(fb, x, ty, w, th, thumb);
    }
}

fn draw_copy_status_panel(
    fb: &Framebuffer,
    writer: &mut crate::TextWriter,
    layout: &Layout,
    text: &[u8],
    is_dark: bool,
    accent: u32,
    text_color: u32,
    detail: u32,
) {
    let panel_w = layout.list_w.min(420);
    let panel_h = 44usize;
    let x = layout.list_x + layout.list_w.saturating_sub(panel_w) / 2;
    let y = layout
        .list_start_y
        .saturating_add(layout.list_h.saturating_sub(panel_h + 8));
    let bg_top = if is_dark { 0x00333A45 } else { 0x00FFFFFF };
    let bg_bottom = if is_dark { 0x00252C36 } else { 0x00EAF3FF };
    fill_vertical_gradient(fb, x, y, panel_w, panel_h, bg_top, bg_bottom);
    display::fill_rect(fb, x, y, panel_w, 1, blend_rgb(accent, 0x00FFFFFF, 38));
    display::fill_rect(fb, x, y + panel_h.saturating_sub(1), panel_w, 1, if is_dark { 0x00566070 } else { 0x00BFD2E8 });

    let bar_x = x + 12;
    let bar_y = y + 25;
    let bar_w = panel_w.saturating_sub(24);
    display::fill_rect(fb, bar_x, bar_y, bar_w, 8, if is_dark { 0x001B2028 } else { 0x00D7E4F3 });
    display::fill_rect(fb, bar_x, bar_y, bar_w, 8, blend_rgb(accent, 0x00FFFFFF, 22));

    writer.set_color(text_color);
    writer.set_pos(x + 12, y + 7);
    let max_chars = panel_w.saturating_sub(24) / 8;
    draw_entry_name(writer, text, max_chars.max(4), None);
    writer.set_color(detail);
    writer.set_pos(x + panel_w.saturating_sub(44), bar_y - 1);
    writer.write_bytes(b"100%");
}

fn draw_sidebar_item(
    fb: &Framebuffer,
    writer: &mut crate::TextWriter,
    x: usize,
    y: usize,
    w: usize,
    label: &[u8],
    active: bool,
    accent: u32,
    text: u32,
) {
    if active {
        let active_bg = blend_rgb(accent, 0x00FFFFFF, 34);
        display::fill_rect(
            fb,
            x.saturating_sub(4),
            y.saturating_sub(2),
            w + 8,
            LINE_HEIGHT,
            active_bg,
        );
        writer.set_color(0x00FFFFFF);
    } else {
        writer.set_color(text);
    }
    writer.set_pos(x, y);
    let max_chars = w.saturating_sub(8) / 8;
    draw_entry_name(writer, label, max_chars.max(4), None);
}

fn entry_type_label(entry: FileEntry) -> &'static [u8] {
    if entry.kind == ENTRY_BACK {
        return b"[..]";
    }
    if entry.source == SRC_DEVICE {
        if name_starts_with(entry.name, entry.name_len, b"USB") {
            return b"[USB]";
        }
        return b"[DISK]";
    }
    if entry.source == SRC_PARTITION {
        return b"[PART]";
    }
    if entry.kind == ENTRY_DIR {
        return b"[DIR]";
    }
    if is_image_file(entry.name, entry.name_len) {
        return b"[IMG]";
    }
    if is_text_file(entry.name, entry.name_len) {
        return b"[TXT]";
    }
    if is_media_file(entry.name, entry.name_len) {
        return b"[MEDIA]";
    }
    b"[FILE]"
}

fn usb_kind_label(kind: UsbControllerKind) -> &'static [u8] {
    match kind {
        UsbControllerKind::Uhci => b"UHCI",
        UsbControllerKind::Ohci => b"OHCI",
        UsbControllerKind::Ehci => b"EHCI",
        UsbControllerKind::Xhci => b"XHCI",
        UsbControllerKind::Unknown => b"controller",
    }
}

fn partition_label(part_index: usize, out: &mut [u8; MAX_NAME]) -> usize {
    let parts = ata_pio::partitions();
    if part_index >= parts.len() {
        return copy_bytes(out, b"Disk partition");
    }
    let p = parts[part_index];
    let mut len = 0usize;
    len += copy_bytes(&mut out[len..], b"Disk ");
    len += write_u32(&mut out[len..], p.drive_index as u32);
    len += copy_bytes(&mut out[len..], b" Part ");
    len += write_u32(&mut out[len..], p.index as u32 + 1);
    if p.gpt {
        len += copy_bytes(&mut out[len..], b" GPT");
    } else {
        len += copy_bytes(&mut out[len..], b" MBR");
    }
    if p.name_len > 0 {
        len += copy_bytes(&mut out[len..], b" ");
        len += copy_bytes(&mut out[len..], &p.name[..p.name_len.min(p.name.len())]);
    }
    let mib = p.sectors / 2048;
    if mib > 0 && len + 6 < out.len() {
        len += copy_bytes(&mut out[len..], b" ");
        if mib >= 1024 {
            len += write_u32(&mut out[len..], (mib / 1024).min(u32::MAX as u64) as u32);
            len += copy_bytes(&mut out[len..], b"G");
        } else {
            len += write_u32(&mut out[len..], mib.min(u32::MAX as u64) as u32);
            len += copy_bytes(&mut out[len..], b"M");
        }
    }
    len.min(MAX_NAME)
}

fn partition_volume_label(part_index: usize, out: &mut [u8; MAX_NAME]) -> usize {
    if let Some(vol) = ntfs::NtfsVolume::open(part_index) {
        return vol.volume_label(out).min(MAX_NAME);
    }
    if let Some(vol) = exfat::ExfatVolume::open(part_index) {
        return vol.volume_label(out).min(MAX_NAME);
    }
    0
}

fn usb_sidebar_label(index: usize, out: &mut [u8; MAX_NAME]) -> usize {
    let controllers = usb::controllers();
    if index >= controllers.len() {
        return copy_bytes(out, b"USB controller");
    }
    let c = controllers[index];
    let mut len = 0usize;
    len += copy_bytes(&mut out[len..], b"USB ");
    len += copy_bytes(&mut out[len..], usb_kind_label(c.kind));
    len += copy_bytes(&mut out[len..], b" ");
    len += write_u32(&mut out[len..], c.bus as u32);
    len += copy_bytes(&mut out[len..], b":");
    len += write_u32(&mut out[len..], c.dev as u32);
    len.min(MAX_NAME)
}

fn name_starts_with(name: [u8; MAX_NAME], len: usize, prefix: &[u8]) -> bool {
    if len < prefix.len() {
        return false;
    }
    for i in 0..prefix.len() {
        if name[i] != prefix[i] {
            return false;
        }
    }
    true
}

fn draw_entry_name(
    writer: &mut crate::TextWriter,
    name: &[u8],
    max_chars: usize,
    suffix: Option<u8>,
) {
    let suffix_chars = if suffix.is_some() { 1 } else { 0 };
    if max_chars <= suffix_chars {
        return;
    }
    let allowed = max_chars - suffix_chars;
    if name.len() <= allowed {
        writer.write_bytes(name);
    } else if allowed > 3 {
        writer.write_bytes(&name[..allowed - 3]);
        writer.write_bytes(b"...");
    } else {
        writer.write_bytes(&name[..allowed]);
    }
    if let Some(ch) = suffix {
        let one = [ch];
        writer.write_bytes(&one);
    }
}

fn is_text_file(name: [u8; MAX_NAME], len: usize) -> bool {
    if len < 4 {
        return false;
    }
    let mut dot = usize::MAX;
    for i in 0..len {
        if name[i] == b'.' {
            dot = i;
        }
    }
    if dot == usize::MAX || dot + 1 >= len {
        return false;
    }
    let ext = &name[dot + 1..len];
    eq_ascii_ci(ext, b"TXT")
        || eq_ascii_ci(ext, b"LOG")
        || eq_ascii_ci(ext, b"INI")
        || eq_ascii_ci(ext, b"CFG")
        || eq_ascii_ci(ext, b"CSV")
}

fn is_media_file(name: [u8; MAX_NAME], len: usize) -> bool {
    if len < 4 {
        return false;
    }
    let mut dot = usize::MAX;
    for i in 0..len {
        if name[i] == b'.' {
            dot = i;
        }
    }
    if dot == usize::MAX || dot + 1 >= len {
        return false;
    }
    let ext = &name[dot + 1..len];
    eq_ascii_ci(ext, b"WAV")
        || eq_ascii_ci(ext, b"MP3")
        || eq_ascii_ci(ext, b"SND")
        || eq_ascii_ci(ext, b"RAW")
        || eq_ascii_ci(ext, b"PCM")
        || eq_ascii_ci(ext, b"MP4")
        || eq_ascii_ci(ext, b"AVI")
        || eq_ascii_ci(ext, b"MOV")
        || eq_ascii_ci(ext, b"WEBM")
        || eq_ascii_ci(ext, b"MPG")
        || eq_ascii_ci(ext, b"MPEG")
}

fn is_image_file(name: [u8; MAX_NAME], len: usize) -> bool {
    if len < 4 {
        return false;
    }
    let mut dot = usize::MAX;
    for i in 0..len {
        if name[i] == b'.' {
            dot = i;
        }
    }
    if dot == usize::MAX || dot + 1 >= len {
        return false;
    }
    let ext = &name[dot + 1..len];
    eq_ascii_ci(ext, b"JPG") || eq_ascii_ci(ext, b"JPEG")
}

fn eq_ascii_ci(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        let ac = ascii_lower(a[i]);
        let bc = ascii_lower(b[i]);
        if ac != bc {
            return false;
        }
    }
    true
}

fn ascii_lower(b: u8) -> u8 {
    if (b'A'..=b'Z').contains(&b) {
        b + 32
    } else {
        b
    }
}

fn names_equal(a: [u8; MAX_NAME], b: [u8; MAX_NAME], len: usize) -> bool {
    for i in 0..len {
        if a[i] != b[i] {
            return false;
        }
    }
    true
}

fn copy_bytes(out: &mut [u8], src: &[u8]) -> usize {
    let len = out.len().min(src.len());
    out[..len].copy_from_slice(&src[..len]);
    len
}

fn write_size_label(out: &mut [u8], bytes: u64) -> usize {
    let units = [b"B" as &[u8], b"KB", b"MB", b"GB", b"TB"];
    let mut value = bytes;
    let mut unit = 0usize;
    while value >= 1024 && unit + 1 < units.len() {
        value /= 1024;
        unit += 1;
    }
    let mut len = write_u64(out, value);
    len += copy_bytes(&mut out[len..], b" ");
    len += copy_bytes(&mut out[len..], units[unit]);
    len
}

fn write_u64(buf: &mut [u8], mut val: u64) -> usize {
    if buf.is_empty() {
        return 0;
    }
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = 0usize;
    while val > 0 && n < tmp.len() {
        tmp[n] = (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    let mut out = 0usize;
    while n > 0 && out < buf.len() {
        n -= 1;
        buf[out] = b'0' + tmp[n];
        out += 1;
    }
    out
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
    let mut n = 0usize;
    while val > 0 && n < tmp.len() {
        tmp[n] = (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    let mut out = 0usize;
    while n > 0 && out < buf.len() {
        n -= 1;
        buf[out] = b'0' + tmp[n];
        out += 1;
    }
    out
}

fn calc_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let w = (fb.width * 3 / 4).min(800).max(400);
    let h = (fb.height * 3 / 4).min(600).max(300);
    let x = (fb.width.saturating_sub(w)) / 2;
    let y = (fb.height.saturating_sub(h)) / 2;
    (x, y, w, h)
}

fn hit(px: usize, py: usize, x: usize, y: usize, w: usize, h: usize) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
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

fn blend_rgb(base: u32, mix: u32, mix_strength: u8) -> u32 {
    let s = mix_strength as u32;
    let inv = 255u32.saturating_sub(s);
    let br = (base >> 16) & 0xFF;
    let bg = (base >> 8) & 0xFF;
    let bb = base & 0xFF;
    let mr = (mix >> 16) & 0xFF;
    let mg = (mix >> 8) & 0xFF;
    let mb = mix & 0xFF;
    let r = (br * inv + mr * s) / 255;
    let g = (bg * inv + mg * s) / 255;
    let b = (bb * inv + mb * s) / 255;
    (r << 16) | (g << 8) | b
}
