#![allow(dead_code)]

use crate::drivers::storage::ata_pio;

pub const MAX_EXFAT_NAME: usize = 48;

const SECTOR_SIZE: usize = 512;
const ENTRY_SIZE: usize = 32;
const MAX_DIR_CLUSTERS: usize = 128;

const ENTRY_TYPE_VOLUME_LABEL: u8 = 0x83;
const ENTRY_TYPE_FILE: u8 = 0x85;
const ENTRY_TYPE_STREAM: u8 = 0xC0;
const ENTRY_TYPE_NAME: u8 = 0xC1;
const FILE_ATTR_DIRECTORY: u16 = 0x0010;

#[derive(Copy, Clone)]
pub struct ExfatEntry {
    pub name: [u8; MAX_EXFAT_NAME],
    pub name_len: usize,
    pub is_dir: bool,
    pub first_cluster: u32,
    pub size: u64,
}

impl ExfatEntry {
    pub const EMPTY: ExfatEntry = ExfatEntry {
        name: [0; MAX_EXFAT_NAME],
        name_len: 0,
        is_dir: false,
        first_cluster: 0,
        size: 0,
    };
}

#[derive(Copy, Clone)]
pub struct ExfatVolume {
    drive_index: usize,
    part_lba: u64,
    fat_lba: u64,
    cluster_heap_lba: u64,
    sectors_per_cluster: usize,
    cluster_count: u32,
    root_dir_cluster: u32,
}

#[derive(Copy, Clone)]
struct PendingFile {
    active: bool,
    stream: bool,
    is_dir: bool,
    expected_chars: usize,
    seen_chars: usize,
    first_cluster: u32,
    size: u64,
    name: [u8; MAX_EXFAT_NAME],
    name_len: usize,
}

impl PendingFile {
    const EMPTY: PendingFile = PendingFile {
        active: false,
        stream: false,
        is_dir: false,
        expected_chars: 0,
        seen_chars: 0,
        first_cluster: 0,
        size: 0,
        name: [0; MAX_EXFAT_NAME],
        name_len: 0,
    };

    fn reset(&mut self) {
        *self = PendingFile::EMPTY;
    }
}

impl ExfatVolume {
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
        if &boot[3..11] != b"EXFAT   " {
            return None;
        }

        let fat_offset = read_u32_le(&boot, 80) as u64;
        let cluster_heap_offset = read_u32_le(&boot, 88) as u64;
        let cluster_count = read_u32_le(&boot, 92);
        let root_dir_cluster = read_u32_le(&boot, 96);
        let bytes_per_sector_shift = boot[108] as usize;
        let sectors_per_cluster_shift = boot[109] as usize;
        let number_of_fats = boot[110];

        if bytes_per_sector_shift != 9 || sectors_per_cluster_shift > 7 || number_of_fats == 0 {
            return None;
        }
        let sectors_per_cluster = 1usize << sectors_per_cluster_shift;
        if sectors_per_cluster == 0 || cluster_count == 0 || root_dir_cluster < 2 {
            return None;
        }

