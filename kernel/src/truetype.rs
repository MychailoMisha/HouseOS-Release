use crate::display::Framebuffer;

const TTF_MAGIC: u32 = 0x0001_0000;
const TTC_MAGIC: u32 = 0x7474_6366;
const MAX_CONTOURS: usize = 32;
const MAX_POINTS: usize = 256;

#[derive(Copy, Clone)]
struct GlyphPoint {
    x: i16,
    y: i16,
    on_curve: bool,
}

impl GlyphPoint {
    const EMPTY: GlyphPoint = GlyphPoint {
        x: 0,
        y: 0,
        on_curve: false,
    };
}

pub struct TtfFont {
    data: &'static [u8],
    glyf_offset: usize,
    loca_offset: usize,
    cmap4_offset: usize,
    hmtx_offset: usize,
    num_glyphs: u16,
    units_per_em: u16,
    index_to_loc_format: i16,
    num_h_metrics: u16,
}

impl TtfFont {
    pub fn new(data: &'static [u8]) -> Option<Self> {
        if data.len() < 12 {
            return None;
        }
        let sfnt = read_u32_be(data, 0)?;
        if sfnt != TTF_MAGIC && sfnt != 0x4F54_544F && sfnt != TTC_MAGIC {
            return None;
        }
        let table_base = if sfnt == TTC_MAGIC {
            read_u32_be(data, 12)? as usize
        } else {
            0
        };
        if table_base + 12 > data.len() {
            return None;
        }
        let num_tables = read_u16_be(data, table_base + 4)? as usize;
        let dir = table_base + 12;
        if dir + num_tables * 16 > data.len() {
            return None;
        }

        let head = find_table(data, dir, num_tables, b"head")?;
        let maxp = find_table(data, dir, num_tables, b"maxp")?;
        let hhea = find_table(data, dir, num_tables, b"hhea")?;
        let glyf = find_table(data, dir, num_tables, b"glyf")?;
        let loca = find_table(data, dir, num_tables, b"loca")?;
        let cmap = find_table(data, dir, num_tables, b"cmap")?;
        let hmtx = find_table(data, dir, num_tables, b"hmtx")?;

        if read_u32_be(data, head + 12)? != 0x5F0F_3CF5 {
            return None;
        }
        let units_per_em = read_u16_be(data, head + 18)?;
        let index_to_loc_format = read_i16_be(data, head + 50)?;
        let num_glyphs = read_u16_be(data, maxp + 4)?;
        let num_h_metrics = read_u16_be(data, hhea + 34)?;
        let cmap4_offset = find_cmap4(data, cmap)?;
        if units_per_em == 0 || num_glyphs == 0 || num_h_metrics == 0 {
            return None;
        }

        Some(Self {
            data,
            glyf_offset: glyf,
            loca_offset: loca,
            cmap4_offset,
            hmtx_offset: hmtx,
            num_glyphs,
            units_per_em,
            index_to_loc_format,
            num_h_metrics,
        })
    }

    fn glyph_index(&self, char_code: u16) -> u16 {
        let data = self.data;
        let off = self.cmap4_offset;
        let seg_count = match read_u16_be(data, off + 6) {
            Some(v) => (v / 2) as usize,
            None => return 0,
        };
        if seg_count == 0 || seg_count > 512 {
            return 0;
        }

        let end_codes = off + 14;
        let start_codes = end_codes + seg_count * 2 + 2;
        let id_deltas = start_codes + seg_count * 2;
        let id_range_offsets = id_deltas + seg_count * 2;
        if id_range_offsets + seg_count * 2 > data.len() {
            return 0;
        }

        for i in 0..seg_count {
            let end = read_u16_be(data, end_codes + i * 2).unwrap_or(0);
            let start = read_u16_be(data, start_codes + i * 2).unwrap_or(0);
            if char_code < start || char_code > end {
                continue;
            }

            let delta = read_i16_be(data, id_deltas + i * 2).unwrap_or(0) as i32;
            let ro_pos = id_range_offsets + i * 2;
            let range_offset = read_u16_be(data, ro_pos).unwrap_or(0) as usize;
            if range_offset == 0 {
                return ((char_code as i32 + delta) & 0xFFFF) as u16;
            }

            let glyph_pos = ro_pos + range_offset + ((char_code - start) as usize) * 2;
            let glyph = read_u16_be(data, glyph_pos).unwrap_or(0);
            if glyph == 0 {
                return 0;
            }
            return ((glyph as i32 + delta) & 0xFFFF) as u16;
        }
        0
    }

