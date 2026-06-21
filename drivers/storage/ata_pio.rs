#![allow(dead_code)]

use crate::drivers::port_io::{inb, inw, io_wait, outb, outw};

const MAX_DRIVES: usize = 4;
const MAX_PARTITIONS: usize = 16;
const SECTOR_SIZE: usize = 512;

const DATA: u16 = 0;
const SECTOR_COUNT: u16 = 2;
const LBA_LOW: u16 = 3;
const LBA_MID: u16 = 4;
const LBA_HIGH: u16 = 5;
const DRIVE_HEAD: u16 = 6;
const STATUS_CMD: u16 = 7;

const STATUS_ERR: u8 = 0x01;
const STATUS_DRQ: u8 = 0x08;
const STATUS_SRV: u8 = 0x10;
const STATUS_DF: u8 = 0x20;
const STATUS_RDY: u8 = 0x40;
const STATUS_BSY: u8 = 0x80;

const CMD_IDENTIFY: u8 = 0xEC;
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_READ_SECTORS_EXT: u8 = 0x24;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_WRITE_SECTORS_EXT: u8 = 0x34;
const CMD_CACHE_FLUSH: u8 = 0xE7;
const CMD_CACHE_FLUSH_EXT: u8 = 0xEA;

#[derive(Copy, Clone)]
pub struct AtaDrive {
    pub present: bool,
    pub channel: usize,
    pub slave: bool,
    pub sectors: u64,
    pub supports_lba48: bool,
}

#[derive(Copy, Clone)]
pub struct MbrPartition {
    pub present: bool,
    pub drive_index: usize,
    pub index: usize,
    pub gpt: bool,
    pub bootable: bool,
    pub part_type: u8,
    pub lba_start: u64,
    pub sectors: u64,
    pub name: [u8; 32],
    pub name_len: usize,
}

impl MbrPartition {
    const EMPTY: MbrPartition = MbrPartition {
        present: false,
        drive_index: 0,
        index: 0,
        gpt: false,
        bootable: false,
        part_type: 0,
        lba_start: 0,
        sectors: 0,
        name: [0; 32],
        name_len: 0,
    };
}

impl AtaDrive {
    const EMPTY: AtaDrive = AtaDrive {
        present: false,
        channel: 0,
        slave: false,
        sectors: 0,
        supports_lba48: false,
    };
}

#[derive(Copy, Clone)]
struct AtaChannel {
    io: u16,
    ctrl: u16,
}

const CHANNELS: [AtaChannel; 2] = [
    AtaChannel { io: 0x1F0, ctrl: 0x3F6 },
    AtaChannel { io: 0x170, ctrl: 0x376 },
];

static mut DRIVES: [AtaDrive; MAX_DRIVES] = [AtaDrive::EMPTY; MAX_DRIVES];
static mut PARTITIONS: [MbrPartition; MAX_PARTITIONS] = [MbrPartition::EMPTY; MAX_PARTITIONS];
static mut DRIVE_COUNT: usize = 0;
static mut PARTITION_COUNT: usize = 0;
static mut SCANNED: bool = false;

pub fn init() -> usize {
    unsafe {
        DRIVE_COUNT = 0;
        PARTITION_COUNT = 0;
        SCANNED = false;
        for channel in 0..CHANNELS.len() {
            for slave in 0..2 {
                if let Some(drive) = identify(channel, slave != 0) {
                    if DRIVE_COUNT < MAX_DRIVES {
                        DRIVES[DRIVE_COUNT] = drive;
                        DRIVE_COUNT += 1;
                    }
                }
            }
        }
        SCANNED = true;
    }
    scan_partitions();
    unsafe { DRIVE_COUNT }
}

pub fn rescan() -> usize {
    init()
}

pub fn drives() -> &'static [AtaDrive] {
    unsafe {
        if !SCANNED {
            init();
        }
        &DRIVES[..DRIVE_COUNT]
    }
}

pub fn partitions() -> &'static [MbrPartition] {
    unsafe {
        if !SCANNED {
            init();
        }
        &PARTITIONS[..PARTITION_COUNT]
    }
}

pub fn read_sector(drive_index: usize, lba: u64, out: &mut [u8]) -> bool {
    read_sectors(drive_index, lba, 1, out)
}

pub fn write_sector(drive_index: usize, lba: u64, data: &[u8]) -> bool {
    if data.len() < SECTOR_SIZE {
        return false;
    }
    let drive = unsafe {
        if !SCANNED {
            init();
        }
        if drive_index >= DRIVE_COUNT {
            return false;
        }
        DRIVES[drive_index]
    };
    if !drive.present || lba >= drive.sectors {
        return false;
    }
    let ok = if drive.supports_lba48 || lba > 0x0FFF_FFFF {
        write_lba48_one(drive, lba, data)
    } else {
        write_lba28_one(drive, lba as u32, data)
    };
    if ok {
        flush_cache(drive)
    } else {
        false
    }
}

