use crate::drivers::storage::ata_pio;
use crate::ModuleRange;

const FAT32_EOC: u32 = 0x0FFFFFF8;
pub const MAX_NAME: usize = 48;
const MAX_COPY_CLUSTERS: usize = 1024;

#[derive(Copy, Clone)]
pub struct DirEntry {
    pub name: [u8; MAX_NAME],
    pub name_len: usize,
    pub is_dir: bool,
    pub cluster: u32,
    pub size: u32,
}

impl DirEntry {
    pub const EMPTY: DirEntry = DirEntry {
        name: [0u8; MAX_NAME],
        name_len: 0,
        is_dir: false,
        cluster: 0,
        size: 0,
    };
}

pub struct Fat32<'a> {
    data: &'a [u8],
    bytes_per_sector: usize,
    sectors_per_cluster: usize,
    fat_start: usize,
    fat_size: usize,
    fats: usize,
    data_start: usize,
    root_cluster: u32,
    total_clusters: u32,
}

impl<'a> Fat32<'a> {
    pub fn new(range: ModuleRange) -> Option<Self> {
        let len = range.end.saturating_sub(range.start);
        if len < 512 {
            return None;
        }
        let raw = unsafe { core::slice::from_raw_parts(range.start as *const u8, len) };
        let data = locate_fat32_volume(raw)?;

        let bytes_per_sector = read_u16_le(data, 11) as usize;
        if bytes_per_sector == 0 || (bytes_per_sector & (bytes_per_sector - 1)) != 0 {
            return None;
        }
        let sectors_per_cluster = data[13] as usize;
        if sectors_per_cluster == 0 {
            return None;
        }
        let reserved_sectors = read_u16_le(data, 14) as usize;
        let fats = data[16] as usize;
        if fats == 0 {
            return None;
        }
        let fat_size16 = read_u16_le(data, 22) as usize;
        let fat_size32 = read_u32_le(data, 36) as usize;
        let fat_size = if fat_size16 != 0 {
            fat_size16
        } else {
            fat_size32
        };
        if fat_size == 0 {
            return None;
        }
        let total16 = read_u16_le(data, 19) as usize;
        let total32 = read_u32_le(data, 32) as usize;
        let total_sectors = if total16 != 0 { total16 } else { total32 };
        if total_sectors == 0 {
            return None;
        }
        let root_cluster = read_u32_le(data, 44);
        if root_cluster < 2 {
            return None;
        }
        let fat_start = reserved_sectors * bytes_per_sector;
        let fat_bytes = fat_size * bytes_per_sector;
        let data_start = fat_start + fat_bytes * fats;
        if data_start >= data.len() {
            return None;
        }
        let data_sectors = total_sectors.saturating_sub(reserved_sectors + fat_size * fats);
        let total_clusters = (data_sectors / sectors_per_cluster) as u32;
        if total_clusters == 0 {
            return None;
        }
        Some(Self {
            data,
            bytes_per_sector,
            sectors_per_cluster,
            fat_start,
            fat_size,
            fats,
            data_start,
            root_cluster,
            total_clusters,
        })
    }

    pub fn root_cluster(&self) -> u32 {
        self.root_cluster
    }