    fn glyph_range(&self, glyph_id: u16) -> Option<(usize, usize)> {
        if glyph_id >= self.num_glyphs {
            return None;
        }
        let current = self.glyph_offset(glyph_id)?;
        let next = self.glyph_offset(glyph_id + 1).unwrap_or(current);
        if next <= current {
            return None;
        }
        let start = self.glyf_offset.checked_add(current)?;
        let end = self.glyf_offset.checked_add(next)?;
        if start >= self.data.len() || end > self.data.len() {
            return None;
        }
        Some((start, end))
    }

    fn glyph_offset(&self, glyph_id: u16) -> Option<usize> {
        let gid = glyph_id.min(self.num_glyphs);
        if self.index_to_loc_format == 0 {
            let idx = self.loca_offset + gid as usize * 2;
            Some(read_u16_be(self.data, idx)? as usize * 2)
        } else {
            let idx = self.loca_offset + gid as usize * 4;
            Some(read_u32_be(self.data, idx)? as usize)
        }
    }

    fn advance_width(&self, glyph_id: u16) -> u16 {
        let metric = if glyph_id < self.num_h_metrics {
            glyph_id as usize
        } else {
            self.num_h_metrics.saturating_sub(1) as usize
        };
        read_u16_be(self.data, self.hmtx_offset + metric * 4).unwrap_or(self.units_per_em / 2)
    }

    pub fn render_text(
        &mut self,
        fb: &Framebuffer,
        x: usize,
        y: usize,
        text: &[u8],
        size_px: u16,
        color: u32,
    ) -> bool {
        let mut cur_x = x as i32;
        let size = size_px.max(8) as i32;
        let mut drew_any = false;
        let mut expected = 0usize;
        let mut drawn = 0usize;
        let mut i = 0usize;
        while i < text.len() {
            let (ch, step) = decode_utf8(text, i);
            i += step;
            if ch == b'\r' as u32 || ch == b'\n' as u32 {
                continue;
            }
            if ch > 0xFFFF {
                cur_x += scale_font_units(self.units_per_em as i32 / 3, size, self.units_per_em);
                continue;
            }
            let glyph_id = self.glyph_index(ch as u16);
            if ch == b' ' as u32 || glyph_id == 0 {
                cur_x += scale_font_units(self.units_per_em as i32 / 3, size, self.units_per_em);
                continue;
            }
            expected += 1;
            if self.draw_glyph(fb, cur_x, y as i32, glyph_id, size, color) {
                drew_any = true;
                drawn += 1;
            }
            let adv = self.advance_width(glyph_id) as i32;
            cur_x += scale_font_units(adv, size, self.units_per_em).max(size / 3);
        }
        drew_any && (expected == 0 || drawn.saturating_mul(2) >= expected)
    }

