#![allow(dead_code)]

use crate::drivers::storage::ata_pio;

pub const ROOT_FILE_REF: u64 = 5;
pub const MAX_NTFS_NAME: usize = 48;

const SECTOR_SIZE: usize = 512;
const MAX_RUNS: usize = 32;
const MAX_RECORD: usize = 4096;
const MAX_INDEX: usize = 4096;
const VOLUME_FILE_REF: u64 = 3;

const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_VOLUME_NAME: u32 = 0x60;
const ATTR_DATA: u32 = 0x80;
const ATTR_INDEX_ROOT: u32 = 0x90;
const ATTR_INDEX_ALLOCATION: u32 = 0xA0;
const ATTR_END: u32 = 0xFFFF_FFFF;
const FILE_ATTR_DIRECTORY: u32 = 0x1000_0000;

#[derive(Copy, Clone)]
pub struct NtfsEntry {
    pub name: [u8; MAX_NTFS_NAME],
    pub name_len: usize,
    pub is_dir: bool,
    pub file_ref: u64,
    pub size: u64,
}

impl NtfsEntry {
    pub const EMPTY: NtfsEntry = NtfsEntry {
        name: [0; MAX_NTFS_NAME],
        name_len: 0,
        is_dir: false,
        file_ref: 0,
        size: 0,
    };
}

#[derive(Copy, Clone)]
struct Run {
    lcn: i64,
    clusters: u64,
}

impl Run {
    const EMPTY: Run = Run { lcn: 0, clusters: 0 };
}

#[derive(Copy, Clone)]
pub struct NtfsVolume {
    drive_index: usize,
    part_lba: u64,
    bytes_per_sector: usize,
    sectors_per_cluster: usize,
    cluster_size: usize,
    file_record_size: usize,
    index_record_size: usize,
    mft_runs: [Run; MAX_RUNS],
    mft_run_count: usize,
}

impl NtfsVolume {
    pub fn open(partition_index: usize) -> Option<Self> {
        let parts = ata_pio::partitions();
        if partition_index >= parts.len() {
            return None;
        }
        let part = parts[partition_index];
        if !part.present {
            return None;
        }
        let mut boot = [0u8; SECTOR_SIZE];
        if !ata_pio::read_sector(part.drive_index, part.lba_start, &mut boot) {
            return None;
        }
        if &boot[3..11] != b"NTFS    " {
            return None;
        }
        let bytes_per_sector = read_u16_le(&boot, 11) as usize;
        if bytes_per_sector != SECTOR_SIZE {
            return None;
        }
        let sectors_per_cluster = boot[13] as usize;
        if sectors_per_cluster == 0 {
            return None;
        }
        let cluster_size = bytes_per_sector * sectors_per_cluster;
        let mft_lcn = read_u64_le(&boot, 48);
        let file_record_size = decode_ntfs_size(boot[64] as i8, cluster_size)?;
        let index_record_size = decode_ntfs_size(boot[68] as i8, cluster_size)?;
        if file_record_size == 0 || file_record_size > MAX_RECORD || index_record_size == 0 || index_record_size > MAX_INDEX {
            return None;
        }

        let mut record = [0u8; MAX_RECORD];
        let record_slice = &mut record[..file_record_size];
        let mft_lba = part.lba_start + mft_lcn.saturating_mul(sectors_per_cluster as u64);
        if !read_bytes(part.drive_index, mft_lba, record_slice) {
            return None;
        }
        if !apply_fixup(record_slice, bytes_per_sector) || &record_slice[0..4] != b"FILE" {
            return None;
        }

        let mut runs = [Run::EMPTY; MAX_RUNS];
        let mut run_count = 0usize;
        let mut attr = read_u16_le(record_slice, 20) as usize;
        while attr + 16 <= record_slice.len() {
            let attr_type = read_u32_le(record_slice, attr);
            if attr_type == ATTR_END {
                break;
            }
            let attr_len = read_u32_le(record_slice, attr + 4) as usize;
            if attr_len == 0 || attr + attr_len > record_slice.len() {
                break;
            }
            let nonresident = record_slice[attr + 8] != 0;
            let name_len = record_slice[attr + 9];
            if attr_type == ATTR_DATA && nonresident && name_len == 0 {
                let run_off = read_u16_le(record_slice, attr + 32) as usize;
                if run_off < attr_len {
                    run_count = parse_runs(&record_slice[attr + run_off..attr + attr_len], &mut runs);
                    break;
                }
            }
            attr += attr_len;
        }
        if run_count == 0 {
            return None;
        }

        Some(Self {
            drive_index: part.drive_index,
            part_lba: part.lba_start,
            bytes_per_sector,
            sectors_per_cluster,
            cluster_size,
            file_record_size,
            index_record_size,
            mft_runs: runs,
            mft_run_count: run_count,
        })
    }