pub fn read_sectors(drive_index: usize, lba: u64, count: u16, out: &mut [u8]) -> bool {
    if count == 0 || out.len() < count as usize * SECTOR_SIZE {
        return false;
    }
    let drive = unsafe {
        if !SCANNED {
            init();
        }
        if drive_index >= DRIVE_COUNT {
            return false;
        }
        DRIVES[drive_index]
    };
    if !drive.present || lba.saturating_add(count as u64) > drive.sectors {
        return false;
    }
    if drive.supports_lba48 {
        read_lba48(drive, lba, count, out)
    } else {
        if lba > 0x0FFF_FFFF || count > 255 {
            return false;
        }
        read_lba28(drive, lba as u32, count as u8, out)
    }
}

fn scan_partitions() {
    let drive_count = unsafe { DRIVE_COUNT };
    for drive_index in 0..drive_count {
        let mut sector = [0u8; SECTOR_SIZE];
        if !read_sector(drive_index, 0, &mut sector) {
            continue;
        }
        if has_protective_mbr(&sector) && parse_gpt(drive_index) {
            continue;
        }
        parse_mbr(drive_index, &sector);
    }
}

fn has_protective_mbr(sector: &[u8; SECTOR_SIZE]) -> bool {
    if sector[510] != 0x55 || sector[511] != 0xAA {
        return false;
    }
    for index in 0..4 {
        if sector[446 + index * 16 + 4] == 0xEE {
            return true;
        }
    }
    false
}

fn parse_gpt(drive_index: usize) -> bool {
    let mut header = [0u8; SECTOR_SIZE];
    if !read_sector(drive_index, 1, &mut header) {
        return false;
    }
    if &header[0..8] != b"EFI PART" {
        return false;
    }
    let entries_lba = read_u64_le(&header[72..80]);
    let entry_count = read_u32_le(&header[80..84]) as usize;
    let entry_size = read_u32_le(&header[84..88]) as usize;
    if entries_lba == 0 || entry_count == 0 || entry_size < 128 || entry_size > 512 {
        return false;
    }
    let mut sector = [0u8; SECTOR_SIZE];
    let mut found = false;
    let max_entries = entry_count.min(128);
    for index in 0..max_entries {
        let byte_off = index * entry_size;
        let lba = entries_lba + (byte_off / SECTOR_SIZE) as u64;
        let off = byte_off % SECTOR_SIZE;
        if off + entry_size > SECTOR_SIZE {
            continue;
        }
        if !read_sector(drive_index, lba, &mut sector) {
            break;
        }
        let entry = &sector[off..off + entry_size];
        if entry[0..16].iter().all(|&b| b == 0) {
            continue;
        }
        let first_lba = read_u64_le(&entry[32..40]);
        let last_lba = read_u64_le(&entry[40..48]);
        if first_lba == 0 || last_lba < first_lba {
            continue;
        }
        let mut name = [0u8; 32];
        let name_len = utf16le_ascii(&entry[56..entry_size.min(128)], &mut name);
        unsafe {
            if PARTITION_COUNT >= MAX_PARTITIONS {
                return found;
            }
            PARTITIONS[PARTITION_COUNT] = MbrPartition {
                present: true,
                drive_index,
                index,
                gpt: true,
                bootable: false,
                part_type: 0xEE,
                lba_start: first_lba,
                sectors: last_lba - first_lba + 1,
                name,
                name_len,
            };
            PARTITION_COUNT += 1;
        }
        found = true;
    }
    found
}

fn parse_mbr(drive_index: usize, sector: &[u8; SECTOR_SIZE]) {
    if sector[510] != 0x55 || sector[511] != 0xAA {
        return;
    }
    for index in 0..4 {
        let off = 446 + index * 16;
        let part_type = sector[off + 4];
        let lba_start = read_u32_le(&sector[off + 8..off + 12]) as u64;
        let sectors = read_u32_le(&sector[off + 12..off + 16]) as u64;
        if part_type == 0 || sectors == 0 {
            continue;
        }
        unsafe {
            if PARTITION_COUNT >= MAX_PARTITIONS {
                return;
            }
            PARTITIONS[PARTITION_COUNT] = MbrPartition {
                present: true,
                drive_index,
                index,
                gpt: false,
                bootable: sector[off] == 0x80,
                part_type,
                lba_start,
                sectors,
                name: [0; 32],
                name_len: 0,
            };
            PARTITION_COUNT += 1;
        }
    }
}