    pub fn list_dir(&self, start_cluster: u32, out: &mut [DirEntry]) -> usize {
        let mut count = 0usize;
        let mut cluster = start_cluster;
        if cluster < 2 {
            return 0;
        }

        let mut lfn_name = [0u8; MAX_NAME];
        let mut lfn_len = 0usize;
        let mut lfn_active = false;
        let cluster_size = self.cluster_size();

        loop {
            let offset = match self.cluster_offset(cluster) {
                Some(v) => v,
                None => break,
            };
            let end = offset + cluster_size;
            if end > self.data.len() {
                break;
            }
            let buf = &self.data[offset..end];
            let mut pos = 0usize;
            while pos + 32 <= buf.len() {
                let entry = &buf[pos..pos + 32];
                let first = entry[0];
                if first == 0x00 {
                    return count;
                }
                if first == 0xE5 {
                    clear_lfn(&mut lfn_name, &mut lfn_len, &mut lfn_active);
                    pos += 32;
                    continue;
                }

                let attr = entry[11];
                if attr == 0x0F {
                    collect_lfn(entry, &mut lfn_name, &mut lfn_len, &mut lfn_active);
                    pos += 32;
                    continue;
                }

                if (attr & 0x08) == 0 {
                    let mut name = [0u8; MAX_NAME];
                    let name_len = if lfn_active && lfn_len > 0 {
                        name[..lfn_len].copy_from_slice(&lfn_name[..lfn_len]);
                        lfn_len
                    } else {
                        sfn_to_name(entry, &mut name)
                    };

                    if name_len > 0 && !is_dot_entry(&name, name_len) {
                        if count < out.len() {
                            let cluster_hi = read_u16_le(entry, 20);
                            let cluster_lo = read_u16_le(entry, 26);
                            let cluster_val = ((cluster_hi as u32) << 16) | cluster_lo as u32;
                            let size = read_u32_le(entry, 28);
                            out[count] = DirEntry {
                                name,
                                name_len,
                                is_dir: (attr & 0x10) != 0,
                                cluster: cluster_val,
                                size,
                            };
                            count += 1;
                        }
                    }
                }

                clear_lfn(&mut lfn_name, &mut lfn_len, &mut lfn_active);
                pos += 32;
            }
            let next = self.fat_entry(cluster);
            if next < 2 || next >= FAT32_EOC {
                break;
            }
            cluster = next;
        }
        count
    }

    pub fn find_dir(&self, start_cluster: u32, target: &[u8]) -> Option<u32> {
        let mut dir_buf = [DirEntry::EMPTY; 32];
        let count = self.list_dir(start_cluster, &mut dir_buf);
        for i in 0..count {
            let entry = dir_buf[i];
            if entry.is_dir && ascii_eq_ci(&entry.name[..entry.name_len], target) {
                return Some(entry.cluster);
            }
        }
        None
    }

    pub fn find_file(&self, start_cluster: u32, target: &[u8]) -> Option<DirEntry> {
        let mut dir_buf = [DirEntry::EMPTY; 64];
        let count = self.list_dir(start_cluster, &mut dir_buf);
        for i in 0..count {
            let entry = dir_buf[i];
            if !entry.is_dir && ascii_eq_ci(&entry.name[..entry.name_len], target) {
                return Some(entry);
            }
        }
        None
    }

    pub fn read_file(&self, start_cluster: u32, file_size: usize, out: &mut [u8]) -> usize {
        if start_cluster < 2 || out.is_empty() || file_size == 0 {
            return 0;
        }

        let mut cluster = start_cluster;
        let cluster_size = self.cluster_size();
        let mut written = 0usize;
        let target = file_size.min(out.len());

        while written < target {
            let offset = match self.cluster_offset(cluster) {
                Some(v) => v,
                None => break,
            };
            let end = offset.saturating_add(cluster_size);
            if end > self.data.len() {
                break;
            }
            let src = &self.data[offset..end];
            let remain = target - written;
            let to_copy = remain.min(src.len());
            out[written..written + to_copy].copy_from_slice(&src[..to_copy]);
            written += to_copy;

            if written >= target {
                break;
            }
            let next = self.fat_entry(cluster);
            if next < 2 || next >= FAT32_EOC {
                break;
            }
            cluster = next;
        }

        written
    }

    fn cluster_size(&self) -> usize {
        self.bytes_per_sector * self.sectors_per_cluster
    }

    fn cluster_offset(&self, cluster: u32) -> Option<usize> {
        if cluster < 2 {
            return None;
        }
        let idx = (cluster - 2) as usize;
        let offset = self.data_start + idx * self.cluster_size();
        if offset >= self.data.len() {
            return None;
        }
        Some(offset)
    }