    pub fn list_dir(&self, file_ref: u64, out: &mut [NtfsEntry]) -> usize {
        let mut record = [0u8; MAX_RECORD];
        if !self.read_record(file_ref, &mut record) {
            return 0;
        }
        let record = &record[..self.file_record_size];
        let mut count = 0usize;
        let mut index_runs = [Run::EMPTY; MAX_RUNS];
        let mut index_run_count = 0usize;
        let mut index_real_size = 0u64;

        let mut attr = read_u16_le(record, 20) as usize;
        while attr + 16 <= record.len() {
            let attr_type = read_u32_le(record, attr);
            if attr_type == ATTR_END {
                break;
            }
            let attr_len = read_u32_le(record, attr + 4) as usize;
            if attr_len == 0 || attr + attr_len > record.len() {
                break;
            }
            let nonresident = record[attr + 8] != 0;
            if attr_type == ATTR_INDEX_ROOT && !nonresident {
                let value_len = read_u32_le(record, attr + 16) as usize;
                let value_off = read_u16_le(record, attr + 20) as usize;
                if value_off + value_len <= attr_len && value_len >= 32 {
                    count = parse_index_root(&record[attr + value_off..attr + value_off + value_len], out, count);
                }
            } else if attr_type == ATTR_INDEX_ALLOCATION && nonresident {
                let run_off = read_u16_le(record, attr + 32) as usize;
                index_real_size = read_u64_le(record, attr + 48);
                if run_off < attr_len {
                    index_run_count = parse_runs(&record[attr + run_off..attr + attr_len], &mut index_runs);
                }
            }
            attr += attr_len;
        }

        if count < out.len() && index_run_count > 0 && index_real_size > 0 {
            count = self.parse_index_allocation(&index_runs, index_run_count, index_real_size, out, count);
        }
        count
    }

    pub fn volume_label(&self, out: &mut [u8; MAX_NTFS_NAME]) -> usize {
        let mut record = [0u8; MAX_RECORD];
        if !self.read_record(VOLUME_FILE_REF, &mut record) {
            return 0;
        }
        let record = &record[..self.file_record_size];
        let mut attr = read_u16_le(record, 20) as usize;
        while attr + 24 <= record.len() {
            let attr_type = read_u32_le(record, attr);
            if attr_type == ATTR_END {
                break;
            }
            let attr_len = read_u32_le(record, attr + 4) as usize;
            if attr_len == 0 || attr + attr_len > record.len() {
                break;
            }
            let nonresident = record[attr + 8] != 0;
            if attr_type == ATTR_VOLUME_NAME && !nonresident {
                let value_len = read_u32_le(record, attr + 16) as usize;
                let value_off = read_u16_le(record, attr + 20) as usize;
                if value_len > 0 && value_off + value_len <= attr_len {
                    return utf16le_ascii(
                        &record[attr + value_off..attr + value_off + value_len],
                        value_len / 2,
                        out,
                    );
                }
                return 0;
            }
            attr += attr_len;
        }
        0
    }

    fn read_record(&self, file_ref: u64, out: &mut [u8; MAX_RECORD]) -> bool {
        let rec = file_ref & 0x0000_FFFF_FFFF_FFFF;
        let byte_off = rec.saturating_mul(self.file_record_size as u64);
        let dst = &mut out[..self.file_record_size];
        if !self.read_from_runs(&self.mft_runs, self.mft_run_count, byte_off, dst) {
            return false;
        }
        apply_fixup(dst, self.bytes_per_sector) && &dst[0..4] == b"FILE"
    }

    fn parse_index_allocation(
        &self,
        runs: &[Run; MAX_RUNS],
        run_count: usize,
        real_size: u64,
        out: &mut [NtfsEntry],
        mut count: usize,
    ) -> usize {
        let mut idx = [0u8; MAX_INDEX];
        let mut off = 0u64;
        while off + self.index_record_size as u64 <= real_size && count < out.len() {
            let dst = &mut idx[..self.index_record_size];
            if !self.read_from_runs(runs, run_count, off, dst) {
                break;
            }
            if apply_fixup(dst, self.bytes_per_sector) && &dst[0..4] == b"INDX" {
                count = parse_index_buffer(dst, out, count);
            }
            off += self.index_record_size as u64;
        }
        count
    }