fn identify(channel: usize, slave: bool) -> Option<AtaDrive> {
    let ch = CHANNELS[channel];
    unsafe {
        outb(ch.ctrl, 0);
        outb(ch.io + DRIVE_HEAD, 0xA0 | slave_bit(slave));
        ata_delay(ch);
        outb(ch.io + SECTOR_COUNT, 0);
        outb(ch.io + LBA_LOW, 0);
        outb(ch.io + LBA_MID, 0);
        outb(ch.io + LBA_HIGH, 0);
        outb(ch.io + STATUS_CMD, CMD_IDENTIFY);
        ata_delay(ch);

        let mut status = inb(ch.io + STATUS_CMD);
        if status == 0 {
            return None;
        }
        if !wait_not_busy(ch) {
            return None;
        }

        let mid = inb(ch.io + LBA_MID);
        let high = inb(ch.io + LBA_HIGH);
        if mid != 0 || high != 0 {
            return None;
        }

        status = inb(ch.io + STATUS_CMD);
        while (status & STATUS_DRQ) == 0 {
            if (status & (STATUS_ERR | STATUS_DF)) != 0 {
                return None;
            }
            status = inb(ch.io + STATUS_CMD);
        }

        let mut words = [0u16; 256];
        for i in 0..256 {
            words[i] = inw(ch.io + DATA);
        }

        let sectors28 = (words[60] as u64) | ((words[61] as u64) << 16);
        let lba48 = (words[83] & (1 << 10)) != 0;
        let sectors48 = (words[100] as u64)
            | ((words[101] as u64) << 16)
            | ((words[102] as u64) << 32)
            | ((words[103] as u64) << 48);
        let sectors = if lba48 && sectors48 > 0 {
            sectors48
        } else {
            sectors28
        };
        if sectors == 0 {
            return None;
        }

        Some(AtaDrive {
            present: true,
            channel,
            slave,
            sectors,
            supports_lba48: lba48,
        })
    }
}

