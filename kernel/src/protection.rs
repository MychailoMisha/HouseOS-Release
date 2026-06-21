use crate::drivers::{battery, net, pci, usb};
use crate::memory;
use crate::system;

pub const REPORT_LINES: usize = 8;
pub const REPORT_COLS: usize = 64;

#[derive(Copy, Clone)]
pub struct ScanReport {
    pub lines: [[u8; REPORT_COLS]; REPORT_LINES],
    pub lens: [usize; REPORT_LINES],
    pub count: usize,
    pub warnings: usize,
}

impl ScanReport {
    pub const EMPTY: ScanReport = ScanReport {
        lines: [[0u8; REPORT_COLS]; REPORT_LINES],
        lens: [0; REPORT_LINES],
        count: 0,
        warnings: 0,
    };

    fn push(&mut self, line: &[u8]) {
        if self.count >= REPORT_LINES {
            return;
        }
        let len = line.len().min(REPORT_COLS);
        self.lines[self.count][..len].copy_from_slice(&line[..len]);
        self.lens[self.count] = len;
        self.count += 1;
    }

    fn push_buf(&mut self, buf: &[u8], len: usize) {
        self.push(&buf[..len.min(buf.len())]);
    }

    fn warn(&mut self, line: &[u8]) {
        self.warnings = self.warnings.saturating_add(1);
        self.push(line);
    }
}

static mut PROTECTION_ENABLED: bool = true;
static mut LAST_SCAN: ScanReport = ScanReport::EMPTY;
static mut HAS_SCAN: bool = false;

pub fn set_enabled(on: bool) {
    unsafe {
        PROTECTION_ENABLED = on;
    }
    memory::set_memory_protection(on);
}

pub fn enabled() -> bool {
    unsafe { PROTECTION_ENABLED }
}

pub fn last_scan() -> Option<ScanReport> {
    unsafe {
        if HAS_SCAN {
            Some(LAST_SCAN)
        } else {
            None
        }
    }
}

pub fn scan_targets() -> ScanReport {
    let mut report = ScanReport::EMPTY;
    report.push(b"Scan targets:");
    report.push(b"kernel runtime + heap guard");
    report.push(b"PCI, USB, NET, battery/power");
    report.push(b"framebuffer + system info");
    report.push(b"HouseOS disk is checked by FAT32 mount");
    report
}

pub fn run_scan() -> ScanReport {
    let mut report = ScanReport::EMPTY;
    report.push(b"HouseOS integrity scan");

    let info = system::system_info();
    let mut display = [0u8; REPORT_COLS];
    let mut pos = 0usize;
    pos += write_str(&mut display[pos..], b"Display ");
    pos += write_u32(&mut display[pos..], info.fb_w as u32);
    pos += write_str(&mut display[pos..], b"x");
    pos += write_u32(&mut display[pos..], info.fb_h as u32);
    pos += write_str(&mut display[pos..], b" ");
    pos += write_u32(&mut display[pos..], info.fb_bpp as u32);
    pos += write_str(&mut display[pos..], b"bpp OK");
    report.push_buf(&display, pos);

    let pci_devices = pci::scan();
    let mut pci_line = [0u8; REPORT_COLS];
    let mut pos = 0usize;
    pos += write_str(&mut pci_line[pos..], b"PCI devices: ");
    pos += write_u32(&mut pci_line[pos..], pci_devices.len() as u32);
    report.push_buf(&pci_line, pos);

    let usb_count = usb::controllers().len();
    let mut usb_line = [0u8; REPORT_COLS];
    let mut pos = 0usize;
    pos += write_str(&mut usb_line[pos..], b"USB controllers: ");
    pos += write_u32(&mut usb_line[pos..], usb_count as u32);
    if usb_count == 0 {
        report.warn(&usb_line[..pos]);
    } else {
        report.push_buf(&usb_line, pos);
    }

    let net_devs = net::devices();
    if net_devs.is_empty() {
        report.warn(b"NET: no adapter detected");
    } else {
        let dev = net_devs[0];
        if dev.driver_online {
            report.push(b"NET: adapter ready");
        } else {
            report.warn(b"NET: adapter detected, driver offline");
        }
    }

    battery::update();
    let mut power_line = [0u8; REPORT_COLS];
    let mut pos = 0usize;
    pos += write_str(&mut power_line[pos..], b"Power: ");
    if battery::has_battery() {
        pos += write_u32(&mut power_line[pos..], battery::get_level() as u32);
        pos += write_str(&mut power_line[pos..], b"% battery");
    } else {
        pos += write_str(&mut power_line[pos..], b"voltage sensor unavailable");
    }
    report.push_buf(&power_line, pos);

    let (used, total, allocs, blocked) = memory::get_memory_stats();
    let mut mem_line = [0u8; REPORT_COLS];
    let mut pos = 0usize;
    pos += write_str(&mut mem_line[pos..], b"RAM guard ");
    pos += write_str(
        &mut mem_line[pos..],
        if memory::memory_protection_enabled() { b"ON " } else { b"OFF " },
    );
    pos += write_u32(&mut mem_line[pos..], (used / 1024 / 1024) as u32);
    pos += write_str(&mut mem_line[pos..], b"/");
    pos += write_u32(&mut mem_line[pos..], (total / 1024 / 1024) as u32);
    pos += write_str(&mut mem_line[pos..], b"MiB a=");
    pos += write_u32(&mut mem_line[pos..], allocs as u32);
    pos += write_str(&mut mem_line[pos..], b" b=");
    pos += write_u32(&mut mem_line[pos..], blocked as u32);
    report.push_buf(&mem_line, pos);

    if report.warnings == 0 {
        report.push(b"Result: OK");
    } else {
        let mut line = [0u8; REPORT_COLS];
        let mut pos = 0usize;
        pos += write_str(&mut line[pos..], b"Result: WARN ");
        pos += write_u32(&mut line[pos..], report.warnings as u32);
        report.push_buf(&line, pos);
    }

    unsafe {
        LAST_SCAN = report;
        HAS_SCAN = true;
    }
    report
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