    fn fat_entry(&self, cluster: u32) -> u32 {
        if cluster > self.total_clusters + 1 {
            return FAT32_EOC;
        }
        let offset = self.fat_start + (cluster as usize) * 4;
        if offset + 4 > self.data.len() {
            return FAT32_EOC;
        }
        read_u32_le(self.data, offset) & 0x0FFF_FFFF
    }

    fn find_entry_offset(&self, start_cluster: u32, target: &[u8]) -> Option<(usize, DirEntry)> {
        let mut cluster = start_cluster;
        if cluster < 2 {
            return None;
        }

        let mut lfn_name = [0u8; MAX_NAME];
        let mut lfn_len = 0usize;
        let mut lfn_active = false;
        let cluster_size = self.cluster_size();

        loop {
            let offset = self.cluster_offset(cluster)?;
            let end = offset + cluster_size;
            if end > self.data.len() {
                return None;
            }
            let buf = &self.data[offset..end];
            let mut pos = 0usize;
            while pos + 32 <= buf.len() {
                let entry = &buf[pos..pos + 32];
                let first = entry[0];
                if first == 0x00 {
                    return None;
                }
                if first == 0xE5 {
                    clear_lfn(&mut lfn_name, &mut lfn_len, &mut lfn_active);
                    pos += 32;
                    continue;
                }

                let attr = entry[11];
                if attr == 0x0F {
                    collect_lfn(entry, &mut lfn_name, &mut lfn_len, &mut lfn_active);
                    pos += 32;
                    continue;
                }

                if (attr & 0x08) == 0 {
                    let mut name = [0u8; MAX_NAME];
                    let name_len = if lfn_active && lfn_len > 0 {
                        name[..lfn_len].copy_from_slice(&lfn_name[..lfn_len]);
                        lfn_len
                    } else {
                        sfn_to_name(entry, &mut name)
                    };

                    if name_len > 0 && ascii_eq_ci(&name[..name_len], target) {
                        let cluster_hi = read_u16_le(entry, 20);
                        let cluster_lo = read_u16_le(entry, 26);
                        let cluster_val = ((cluster_hi as u32) << 16) | cluster_lo as u32;
                        let size = read_u32_le(entry, 28);
                        return Some((
                            offset + pos,
                            DirEntry {
                                name,
                                name_len,
                                is_dir: (attr & 0x10) != 0,
                                cluster: cluster_val,
                                size,
                            },
                        ));
                    }
                }

                clear_lfn(&mut lfn_name, &mut lfn_len, &mut lfn_active);
                pos += 32;
            }
            let next = self.fat_entry(cluster);
            if next < 2 || next >= FAT32_EOC {
                break;
            }
            cluster = next;
        }
        None
    }
}