    fn read_from_runs(&self, runs: &[Run; MAX_RUNS], run_count: usize, offset: u64, out: &mut [u8]) -> bool {
        let mut copied = 0usize;
        while copied < out.len() {
            let abs = offset + copied as u64;
            let vcn = abs / self.cluster_size as u64;
            let in_cluster = (abs % self.cluster_size as u64) as usize;
            let lcn = match map_vcn(runs, run_count, vcn) {
                Some(v) => v,
                None => return false,
            };
            if lcn < 0 {
                return false;
            }
            let sector_in_cluster = in_cluster / SECTOR_SIZE;
            let in_sector = in_cluster % SECTOR_SIZE;
            let lba = self.part_lba
                + (lcn as u64).saturating_mul(self.sectors_per_cluster as u64)
                + sector_in_cluster as u64;
            let mut sector = [0u8; SECTOR_SIZE];
            if !ata_pio::read_sector(self.drive_index, lba, &mut sector) {
                return false;
            }
            let chunk = (SECTOR_SIZE - in_sector).min(out.len() - copied);
            out[copied..copied + chunk].copy_from_slice(&sector[in_sector..in_sector + chunk]);
            copied += chunk;
        }
        true
    }
}

fn parse_index_root(value: &[u8], out: &mut [NtfsEntry], count: usize) -> usize {
    if value.len() < 32 {
        return count;
    }
    let hdr = 16usize;
    let entries_off = read_u32_le(value, hdr) as usize;
    let entries_size = read_u32_le(value, hdr + 4) as usize;
    let start = hdr + entries_off;
    let end = start.saturating_add(entries_size).min(value.len());
    parse_index_entries(value, start, end, out, count)
}

fn parse_index_buffer(buf: &[u8], out: &mut [NtfsEntry], count: usize) -> usize {
    if buf.len() < 40 {
        return count;
    }
    let hdr = 24usize;
    let entries_off = read_u32_le(buf, hdr) as usize;
    let entries_size = read_u32_le(buf, hdr + 4) as usize;
    let start = hdr + entries_off;
    let end = start.saturating_add(entries_size).min(buf.len());
    parse_index_entries(buf, start, end, out, count)
}

fn parse_index_entries(buf: &[u8], mut pos: usize, end: usize, out: &mut [NtfsEntry], mut count: usize) -> usize {
    while pos + 16 <= end && count < out.len() {
        let file_ref = read_u64_le_at(buf, pos) & 0x0000_FFFF_FFFF_FFFF;
        let entry_len = read_u16_le(buf, pos + 8) as usize;
        let key_len = read_u16_le(buf, pos + 10) as usize;
        let flags = read_u16_le(buf, pos + 12);
        if entry_len < 16 || pos + entry_len > end {
            break;
        }
        if (flags & 0x02) != 0 {
            break;
        }
        let key = pos + 16;
        if key_len >= 66 && key + key_len <= buf.len() {
            let namespace = buf[key + 65];
            if namespace != 2 {
                let mut name = [0u8; MAX_NTFS_NAME];
                let name_len = utf16le_ascii(&buf[key + 66..key + key_len], buf[key + 64] as usize, &mut name);
                if name_len > 0 {
                    let attrs = read_u32_le(buf, key + 56);
                    let size = read_u64_le_at(buf, key + 48);
                    out[count] = NtfsEntry {
                        name,
                        name_len,
                        is_dir: (attrs & FILE_ATTR_DIRECTORY) != 0,
                        file_ref,
                        size,
                    };
                    count += 1;
                }
            }
        }
        pos += entry_len;
    }
    count
}

fn map_vcn(runs: &[Run; MAX_RUNS], run_count: usize, vcn: u64) -> Option<i64> {
    let mut base = 0u64;
    for i in 0..run_count {
        let run = runs[i];
        if vcn >= base && vcn < base + run.clusters {
            return Some(run.lcn + (vcn - base) as i64);
        }
        base += run.clusters;
    }
    None
}

fn parse_runs(data: &[u8], out: &mut [Run; MAX_RUNS]) -> usize {
    let mut pos = 0usize;
    let mut count = 0usize;
    let mut current_lcn = 0i64;
    while pos < data.len() && count < out.len() {
        let header = data[pos];
        pos += 1;
        if header == 0 {
            break;
        }
        let len_size = (header & 0x0F) as usize;
        let off_size = (header >> 4) as usize;
        if len_size == 0 || pos + len_size + off_size > data.len() {
            break;
        }
        let clusters = read_var_unsigned(&data[pos..pos + len_size]);
        pos += len_size;
        let delta = read_var_signed(&data[pos..pos + off_size]);
        pos += off_size;
        current_lcn += delta;
        if clusters > 0 {
            out[count] = Run {
                lcn: current_lcn,
                clusters,
            };
            count += 1;
        }
    }
    count
}