        Some(Self {
            drive_index: part.drive_index,
            part_lba: part.lba_start,
            fat_lba: part.lba_start + fat_offset,
            cluster_heap_lba: part.lba_start + cluster_heap_offset,
            sectors_per_cluster,
            cluster_count,
            root_dir_cluster,
        })
    }

    pub fn root_cluster(&self) -> u32 {
        self.root_dir_cluster
    }

    pub fn volume_label(&self, out: &mut [u8; MAX_EXFAT_NAME]) -> usize {
        let cluster = self.root_dir_cluster;
        if !self.valid_cluster(cluster) {
            return 0;
        }
        let base_lba = self.cluster_lba(cluster);
        for sector in 0..self.sectors_per_cluster {
            let mut buf = [0u8; SECTOR_SIZE];
            if !ata_pio::read_sector(self.drive_index, base_lba + sector as u64, &mut buf) {
                return 0;
            }
            let mut off = 0usize;
            while off + ENTRY_SIZE <= SECTOR_SIZE {
                let entry_type = buf[off];
                if entry_type == 0 {
                    return 0;
                }
                if entry_type == ENTRY_TYPE_VOLUME_LABEL {
                    let chars = (buf[off + 1] as usize).min(11);
                    let mut len = 0usize;
                    let mut p = off + 2;
                    for _ in 0..chars {
                        if p + 1 >= off + ENTRY_SIZE {
                            break;
                        }
                        let code = (buf[p] as u16) | ((buf[p + 1] as u16) << 8);
                        if code == 0 {
                            break;
                        }
                        append_utf16_ascii(code, out, &mut len);
                        p += 2;
                    }
                    return len;
                }
                off += ENTRY_SIZE;
            }
        }
        0
    }

    pub fn list_dir(&self, start_cluster: u32, out: &mut [ExfatEntry]) -> usize {
        if !self.valid_cluster(start_cluster) {
            return 0;
        }

        let mut count = 0usize;
        let mut cluster = start_cluster;
        let mut clusters_read = 0usize;
        let mut pending = PendingFile::EMPTY;

        while self.valid_cluster(cluster) && clusters_read < MAX_DIR_CLUSTERS && count < out.len() {
            let base_lba = self.cluster_lba(cluster);
            for sector in 0..self.sectors_per_cluster {
                let mut buf = [0u8; SECTOR_SIZE];
                if !ata_pio::read_sector(self.drive_index, base_lba + sector as u64, &mut buf) {
                    return count;
                }

                let mut off = 0usize;
                while off + ENTRY_SIZE <= SECTOR_SIZE && count < out.len() {
                    let entry_type = buf[off];
                    if entry_type == 0 {
                        return count;
                    }
                    if (entry_type & 0x80) == 0 {
                        pending.reset();
                        off += ENTRY_SIZE;
                        continue;
                    }

                    match entry_type {
                        ENTRY_TYPE_FILE => {
                            pending.reset();
                            pending.active = true;
                            let attr = read_u16_le(&buf, off + 4);
                            pending.is_dir = (attr & FILE_ATTR_DIRECTORY) != 0;
                        }
                        ENTRY_TYPE_STREAM => {
                            if pending.active {
                                pending.stream = true;
                                pending.expected_chars = buf[off + 3] as usize;
                                pending.first_cluster = read_u32_le(&buf, off + 20);
                                pending.size = read_u64_le(&buf, off + 24);
                            }
                        }
                        ENTRY_TYPE_NAME => {
                            if pending.active && pending.stream {
                                append_name_entry(&buf[off + 2..off + ENTRY_SIZE], &mut pending);
                                if pending.expected_chars == 0 || pending.seen_chars >= pending.expected_chars {
                                    if pending.name_len > 0 {
                                        out[count] = ExfatEntry {
                                            name: pending.name,
                                            name_len: pending.name_len,
                                            is_dir: pending.is_dir,
                                            first_cluster: pending.first_cluster,
                                            size: pending.size,
                                        };
                                        count += 1;
                                    }
                                    pending.reset();
                                }
                            }
                        }
                        _ => {}
                    }

                    off += ENTRY_SIZE;
                }
            }

            clusters_read += 1;
            match self.next_cluster(cluster) {
                Some(next) if next != cluster => cluster = next,
                _ => break,
            }
        }

        count
    }

    fn cluster_lba(&self, cluster: u32) -> u64 {
        self.cluster_heap_lba
            + (cluster.saturating_sub(2) as u64).saturating_mul(self.sectors_per_cluster as u64)
    }

    fn valid_cluster(&self, cluster: u32) -> bool {
        cluster >= 2 && cluster < self.cluster_count.saturating_add(2)
    }

    fn next_cluster(&self, cluster: u32) -> Option<u32> {
        if !self.valid_cluster(cluster) {
            return None;
        }
        let offset = cluster as u64 * 4;
        let lba = self.fat_lba + offset / SECTOR_SIZE as u64;
        let in_sector = (offset % SECTOR_SIZE as u64) as usize;
        let mut buf = [0u8; SECTOR_SIZE];
        if !ata_pio::read_sector(self.drive_index, lba, &mut buf) || in_sector + 4 > buf.len() {
            return None;
        }
        let next = read_u32_le(&buf, in_sector);
        if next >= 0xFFFF_FFF8 || next == 0 {
            None
        } else if self.valid_cluster(next) {
            Some(next)
        } else {
            None
        }
    }
}

fn append_name_entry(src: &[u8], pending: &mut PendingFile) {
    let mut i = 0usize;
    while i + 1 < src.len() && pending.seen_chars < pending.expected_chars {
        let code = (src[i] as u16) | ((src[i + 1] as u16) << 8);
        if code == 0 {
            break;
        }
        append_utf16_ascii(code, &mut pending.name, &mut pending.name_len);
        pending.seen_chars += 1;
        i += 2;
    }
}

fn append_utf16_ascii(code: u16, out: &mut [u8; MAX_EXFAT_NAME], len: &mut usize) {
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

fn append_name_bytes(out: &mut [u8; MAX_EXFAT_NAME], len: &mut usize, bytes: &[u8]) {
    for &b in bytes {
        if *len >= out.len() {
            break;
        }
        out[*len] = b;
        *len += 1;
    }
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
    (buf[off] as u64)
        | ((buf[off + 1] as u64) << 8)
        | ((buf[off + 2] as u64) << 16)
        | ((buf[off + 3] as u64) << 24)
        | ((buf[off + 4] as u64) << 32)
        | ((buf[off + 5] as u64) << 40)
        | ((buf[off + 6] as u64) << 48)
        | ((buf[off + 7] as u64) << 56)
}