pub unsafe fn overwrite_existing_file(range: ModuleRange, dirs: &[&[u8]], file: &[u8], input: &[u8]) -> bool {
    let (volume_offset, _) = match locate_fat32_volume_range(range) {
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
    let (entry_off, entry) = match fs.find_entry_offset(cluster, file) {
        Some(v) => v,
        None => return false,
    };
    if entry.is_dir || entry.cluster < 2 {
        return false;
    }

    let cluster_size = fs.cluster_size();
    let mut capacity = 0usize;
    let mut fat_cluster = entry.cluster;
    while fat_cluster >= 2 && fat_cluster < FAT32_EOC {
        capacity = capacity.saturating_add(cluster_size);
        let next = fs.fat_entry(fat_cluster);
        if next < 2 || next >= FAT32_EOC {
            break;
        }
        fat_cluster = next;
    }
    if input.len() > capacity {
        return false;
    }

    let raw_len = range.end.saturating_sub(range.start);
    let raw = core::slice::from_raw_parts_mut(range.start as *mut u8, raw_len);
    let mut src_pos = 0usize;
    let mut fat_cluster = entry.cluster;
    while fat_cluster >= 2 && fat_cluster < FAT32_EOC {
        let rel = match fs.cluster_offset(fat_cluster) {
            Some(v) => v,
            None => return false,
        };
        let abs = volume_offset + rel;
        if abs + cluster_size > raw.len() {
            return false;
        }
        let chunk_cap = cluster_size;
        let write_len = input.len().saturating_sub(src_pos).min(chunk_cap);
        for i in 0..write_len {
            raw[abs + i] = input[src_pos + i];
        }
        for i in write_len..chunk_cap {
            raw[abs + i] = 0;
        }
        src_pos += write_len;
        let next = fs.fat_entry(fat_cluster);
        if next < 2 || next >= FAT32_EOC {
            break;
        }
        fat_cluster = next;
    }

    let abs_entry = volume_offset + entry_off;
    if abs_entry + 32 <= raw.len() {
        write_u32_le(raw, abs_entry + 28, input.len() as u32);
        sync_module_to_house_disk(range)
    } else {
        false
    }
}

pub unsafe fn copy_file_to_dir(
    range: ModuleRange,
    src_parent_cluster: u32,
    src_name: &[u8],
    dst_parent_cluster: u32,
) -> bool {
    let (volume_offset, _) = match locate_fat32_volume_range(range) {
        Some(v) => v,
        None => return false,
    };
    let fs = match Fat32::new(range) {
        Some(v) => v,
        None => return false,
    };
    let (_, src_entry) = match fs.find_entry_offset(src_parent_cluster, src_name) {
        Some(v) => v,
        None => return false,
    };
    if src_entry.is_dir {
        return false;
    }

    let dst_entry_off = match find_free_dir_entry(&fs, dst_parent_cluster) {
        Some(v) => v,
        None => return false,
    };
    let mut sfn = [b' '; 11];
    if !build_unique_sfn(&fs, dst_parent_cluster, &src_entry.name[..src_entry.name_len], &mut sfn) {
        return false;
    }

    let cluster_size = fs.cluster_size();
    let needed = if src_entry.size == 0 {
        0usize
    } else {
        ((src_entry.size as usize) + cluster_size - 1) / cluster_size
    };
    if needed > MAX_COPY_CLUSTERS {
        return false;
    }

    let mut dst_clusters = [0u32; MAX_COPY_CLUSTERS];
    if needed > 0 && find_free_clusters(&fs, needed, &mut dst_clusters) < needed {
        return false;
    }

    let raw_len = range.end.saturating_sub(range.start);
    let raw = core::slice::from_raw_parts_mut(range.start as *mut u8, raw_len);

    for i in 0..needed {
        let next = if i + 1 < needed {
            dst_clusters[i + 1]
        } else {
            FAT32_EOC
        };
        if !write_fat_entry_all(raw, volume_offset, &fs, dst_clusters[i], next) {
            return false;
        }
    }

    let mut src_cluster = src_entry.cluster;
    let mut copied = 0usize;
    for i in 0..needed {
        if src_cluster < 2 || src_cluster >= FAT32_EOC {
            return false;
        }
        let src_rel = match fs.cluster_offset(src_cluster) {
            Some(v) => v,
            None => return false,
        };
        let dst_rel = match fs.cluster_offset(dst_clusters[i]) {
            Some(v) => v,
            None => return false,
        };
        let src_abs = volume_offset + src_rel;
        let dst_abs = volume_offset + dst_rel;
        if src_abs + cluster_size > raw.len() || dst_abs + cluster_size > raw.len() {
            return false;
        }
        let remain = (src_entry.size as usize).saturating_sub(copied);
        let take = remain.min(cluster_size);
        for j in 0..take {
            raw[dst_abs + j] = raw[src_abs + j];
        }
        for j in take..cluster_size {
            raw[dst_abs + j] = 0;
        }
        copied += take;
        let next = fs.fat_entry(src_cluster);
        if next < 2 || next >= FAT32_EOC {
            break;
        }
        src_cluster = next;
    }

    let abs_entry = volume_offset + dst_entry_off;
    if abs_entry + 32 > raw.len() {
        return false;
    }
    for i in 0..32 {
        raw[abs_entry + i] = 0;
    }
    for i in 0..11 {
        raw[abs_entry + i] = sfn[i];
    }
    raw[abs_entry + 11] = 0x20;
    let first_cluster = if needed > 0 { dst_clusters[0] } else { 0 };
    write_u16_le(raw, abs_entry + 20, (first_cluster >> 16) as u16);
    write_u16_le(raw, abs_entry + 26, (first_cluster & 0xFFFF) as u16);
    write_u32_le(raw, abs_entry + 28, src_entry.size);
    sync_module_to_house_disk(range)
}

pub unsafe fn mark_deleted(range: ModuleRange, parent_cluster: u32, name: &[u8]) -> bool {
    let (volume_offset, _) = match locate_fat32_volume_range(range) {
        Some(v) => v,
        None => return false,
    };
    let fs = match Fat32::new(range) {
        Some(v) => v,
        None => return false,
    };
    let (entry_off, _) = match fs.find_entry_offset(parent_cluster, name) {
        Some(v) => v,
        None => return false,
    };
    let raw_len = range.end.saturating_sub(range.start);
    let raw = core::slice::from_raw_parts_mut(range.start as *mut u8, raw_len);
    let abs_entry = volume_offset + entry_off;
    if abs_entry < raw.len() {
        raw[abs_entry] = 0xE5;
        sync_module_to_house_disk(range)
    } else {
        false
    }
}

fn find_free_dir_entry(fs: &Fat32, start_cluster: u32) -> Option<usize> {
    let mut cluster = start_cluster;
    if cluster < 2 {
        return None;
    }
    let cluster_size = fs.cluster_size();
    loop {
        let offset = fs.cluster_offset(cluster)?;
        let end = offset + cluster_size;
        if end > fs.data.len() {
            return None;
        }
        let buf = &fs.data[offset..end];
        let mut pos = 0usize;
        while pos + 32 <= buf.len() {
            let first = buf[pos];
            if first == 0x00 || first == 0xE5 {
                return Some(offset + pos);
            }
            pos += 32;
        }
        let next = fs.fat_entry(cluster);
        if next < 2 || next >= FAT32_EOC {
            return None;
        }
        cluster = next;
    }
}

fn find_free_clusters(fs: &Fat32, needed: usize, out: &mut [u32; MAX_COPY_CLUSTERS]) -> usize {
    let mut count = 0usize;
    let max_cluster = fs.total_clusters.saturating_add(2);
    let mut cluster = 2u32;
    while cluster < max_cluster && count < needed && count < out.len() {
        if fs.fat_entry(cluster) == 0 {
            out[count] = cluster;
            count += 1;
        }
        cluster += 1;
    }
    count
}

fn write_fat_entry_all(raw: &mut [u8], volume_offset: usize, fs: &Fat32, cluster: u32, value: u32) -> bool {
    let val = value & 0x0FFF_FFFF;
    for fat in 0..fs.fats {
        let off = volume_offset
            + fs.fat_start
            + fat.saturating_mul(fs.fat_size).saturating_mul(fs.bytes_per_sector)
            + (cluster as usize).saturating_mul(4);
        if off + 4 > raw.len() {
            return false;
        }
        write_u32_le(raw, off, val);
    }
    true
}

fn build_unique_sfn(fs: &Fat32, dst_parent_cluster: u32, src_name: &[u8], out: &mut [u8; 11]) -> bool {
    if build_sfn_from_name(src_name, out) {
        let mut display = [0u8; MAX_NAME];
        let display_len = sfn_to_name(out, &mut display);
        if display_len > 0 && fs.find_entry_offset(dst_parent_cluster, &display[..display_len]).is_none() {
            return true;
        }
    }

    let mut ext = [b' '; 3];
    extract_ext(src_name, &mut ext);
    for n in 1..10000u32 {
        for b in out.iter_mut() {
            *b = b' ';
        }
        out[0] = b'C';
        out[1] = b'O';
        out[2] = b'P';
        out[3] = b'Y';
        let digits = write_decimal_into(&mut out[4..8], n);
        if digits == 0 {
            return false;
        }
        out[8] = ext[0];
        out[9] = ext[1];
        out[10] = ext[2];
        let mut display = [0u8; MAX_NAME];
        let display_len = sfn_to_name(out, &mut display);
        if display_len > 0 && fs.find_entry_offset(dst_parent_cluster, &display[..display_len]).is_none() {
            return true;
        }
    }
    false
}

fn build_sfn_from_name(name: &[u8], out: &mut [u8; 11]) -> bool {
    for b in out.iter_mut() {
        *b = b' ';
    }
    let mut dot = usize::MAX;
    for i in 0..name.len() {
        if name[i] == b'.' {
            dot = i;
        }
    }
    let base_end = if dot == usize::MAX { name.len() } else { dot };
    if base_end == 0 || base_end > 8 {
        return false;
    }
    for i in 0..base_end {
        let ch = sfn_char(name[i]);
        if ch == 0 {
            return false;
        }
        out[i] = ch;
    }
    if dot != usize::MAX {
        let ext = &name[dot + 1..];
        if ext.is_empty() || ext.len() > 3 {
            return false;
        }
        for i in 0..ext.len() {
            let ch = sfn_char(ext[i]);
            if ch == 0 {
                return false;
            }
            out[8 + i] = ch;
        }
    }
    true
}

fn extract_ext(name: &[u8], out: &mut [u8; 3]) {
    out[0] = b' ';
    out[1] = b' ';
    out[2] = b' ';
    let mut dot = usize::MAX;
    for i in 0..name.len() {
        if name[i] == b'.' {
            dot = i;
        }
    }
    if dot == usize::MAX {
        return;
    }
    let ext = &name[dot + 1..];
    for i in 0..ext.len().min(3) {
        let ch = sfn_char(ext[i]);
        if ch == 0 {
            return;
        }
        out[i] = ch;
    }
}

fn sfn_char(ch: u8) -> u8 {
    let up = if ch >= b'a' && ch <= b'z' { ch - 32 } else { ch };
    if (up >= b'A' && up <= b'Z') || (up >= b'0' && up <= b'9') || up == b'_' || up == b'-' {
        up
    } else {
        0
    }
}

fn write_decimal_into(out: &mut [u8], mut val: u32) -> usize {
    if out.is_empty() || val == 0 {
        return 0;
    }
    let mut tmp = [0u8; 10];
    let mut n = 0usize;
    while val > 0 && n < tmp.len() {
        tmp[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    if n > out.len() {
        return 0;
    }
    let mut written = 0usize;
    while n > 0 {
        n -= 1;
        out[written] = tmp[n];
        written += 1;
    }
    written
}

fn sync_module_to_house_disk(range: ModuleRange) -> bool {
    let raw_len = range.end.saturating_sub(range.start);
    if raw_len < 512 {
        return false;
    }
    let raw = unsafe { core::slice::from_raw_parts(range.start as *const u8, raw_len) };
    let drive_index = match find_houseos_drive() {
        Some(v) => v,
        None => return false,
    };
    let sectors = (raw_len + 511) / 512;
    for sector in 0..sectors {
        let mut sector_buf = [0u8; 512];
        let start = sector * 512;
        let end = (start + 512).min(raw_len);
        if start < end {
            sector_buf[..end - start].copy_from_slice(&raw[start..end]);
        }
        if !ata_pio::write_sector(drive_index, sector as u64, &sector_buf) {
            return false;
        }
    }
    true
}

fn find_houseos_drive() -> Option<usize> {
    let drives = ata_pio::drives();
    let mut mbr = [0u8; 512];
    let mut boot = [0u8; 512];
    for drive_index in 0..drives.len() {
        if !ata_pio::read_sector(drive_index, 0, &mut mbr) {
            continue;
        }
        if mbr[510] != 0x55 || mbr[511] != 0xAA {
            continue;
        }
        for part in 0..4 {
            let entry = 446 + part * 16;
            let part_type = mbr[entry + 4];
            if !is_fat32_partition_type(part_type) {
                continue;
            }
            let lba_start = read_u32_le(&mbr, entry + 8) as u64;
            if lba_start == 0 {
                continue;
            }
            if !ata_pio::read_sector(drive_index, lba_start, &mut boot) {
                continue;
            }
            if is_fat32_boot_sector(&boot) && fat_label_matches(&boot, b"HOUSEOS") {
                return Some(drive_index);
            }
        }
    }
    None
}

fn fat_label_matches(boot: &[u8], label: &[u8]) -> bool {
    if boot.len() < 82 {
        return false;
    }
    for i in 0..label.len().min(11) {
        if boot[71 + i] != label[i] {
            return false;
        }
    }
    true
}

fn locate_fat32_volume<'a>(data: &'a [u8]) -> Option<&'a [u8]> {
    if is_fat32_boot_sector(data) {
        return Some(data);
    }
    if data.len() < 512 || data[510] != 0x55 || data[511] != 0xAA {
        return None;
    }

    for i in 0..4 {
        let entry = 446 + i * 16;
        let part_type = data[entry + 4];
        if !is_fat32_partition_type(part_type) {
            continue;
        }
        let lba_start = read_u32_le(data, entry + 8) as usize;
        let sector_count = read_u32_le(data, entry + 12) as usize;
        if lba_start == 0 || sector_count == 0 {
            continue;
        }
        let offset = lba_start.saturating_mul(512);
        let byte_len = sector_count.saturating_mul(512);
        let end = offset.saturating_add(byte_len);
        if offset >= data.len() || end > data.len() {
            continue;
        }
        let volume = &data[offset..end];
        if is_fat32_boot_sector(volume) {
            return Some(volume);
        }
    }
    None
}

fn locate_fat32_volume_range(range: ModuleRange) -> Option<(usize, usize)> {
    let len = range.end.saturating_sub(range.start);
    if len < 512 {
        return None;
    }
    let data = unsafe { core::slice::from_raw_parts(range.start as *const u8, len) };
    if is_fat32_boot_sector(data) {
        return Some((0, data.len()));
    }
    if data[510] != 0x55 || data[511] != 0xAA {
        return None;
    }

    for i in 0..4 {
        let entry = 446 + i * 16;
        let part_type = data[entry + 4];
        if !is_fat32_partition_type(part_type) {
            continue;
        }
        let lba_start = read_u32_le(data, entry + 8) as usize;
        let sector_count = read_u32_le(data, entry + 12) as usize;
        if lba_start == 0 || sector_count == 0 {
            continue;
        }
        let offset = lba_start.saturating_mul(512);
        let byte_len = sector_count.saturating_mul(512);
        let end = offset.saturating_add(byte_len);
        if offset >= data.len() || end > data.len() {
            continue;
        }
        if is_fat32_boot_sector(&data[offset..end]) {
            return Some((offset, byte_len));
        }
    }
    None
}

fn is_fat32_partition_type(part_type: u8) -> bool {
    part_type == 0x0B || part_type == 0x0C || part_type == 0x1B || part_type == 0x1C
}

fn is_fat32_boot_sector(buf: &[u8]) -> bool {
    if buf.len() < 512 || buf[510] != 0x55 || buf[511] != 0xAA {
        return false;
    }
    let bps = read_u16_le(buf, 11);
    if bps != 512 {
        return false;
    }
    if buf[13] == 0 || buf[16] == 0 {
        return false;
    }
    let fat_size = read_u16_le(buf, 22) as u32 + read_u32_le(buf, 36);
    if fat_size == 0 {
        return false;
    }
    let root_cluster = read_u32_le(buf, 44);
    if root_cluster < 2 {
        return false;
    }
    let fs_type = &buf[82..90];
    fs_type[0] == b'F' && fs_type[1] == b'A' && fs_type[2] == b'T'
}

fn read_u16_le(buf: &[u8], offset: usize) -> u16 {
    if offset + 2 > buf.len() {
        return 0;
    }
    (buf[offset] as u16) | ((buf[offset + 1] as u16) << 8)
}

fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    if offset + 4 > buf.len() {
        return 0;
    }
    (buf[offset] as u32)
        | ((buf[offset + 1] as u32) << 8)
        | ((buf[offset + 2] as u32) << 16)
        | ((buf[offset + 3] as u32) << 24)
}