fn apply_fixup(buf: &mut [u8], sector_size: usize) -> bool {
    if buf.len() < 8 || sector_size == 0 {
        return false;
    }
    let usa_off = read_u16_le(buf, 4) as usize;
    let usa_count = read_u16_le(buf, 6) as usize;
    if usa_count == 0 || usa_off + usa_count * 2 > buf.len() {
        return false;
    }
    for i in 1..usa_count {
        let end = i * sector_size;
        if end < 2 || end > buf.len() {
            return false;
        }
        let src = usa_off + i * 2;
        buf[end - 2] = buf[src];
        buf[end - 1] = buf[src + 1];
    }
    true
}

fn read_bytes(drive_index: usize, lba: u64, out: &mut [u8]) -> bool {
    if out.len() % SECTOR_SIZE != 0 {
        return false;
    }
    let sectors = out.len() / SECTOR_SIZE;
    if sectors == 0 || sectors > u16::MAX as usize {
        return false;
    }
    ata_pio::read_sectors(drive_index, lba, sectors as u16, out)
}

fn decode_ntfs_size(raw: i8, cluster_size: usize) -> Option<usize> {
    if raw < 0 {
        let shift = (-raw) as usize;
        if shift >= usize::BITS as usize {
            None
        } else {
            Some(1usize << shift)
        }
    } else {
        Some(raw as usize * cluster_size)
    }
}

fn utf16le_ascii(src: &[u8], max_chars: usize, out: &mut [u8; MAX_NTFS_NAME]) -> usize {
    let mut len = 0usize;
    let mut i = 0usize;
    let mut chars = 0usize;
    while i + 1 < src.len() && len < out.len() && chars < max_chars {
        let code = (src[i] as u16) | ((src[i + 1] as u16) << 8);
        if code == 0 {
            break;
        }
        append_utf16_ascii(code, out, &mut len);
        chars += 1;
        i += 2;
    }
    len
}