    fn draw_glyph(&self, fb: &Framebuffer, x: i32, baseline_y: i32, glyph_id: u16, size: i32, color: u32) -> bool {
        let (start, end) = match self.glyph_range(glyph_id) {
            Some(v) => v,
            None => return false,
        };
        if start + 10 > end {
            return false;
        }
        let data = self.data;
        let contours = read_i16_be(data, start).unwrap_or(0);
        if contours <= 0 || contours as usize > MAX_CONTOURS {
            return false;
        }
        let contour_count = contours as usize;
        let x_min = read_i16_be(data, start + 2).unwrap_or(0) as i32;
        let y_min = read_i16_be(data, start + 4).unwrap_or(0) as i32;
        let x_max = read_i16_be(data, start + 6).unwrap_or(0) as i32;
        let y_max = read_i16_be(data, start + 8).unwrap_or(0) as i32;
        if x_max <= x_min || y_max <= y_min {
            return false;
        }

        let mut ends = [0u16; MAX_CONTOURS];
        let mut pos = start + 10;
        for i in 0..contour_count {
            ends[i] = read_u16_be(data, pos).unwrap_or(0);
            pos += 2;
        }
        let point_count = ends[contour_count - 1] as usize + 1;
        if point_count == 0 || point_count > MAX_POINTS || pos + 2 > end {
            return false;
        }
        let instruction_len = read_u16_be(data, pos).unwrap_or(0) as usize;
        pos += 2 + instruction_len;
        if pos >= end {
            return false;
        }

        let mut flags = [0u8; MAX_POINTS];
        let mut f = 0usize;
        while f < point_count && pos < end {
            let flag = data[pos];
            pos += 1;
            flags[f] = flag;
            f += 1;
            if (flag & 0x08) != 0 {
                if pos >= end {
                    return false;
                }
                let repeat = data[pos] as usize;
                pos += 1;
                for _ in 0..repeat {
                    if f >= point_count {
                        break;
                    }
                    flags[f] = flag;
                    f += 1;
                }
            }
        }
        if f < point_count {
            return false;
        }

        let mut points = [GlyphPoint::EMPTY; MAX_POINTS];
        let mut x_acc = 0i16;
        for i in 0..point_count {
            let flag = flags[i];
            let dx = if (flag & 0x02) != 0 {
                if pos >= end {
                    return false;
                }
                let v = data[pos] as i16;
                pos += 1;
                if (flag & 0x10) != 0 { v } else { -v }
            } else if (flag & 0x10) != 0 {
                0
            } else {
                if pos + 2 > end {
                    return false;
                }
                let v = read_i16_be(data, pos).unwrap_or(0);
                pos += 2;
                v
            };
            x_acc = x_acc.wrapping_add(dx);
            points[i].x = x_acc;
            points[i].on_curve = (flag & 0x01) != 0;
        }

        let mut y_acc = 0i16;
        for i in 0..point_count {
            let flag = flags[i];
            let dy = if (flag & 0x04) != 0 {
                if pos >= end {
                    return false;
                }
                let v = data[pos] as i16;
                pos += 1;
                if (flag & 0x20) != 0 { v } else { -v }
            } else if (flag & 0x20) != 0 {
                0
            } else {
                if pos + 2 > end {
                    return false;
                }
                let v = read_i16_be(data, pos).unwrap_or(0);
                pos += 2;
                v
            };
            y_acc = y_acc.wrapping_add(dy);
            points[i].y = y_acc;
        }

        let upem = self.units_per_em;
        let px_min = x + scale_font_units(x_min, size, upem) - 1;
        let px_max = x + scale_font_units(x_max, size, upem) + 1;
        let py_min = baseline_y - scale_font_units(y_max, size, upem) - 1;
        let py_max = baseline_y - scale_font_units(y_min, size, upem) + 1;
        if px_max <= px_min || py_max <= py_min {
            return false;
        }

        let width = (px_max - px_min).min(96);
        let height = (py_max - py_min).min(96);
        for yy in 0..height {
            let py = py_min + yy;
            if py < 0 || py >= fb.height as i32 {
                continue;
            }
            let font_y = ((baseline_y - py) * upem as i32) / size;
            for xx in 0..width {
                let px = px_min + xx;
                if px < 0 || px >= fb.width as i32 {
                    continue;
                }
                let font_x = ((px - x) * upem as i32) / size;
                if point_inside(&points[..point_count], &ends[..contour_count], font_x, font_y) {
                    crate::display::put_pixel(fb, px as usize, py as usize, color);
                }
            }
        }
        true
    }
}

fn decode_utf8(text: &[u8], i: usize) -> (u32, usize) {
    let b0 = text[i];
    if b0 < 0x80 {
        return (b0 as u32, 1);
    }
    if (b0 & 0xE0) == 0xC0 && i + 1 < text.len() {
        let b1 = text[i + 1];
        if (b1 & 0xC0) == 0x80 {
            let cp = (((b0 & 0x1F) as u32) << 6) | ((b1 & 0x3F) as u32);
            return (cp, 2);
        }
    }
    if (b0 & 0xF0) == 0xE0 && i + 2 < text.len() {
        let b1 = text[i + 1];
        let b2 = text[i + 2];
        if (b1 & 0xC0) == 0x80 && (b2 & 0xC0) == 0x80 {
            let cp = (((b0 & 0x0F) as u32) << 12)
                | (((b1 & 0x3F) as u32) << 6)
                | ((b2 & 0x3F) as u32);
            return (cp, 3);
        }
    }
    if (b0 & 0xF8) == 0xF0 && i + 3 < text.len() {
        let b1 = text[i + 1];
        let b2 = text[i + 2];
        let b3 = text[i + 3];
        if (b1 & 0xC0) == 0x80 && (b2 & 0xC0) == 0x80 && (b3 & 0xC0) == 0x80 {
            let cp = (((b0 & 0x07) as u32) << 18)
                | (((b1 & 0x3F) as u32) << 12)
                | (((b2 & 0x3F) as u32) << 6)
                | ((b3 & 0x3F) as u32);
            return (cp, 4);
        }
    }
    (b'?' as u32, 1)
}