fn write_u16_le(buf: &mut [u8], offset: usize, val: u16) {
    if offset + 2 > buf.len() {
        return;
    }
    buf[offset] = (val & 0xFF) as u8;
    buf[offset + 1] = ((val >> 8) & 0xFF) as u8;
}

fn write_u32_le(buf: &mut [u8], offset: usize, val: u32) {
    if offset + 4 > buf.len() {
        return;
    }
    buf[offset] = (val & 0xFF) as u8;
    buf[offset + 1] = ((val >> 8) & 0xFF) as u8;
    buf[offset + 2] = ((val >> 16) & 0xFF) as u8;
    buf[offset + 3] = ((val >> 24) & 0xFF) as u8;
}

fn collect_lfn(entry: &[u8], out: &mut [u8; MAX_NAME], len: &mut usize, active: &mut bool) {
    let order = entry[0] & 0x1F;
    if order == 0 {
        clear_lfn(out, len, active);
        return;
    }
    if (entry[0] & 0x40) != 0 {
        clear_name(out);
        *len = 0;
        *active = true;
    } else if !*active {
        *active = true;
    }

    let base = (order as usize).saturating_sub(1).saturating_mul(13);
    let offsets = [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
    for i in 0..offsets.len() {
        let ch = read_u16_le(entry, offsets[i]);
        if ch == 0x0000 {
            break;
        }
        if ch == 0xFFFF {
            continue;
        }
        let dest = base + i;
        if dest < MAX_NAME {
            out[dest] = if ch < 0x80 { ch as u8 } else { b'?' };
            if dest + 1 > *len {
                *len = dest + 1;
            }
        }
    }
}

fn clear_lfn(out: &mut [u8; MAX_NAME], len: &mut usize, active: &mut bool) {
    if *active {
        clear_name(out);
    }
    *len = 0;
    *active = false;
}

fn clear_name(out: &mut [u8; MAX_NAME]) {
    for b in out.iter_mut() {
        *b = 0;
    }
}

fn sfn_to_name(entry: &[u8], out: &mut [u8; MAX_NAME]) -> usize {
    let mut base_end = 8usize;
    while base_end > 0 && entry[base_end - 1] == b' ' {
        base_end -= 1;
    }
    let mut ext_end = 3usize;
    while ext_end > 0 && entry[8 + ext_end - 1] == b' ' {
        ext_end -= 1;
    }
    if base_end == 0 {
        return 0;
    }
    let mut len = 0usize;
    for i in 0..base_end {
        let mut b = entry[i];
        if b == 0x05 {
            b = 0xE5;
        }
        if len < out.len() {
            out[len] = b;
            len += 1;
        }
    }
    if ext_end > 0 && len + 1 < out.len() {
        out[len] = b'.';
        len += 1;
        for i in 0..ext_end {
            if len < out.len() {
                out[len] = entry[8 + i];
                len += 1;
            }
        }
    }
    len
}

fn is_dot_entry(name: &[u8; MAX_NAME], len: usize) -> bool {
    (len == 1 && name[0] == b'.') || (len == 2 && name[0] == b'.' && name[1] == b'.')
}

fn ascii_eq_ci(a: &[u8], b: &[u8]) -> bool {
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
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
}