fn append_utf16_ascii(code: u16, out: &mut [u8; MAX_NTFS_NAME], len: &mut usize) {
    match code {
        0x20..=0x7E => append_name_bytes(out, len, &[code as u8]),
        0x00A0 => append_name_bytes(out, len, b" "),
        0x2010 | 0x2011 | 0x2012 | 0x2013 | 0x2014 | 0x2212 => append_name_bytes(out, len, b"-"),
        0x2018 | 0x2019 | 0x02BC => append_name_bytes(out, len, b"'"),
        0x201C | 0x201D => append_name_bytes(out, len, b"\""),
        0x0401 => append_name_bytes(out, len, b"Yo"),
        0x0451 => append_name_bytes(out, len, b"yo"),
        0x0404 => append_name_bytes(out, len, b"Ye"),
        0x0454 => append_name_bytes(out, len, b"ye"),
        0x0406 => append_name_bytes(out, len, b"I"),
        0x0456 => append_name_bytes(out, len, b"i"),
        0x0407 => append_name_bytes(out, len, b"Yi"),
        0x0457 => append_name_bytes(out, len, b"yi"),
        0x0490 => append_name_bytes(out, len, b"G"),
        0x0491 => append_name_bytes(out, len, b"g"),
        0x0410 => append_name_bytes(out, len, b"A"),
        0x0430 => append_name_bytes(out, len, b"a"),
        0x0411 => append_name_bytes(out, len, b"B"),
        0x0431 => append_name_bytes(out, len, b"b"),
        0x0412 => append_name_bytes(out, len, b"V"),
        0x0432 => append_name_bytes(out, len, b"v"),
        0x0413 => append_name_bytes(out, len, b"H"),
        0x0433 => append_name_bytes(out, len, b"h"),
        0x0414 => append_name_bytes(out, len, b"D"),
        0x0434 => append_name_bytes(out, len, b"d"),
        0x0415 => append_name_bytes(out, len, b"E"),
        0x0435 => append_name_bytes(out, len, b"e"),
        0x0416 => append_name_bytes(out, len, b"Zh"),
        0x0436 => append_name_bytes(out, len, b"zh"),
        0x0417 => append_name_bytes(out, len, b"Z"),
        0x0437 => append_name_bytes(out, len, b"z"),
        0x0418 => append_name_bytes(out, len, b"Y"),
        0x0438 => append_name_bytes(out, len, b"y"),
        0x0419 => append_name_bytes(out, len, b"Y"),
        0x0439 => append_name_bytes(out, len, b"y"),
        0x041A => append_name_bytes(out, len, b"K"),
        0x043A => append_name_bytes(out, len, b"k"),
        0x041B => append_name_bytes(out, len, b"L"),
        0x043B => append_name_bytes(out, len, b"l"),
        0x041C => append_name_bytes(out, len, b"M"),
        0x043C => append_name_bytes(out, len, b"m"),
        0x041D => append_name_bytes(out, len, b"N"),
        0x043D => append_name_bytes(out, len, b"n"),
        0x041E => append_name_bytes(out, len, b"O"),
        0x043E => append_name_bytes(out, len, b"o"),
        0x041F => append_name_bytes(out, len, b"P"),
        0x043F => append_name_bytes(out, len, b"p"),
        0x0420 => append_name_bytes(out, len, b"R"),
        0x0440 => append_name_bytes(out, len, b"r"),
        0x0421 => append_name_bytes(out, len, b"S"),
        0x0441 => append_name_bytes(out, len, b"s"),
        0x0422 => append_name_bytes(out, len, b"T"),
        0x0442 => append_name_bytes(out, len, b"t"),
        0x0423 => append_name_bytes(out, len, b"U"),
        0x0443 => append_name_bytes(out, len, b"u"),
        0x0424 => append_name_bytes(out, len, b"F"),
        0x0444 => append_name_bytes(out, len, b"f"),
        0x0425 => append_name_bytes(out, len, b"Kh"),
        0x0445 => append_name_bytes(out, len, b"kh"),
        0x0426 => append_name_bytes(out, len, b"Ts"),
        0x0446 => append_name_bytes(out, len, b"ts"),
        0x0427 => append_name_bytes(out, len, b"Ch"),
        0x0447 => append_name_bytes(out, len, b"ch"),
        0x0428 => append_name_bytes(out, len, b"Sh"),
        0x0448 => append_name_bytes(out, len, b"sh"),
        0x0429 => append_name_bytes(out, len, b"Shch"),
        0x0449 => append_name_bytes(out, len, b"shch"),
        0x042A | 0x044A | 0x042C | 0x044C => {}
        0x042B => append_name_bytes(out, len, b"Y"),
        0x044B => append_name_bytes(out, len, b"y"),
        0x042D => append_name_bytes(out, len, b"E"),
        0x044D => append_name_bytes(out, len, b"e"),
        0x042E => append_name_bytes(out, len, b"Yu"),
        0x044E => append_name_bytes(out, len, b"yu"),
        0x042F => append_name_bytes(out, len, b"Ya"),
        0x044F => append_name_bytes(out, len, b"ya"),
        _ => append_name_bytes(out, len, b"_"),
    }
}

fn append_name_bytes(out: &mut [u8; MAX_NTFS_NAME], len: &mut usize, bytes: &[u8]) {
    for &b in bytes {
        if *len >= out.len() {
            break;
        }
        out[*len] = b;
        *len += 1;
    }
}

fn read_var_unsigned(buf: &[u8]) -> u64 {
    let mut v = 0u64;
    for i in 0..buf.len().min(8) {
        v |= (buf[i] as u64) << (i * 8);
    }
    v
}

fn read_var_signed(buf: &[u8]) -> i64 {
    if buf.is_empty() {
        return 0;
    }
    let mut v = read_var_unsigned(buf) as i64;
    let bits = buf.len().min(8) * 8;
    if bits < 64 && (buf[buf.len() - 1] & 0x80) != 0 {
        v |= !0i64 << bits;
    }
    v
}

fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    if off + 2 > buf.len() {
        return 0;
    }
    (buf[off] as u16) | ((buf[off + 1] as u16) << 8)
}

fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    if off + 4 > buf.len() {
        return 0;
    }
    (buf[off] as u32)
        | ((buf[off + 1] as u32) << 8)
        | ((buf[off + 2] as u32) << 16)
        | ((buf[off + 3] as u32) << 24)
}

fn read_u64_le(buf: &[u8], off: usize) -> u64 {
    if off + 8 > buf.len() {
        return 0;
    }
    read_u64_le_at(buf, off)
}

fn read_u64_le_at(buf: &[u8], off: usize) -> u64 {
    (buf[off] as u64)
        | ((buf[off + 1] as u64) << 8)
        | ((buf[off + 2] as u64) << 16)
        | ((buf[off + 3] as u64) << 24)
        | ((buf[off + 4] as u64) << 32)
        | ((buf[off + 5] as u64) << 40)
        | ((buf[off + 6] as u64) << 48)
        | ((buf[off + 7] as u64) << 56)
}