fn read_lba28(drive: AtaDrive, lba: u32, count: u8, out: &mut [u8]) -> bool {
    let ch = CHANNELS[drive.channel];
    unsafe {
        if !select_drive(ch, drive.slave) {
            return false;
        }
        outb(ch.io + DRIVE_HEAD, 0xE0 | slave_bit(drive.slave) | (((lba >> 24) & 0x0F) as u8));
        outb(ch.io + SECTOR_COUNT, count);
        outb(ch.io + LBA_LOW, (lba & 0xFF) as u8);
        outb(ch.io + LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ch.io + LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(ch.io + STATUS_CMD, CMD_READ_SECTORS);
    }
    read_pio_data(ch, count as usize, out)
}

fn read_lba48(drive: AtaDrive, lba: u64, count: u16, out: &mut [u8]) -> bool {
    let ch = CHANNELS[drive.channel];
    unsafe {
        if !select_drive(ch, drive.slave) {
            return false;
        }
        outb(ch.io + DRIVE_HEAD, 0x40 | slave_bit(drive.slave));
        outb(ch.io + SECTOR_COUNT, (count >> 8) as u8);
        outb(ch.io + LBA_LOW, ((lba >> 24) & 0xFF) as u8);
        outb(ch.io + LBA_MID, ((lba >> 32) & 0xFF) as u8);
        outb(ch.io + LBA_HIGH, ((lba >> 40) & 0xFF) as u8);
        outb(ch.io + SECTOR_COUNT, (count & 0xFF) as u8);
        outb(ch.io + LBA_LOW, (lba & 0xFF) as u8);
        outb(ch.io + LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ch.io + LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(ch.io + STATUS_CMD, CMD_READ_SECTORS_EXT);
    }
    read_pio_data(ch, count as usize, out)
}

fn write_lba28_one(drive: AtaDrive, lba: u32, data: &[u8]) -> bool {
    let ch = CHANNELS[drive.channel];
    unsafe {
        if !select_drive(ch, drive.slave) {
            return false;
        }
        outb(ch.io + DRIVE_HEAD, 0xE0 | slave_bit(drive.slave) | (((lba >> 24) & 0x0F) as u8));
        outb(ch.io + SECTOR_COUNT, 1);
        outb(ch.io + LBA_LOW, (lba & 0xFF) as u8);
        outb(ch.io + LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ch.io + LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(ch.io + STATUS_CMD, CMD_WRITE_SECTORS);
    }
    write_pio_sector(ch, data)
}

fn write_lba48_one(drive: AtaDrive, lba: u64, data: &[u8]) -> bool {
    let ch = CHANNELS[drive.channel];
    unsafe {
        if !select_drive(ch, drive.slave) {
            return false;
        }
        outb(ch.io + DRIVE_HEAD, 0x40 | slave_bit(drive.slave));
        outb(ch.io + SECTOR_COUNT, 0);
        outb(ch.io + LBA_LOW, ((lba >> 24) & 0xFF) as u8);
        outb(ch.io + LBA_MID, ((lba >> 32) & 0xFF) as u8);
        outb(ch.io + LBA_HIGH, ((lba >> 40) & 0xFF) as u8);
        outb(ch.io + SECTOR_COUNT, 1);
        outb(ch.io + LBA_LOW, (lba & 0xFF) as u8);
        outb(ch.io + LBA_MID, ((lba >> 8) & 0xFF) as u8);
        outb(ch.io + LBA_HIGH, ((lba >> 16) & 0xFF) as u8);
        outb(ch.io + STATUS_CMD, CMD_WRITE_SECTORS_EXT);
    }
    write_pio_sector(ch, data)
}

fn read_pio_data(ch: AtaChannel, sectors: usize, out: &mut [u8]) -> bool {
    for sector in 0..sectors {
        if !wait_data_ready(ch) {
            return false;
        }
        let base = sector * SECTOR_SIZE;
        for i in 0..256 {
            let word = unsafe { inw(ch.io + DATA) };
            out[base + i * 2] = (word & 0xFF) as u8;
            out[base + i * 2 + 1] = (word >> 8) as u8;
        }
        unsafe {
            ata_delay(ch);
        }
    }
    true
}

fn write_pio_sector(ch: AtaChannel, data: &[u8]) -> bool {
    if !wait_data_ready(ch) {
        return false;
    }
    for i in 0..256 {
        let lo = data[i * 2] as u16;
        let hi = data[i * 2 + 1] as u16;
        unsafe {
            outw(ch.io + DATA, lo | (hi << 8));
        }
    }
    unsafe {
        ata_delay(ch);
    }
    wait_not_busy(ch)
}

fn flush_cache(drive: AtaDrive) -> bool {
    let ch = CHANNELS[drive.channel];
    unsafe {
        if !select_drive(ch, drive.slave) {
            return false;
        }
        outb(ch.io + STATUS_CMD, if drive.supports_lba48 { CMD_CACHE_FLUSH_EXT } else { CMD_CACHE_FLUSH });
    }
    wait_not_busy(ch)
}

unsafe fn select_drive(ch: AtaChannel, slave: bool) -> bool {
    outb(ch.io + DRIVE_HEAD, 0xA0 | slave_bit(slave));
    ata_delay(ch);
    wait_not_busy(ch)
}

fn wait_not_busy(ch: AtaChannel) -> bool {
    for _ in 0..100_000 {
        let status = unsafe { inb(ch.io + STATUS_CMD) };
        if (status & STATUS_BSY) == 0 {
            return true;
        }
    }
    false
}

fn wait_data_ready(ch: AtaChannel) -> bool {
    for _ in 0..200_000 {
        let status = unsafe { inb(ch.io + STATUS_CMD) };
        if (status & (STATUS_ERR | STATUS_DF)) != 0 {
            return false;
        }
        if (status & STATUS_BSY) == 0 && (status & STATUS_DRQ) != 0 {
            return true;
        }
        if (status & (STATUS_RDY | STATUS_SRV)) == 0 {
            unsafe {
                io_wait();
            }
        }
    }
    false
}

unsafe fn ata_delay(ch: AtaChannel) {
    for _ in 0..4 {
        let _ = inb(ch.ctrl);
    }
}

fn slave_bit(slave: bool) -> u8 {
    if slave { 0x10 } else { 0 }
}

fn read_u32_le(buf: &[u8]) -> u32 {
    if buf.len() < 4 {
        return 0;
    }
    (buf[0] as u32)
        | ((buf[1] as u32) << 8)
        | ((buf[2] as u32) << 16)
        | ((buf[3] as u32) << 24)
}

fn read_u64_le(buf: &[u8]) -> u64 {
    if buf.len() < 8 {
        return 0;
    }
    (buf[0] as u64)
        | ((buf[1] as u64) << 8)
        | ((buf[2] as u64) << 16)
        | ((buf[3] as u64) << 24)
        | ((buf[4] as u64) << 32)
        | ((buf[5] as u64) << 40)
        | ((buf[6] as u64) << 48)
        | ((buf[7] as u64) << 56)
}

fn utf16le_ascii(src: &[u8], out: &mut [u8; 32]) -> usize {
    let mut len = 0usize;
    let mut i = 0usize;
    while i + 1 < src.len() && len < out.len() {
        let lo = src[i];
        let hi = src[i + 1];
        if lo == 0 && hi == 0 {
            break;
        }
        out[len] = if hi == 0 && lo >= 32 && lo < 127 { lo } else { b'?' };
        len += 1;
        i += 2;
    }
    len
}