fn point_inside(points: &[GlyphPoint], ends: &[u16], x: i32, y: i32) -> bool {
    let mut inside = false;
    let mut contour_start = 0usize;
    for &end in ends {
        let contour_end = end as usize;
        if contour_end >= points.len() || contour_start > contour_end {
            break;
        }
        let mut prev = points[contour_end];
        for i in contour_start..=contour_end {
            let current = points[i];
            if segment_crosses(prev, current, x, y) {
                inside = !inside;
            }
            prev = current;
        }
        contour_start = contour_end + 1;
    }
    inside
}

fn segment_crosses(a: GlyphPoint, b: GlyphPoint, x: i32, y: i32) -> bool {
    let y1 = a.y as i32;
    let y2 = b.y as i32;
    if (y1 > y) == (y2 > y) {
        return false;
    }
    let x1 = a.x as i32;
    let x2 = b.x as i32;
    let x_cross = x1 + (y - y1) * (x2 - x1) / (y2 - y1);
    x < x_cross
}

fn find_table(data: &[u8], dir: usize, num_tables: usize, tag: &[u8; 4]) -> Option<usize> {
    for i in 0..num_tables {
        let e = dir + i * 16;
        if e + 16 > data.len() {
            return None;
        }
        if &data[e..e + 4] == tag {
            let offset = read_u32_be(data, e + 8)? as usize;
            let len = read_u32_be(data, e + 12)? as usize;
            if offset < data.len() && offset.saturating_add(len) <= data.len() {
                return Some(offset);
            }
        }
    }
    None
}

fn find_cmap4(data: &[u8], cmap: usize) -> Option<usize> {
    let count = read_u16_be(data, cmap + 2)? as usize;
    let mut fallback = 0usize;
    for i in 0..count {
        let rec = cmap + 4 + i * 8;
        if rec + 8 > data.len() {
            return None;
        }
        let platform = read_u16_be(data, rec)?;
        let encoding = read_u16_be(data, rec + 2)?;
        let sub = cmap + read_u32_be(data, rec + 4)? as usize;
        if sub + 2 > data.len() || read_u16_be(data, sub)? != 4 {
            continue;
        }
        if platform == 3 && (encoding == 1 || encoding == 10) {
            return Some(sub);
        }
        if fallback == 0 {
            fallback = sub;
        }
    }
    if fallback != 0 { Some(fallback) } else { None }
}

fn scale_font_units(v: i32, size: i32, units_per_em: u16) -> i32 {
    (v * size) / units_per_em.max(1) as i32
}

fn read_u16_be(buf: &[u8], off: usize) -> Option<u16> {
    if off + 2 > buf.len() {
        None
    } else {
        Some(((buf[off] as u16) << 8) | buf[off + 1] as u16)
    }
}

fn read_i16_be(buf: &[u8], off: usize) -> Option<i16> {
    read_u16_be(buf, off).map(|v| v as i16)
}

fn read_u32_be(buf: &[u8], off: usize) -> Option<u32> {
    if off + 4 > buf.len() {
        None
    } else {
        Some(
            ((buf[off] as u32) << 24)
                | ((buf[off + 1] as u32) << 16)
                | ((buf[off + 2] as u32) << 8)
                | buf[off + 3] as u32,
        )
    }
}

static mut DEFAULT_FONT: Option<TtfFont> = None;

pub fn init_default_font(data: &'static [u8]) -> bool {
    unsafe {
        if let Some(font) = TtfFont::new(data) {
            DEFAULT_FONT = Some(font);
            true
        } else {
            false
        }
    }
}

pub fn get_default_font() -> Option<&'static mut TtfFont> {
    unsafe { DEFAULT_FONT.as_mut() }
}
