use crate::clipboard;
use crate::memory;
use crate::netstack;
use crate::protection;
use crate::rtc;
use crate::system;
use crate::drivers::battery;
use crate::drivers::storage::{ata_pio, exfat, ntfs};
use crate::drivers::usb;
use crate::drivers::audio::pc_speaker;

#[derive(Copy, Clone, PartialEq)]
pub enum ConsoleAction {
    None,
    OpenExplorer,
    OpenClipboard,
    OpenNotepad,
    OpenBrowser,
    OpenSecurity,
    Reboot,
    Shutdown,
}

#[derive(Copy, Clone, PartialEq)]
pub enum LineType {
    Normal,
    Success,
    Error,
    Info,
    CommandHelp,
}

pub struct CommandResult {
    pub lines: [[u8; 64]; 24],
    pub lens: [usize; 24],
    pub types: [LineType; 24],
    pub count: usize,
    pub action: ConsoleAction,
}

impl CommandResult {
    pub fn new() -> Self {
        Self {
            lines: [[0u8; 64]; 24],
            lens: [0; 24],
            types: [LineType::Normal; 24],
            count: 0,
            action: ConsoleAction::None,
        }
    }

    pub fn add_line(&mut self, text: &[u8], line_type: LineType) {
        if self.count >= self.lines.len() {
            return;
        }
        let mut len = 0;
        for &b in text {
            if len >= 64 {
                break;
            }
            self.lines[self.count][len] = b;
            len += 1;
        }
        self.lens[self.count] = len;
        self.types[self.count] = line_type;
        self.count += 1;
    }

    pub fn add_success(&mut self, text: &[u8]) {
        self.add_line(text, LineType::Success);
    }

    pub fn add_error(&mut self, text: &[u8]) {
        self.add_line(text, LineType::Error);
    }

    pub fn add_info(&mut self, text: &[u8]) {
        self.add_line(text, LineType::Info);
    }

    pub fn add_normal(&mut self, text: &[u8]) {
        self.add_line(text, LineType::Normal);
    }

    pub fn add_command_help(&mut self, command: &[u8], description: &[u8]) {
        let mut line = [0u8; 64];
        let mut pos = 0usize;
        pos += write_str(&mut line[pos..], command);
        pos += write_str(&mut line[pos..], b" - ");
        pos += write_str(&mut line[pos..], description);
        self.add_line(&line[..pos], LineType::CommandHelp);
    }
}

pub fn execute_command(cmd: &[u8], len: usize, rand_state: &mut u32, fb_w: usize, fb_h: usize) -> CommandResult {
    let mut result = CommandResult::new();
    
    let (head, rest) = split_first_word(cmd, len);
    if head.is_empty() {
        return result;
    }

    if eq_ignore_case(head, b"help") {
        result.add_info(b"=== Commands ===");
        result.add_command_help(b"help", b"show this list");
        result.add_command_help(b"clear / cls", b"clear terminal");
        result.add_command_help(b"explorer / dir / ls", b"open file explorer");
        result.add_command_help(b"office / notepad", b"open text editor");
        result.add_command_help(b"browser / web", b"open browser");
        result.add_command_help(b"security", b"open protection window");
        result.add_command_help(b"scan", b"scan system integrity");
        result.add_command_help(b"protect on/off", b"toggle guard");
        result.add_command_help(b"disk / disk rescan", b"show or rescan devices");
        result.add_command_help(b"sound", b"sound and volume tools");
        result.add_command_help(b"https <url>", b"DNS/TCP/TLS probe");
        result.add_command_help(b"net", b"network diagnostics");
        result.add_command_help(b"battery", b"battery status");
        result.add_command_help(b"set", b"theme, clock, accent, mouse");
        result.add_command_help(b"reboot / shutdown", b"power commands");
    } else if eq_ignore_case(head, b"clear") || eq_ignore_case(head, b"cls") {
        result.add_success(b"Screen cleared");
    } else if eq_ignore_case(head, b"echo") {
        if rest.is_empty() {
            result.add_normal(b"");
        } else {
            result.add_normal(rest);
        }
    } else if eq_ignore_case(head, b"ver") || eq_ignore_case(head, b"version") {
        result.add_info(b"HouseOS v3.4.0");
        result.add_success(b"Build: Stable (GMT aware)");
    } else if eq_ignore_case(head, b"about") {
        result.add_info(b"=== HouseOS v3.4 ===");
        result.add_normal(b"Lightweight OS Demo with timezone support");
        result.add_success(b"Status: Running");
    } else if eq_ignore_case(head, b"res") || eq_ignore_case(head, b"mode") {
        let mut buf = [0u8; 64];
        let mut pos = 0;
        pos += write_str(&mut buf[pos..], b"Resolution: ");
        pos += write_u32(&mut buf[pos..], fb_w as u32);
        if pos < 64 { buf[pos] = b'x'; pos += 1; }
        pos += write_u32(&mut buf[pos..], fb_h as u32);
        result.add_line(&buf[..pos], LineType::Info);
        result.add_success(b"Display mode active");
    } else if eq_ignore_case(head, b"whoami") {
        result.add_success(b"root");
    } else if eq_ignore_case(head, b"ping") {
        result.add_success(b"pong");
    } else if eq_ignore_case(head, b"time") {
        if let Some(t) = rtc::read_time() {
            let adjusted = system::apply_timezone(t);
            let settings = system::ui_settings();
            let mut hour = adjusted.hour;
            let mut is_pm = false;
            
            if !settings.clock_24h {
                if hour >= 12 {
                    is_pm = true;
                }
                if hour == 0 {
                    hour = 12;
                } else if hour > 12 {
                    hour -= 12;
                }
            }
            
            let mut buf = [0u8; 64];
            let mut pos = 0;
            pos += write_str(&mut buf[pos..], b"Time (GMT");
            let offset = system::get_gmt_offset();
            if offset >= 0 {
                if pos < 64 { buf[pos] = b'+'; pos += 1; }
                pos += write_i8(&mut buf[pos..], offset);
            } else {
                if pos < 64 { buf[pos] = b'-'; pos += 1; }
                pos += write_i8(&mut buf[pos..], -offset);
            }
            if pos < 64 { buf[pos] = b')'; pos += 1; }
            if pos < 64 { buf[pos] = b':'; pos += 1; }
            pos += write_str(&mut buf[pos..], b" ");
            pos += write_two(&mut buf[pos..], hour);
            if pos < 64 { buf[pos] = b':'; pos += 1; }
            pos += write_two(&mut buf[pos..], adjusted.min);
            if pos < 64 { buf[pos] = b':'; pos += 1; }
            pos += write_two(&mut buf[pos..], adjusted.sec);
            
            if !settings.clock_24h {
                if pos < 64 { buf[pos] = b' '; pos += 1; }
                if is_pm {
                    pos += write_str(&mut buf[pos..], b"PM");
                } else {
                    pos += write_str(&mut buf[pos..], b"AM");
                }
            }
            
            result.add_line(&buf[..pos], LineType::Success);
        } else {
            result.add_error(b"Time unavailable");
        }
    } else if eq_ignore_case(head, b"date") {
        if let Some(t) = rtc::read_time() {
            let adjusted = system::apply_timezone(t);
            let mut buf = [0u8; 64];
            let mut pos = 0;
            pos += write_str(&mut buf[pos..], b"Date: ");
            pos += write_u32(&mut buf[pos..], adjusted.year as u32);
            if pos < 64 { buf[pos] = b'-'; pos += 1; }
            pos += write_two(&mut buf[pos..], adjusted.month);
            if pos < 64 { buf[pos] = b'-'; pos += 1; }
            pos += write_two(&mut buf[pos..], adjusted.day);
            result.add_line(&buf[..pos], LineType::Success);
        } else {
            result.add_error(b"Date unavailable");
        }
    } else if eq_ignore_case(head, b"mem") {
        let info = system::system_info();
        let total_mib = info.mem_total_kib / 1024;
        let avail_mib = info.mem_avail_kib / 1024;
        let used_mib = total_mib.saturating_sub(avail_mib);
        
        result.add_info(b"=== Memory Info ===");
        add_mem_line(&mut result, b"Total: ", total_mib, b" MiB");
        add_mem_line(&mut result, b"Available: ", avail_mib, b" MiB");
        add_mem_line(&mut result, b"Used: ", used_mib, b" MiB");
        add_runtime_mem_line(&mut result);
        result.add_success(b"Memory OK");
    } else if eq_ignore_case(head, b"sysinfo") {
        let info = system::system_info();
        result.add_info(b"=== System Info ===");
        
        let mut buf = [0u8; 64];
        let mut pos = 0;
        pos += write_str(&mut buf[pos..], b"Display: ");
        pos += write_u32(&mut buf[pos..], info.fb_w as u32);
        if pos < 64 { buf[pos] = b'x'; pos += 1; }
        pos += write_u32(&mut buf[pos..], info.fb_h as u32);
        if pos < 64 { buf[pos] = b' '; pos += 1; }
        pos += write_u32(&mut buf[pos..], info.fb_bpp as u32);
        pos += write_str(&mut buf[pos..], b"bpp");
        result.add_line(&buf[..pos], LineType::Normal);
        
        let total_mib = info.mem_total_kib / 1024;
        add_mem_line(&mut result, b"RAM: ", total_mib, b" MiB");
        result.add_success(b"System healthy");
    } else if eq_ignore_case(head, b"rand") {
        *rand_state = rand_state.wrapping_mul(1664525).wrapping_add(1013904223);
        let mut buf = [0u8; 64];
        let mut pos = 0;
        pos += write_str(&mut buf[pos..], b"Random: ");
        pos += write_u32(&mut buf[pos..], *rand_state);
        result.add_line(&buf[..pos], LineType::Success);
    } else if eq_ignore_case(head, b"set") {
        handle_set(rest, &mut result);
    } else if eq_ignore_case(head, b"clip") || eq_ignore_case(head, b"clipboard") {
        result.add_success(b"Opening clipboard...");
        result.action = ConsoleAction::OpenClipboard;
    } else if eq_ignore_case(head, b"copy") {
        if rest.is_empty() {
            result.add_error(b"Usage: copy <text>");
        } else {
            clipboard::set(rest);
            result.add_success(b"Copied to clipboard");
        }
    } else if eq_ignore_case(head, b"paste") {
        let data = clipboard::data();
        if data.is_empty() {
            result.add_error(b"Clipboard is empty");
        } else {
            result.add_normal(data);
        }
    } else if eq_ignore_case(head, b"explorer") || eq_ignore_case(head, b"dir") || eq_ignore_case(head, b"ls") {
        result.add_success(b"Opening file explorer...");
        result.action = ConsoleAction::OpenExplorer;
    } else if eq_ignore_case(head, b"notepad") || eq_ignore_case(head, b"note") || eq_ignore_case(head, b"office") || eq_ignore_case(head, b"writer") {
        result.add_success(b"Opening HouseOffice...");
        result.action = ConsoleAction::OpenNotepad;
    } else if eq_ignore_case(head, b"browser") || eq_ignore_case(head, b"web") {
        result.add_success(b"Opening browser...");
        result.action = ConsoleAction::OpenBrowser;
    } else if eq_ignore_case(head, b"security") || eq_ignore_case(head, b"defender") {
        result.add_success(b"Opening security window...");
        result.action = ConsoleAction::OpenSecurity;
    } else if eq_ignore_case(head, b"gmt") || eq_ignore_case(head, b"tz") {
        handle_gmt(rest, &mut result);
    } else if eq_ignore_case(head, b"uptime") {
        handle_uptime(&mut result);
    } else if eq_ignore_case(head, b"beep") {
        handle_beep(&mut result);
    } else if eq_ignore_case(head, b"sound") || eq_ignore_case(head, b"audio") {
        handle_sound(rest, &mut result);
    } else if eq_ignore_case(head, b"disk") || eq_ignore_case(head, b"storage") {
        handle_disk(rest, &mut result);
    } else if eq_ignore_case(head, b"battery") || eq_ignore_case(head, b"powerstat") {
        handle_battery(&mut result);
    } else if eq_ignore_case(head, b"https") || eq_ignore_case(head, b"tls") {
        handle_https(rest, &mut result);
    } else if eq_ignore_case(head, b"sysfetch") {
        handle_sysfetch(&mut result, fb_w, fb_h);
    } else if eq_ignore_case(head, b"scan") || eq_ignore_case(head, b"integrity") {
        handle_scan(rest, &mut result);
        if let Some(report) = protection::last_scan() {
            if report.warnings > 0 {
                result.action = ConsoleAction::OpenSecurity;
            }
        }
    } else if eq_ignore_case(head, b"protect") || eq_ignore_case(head, b"guard") {
        handle_protect(rest, &mut result);
    } else if eq_ignore_case(head, b"net") || eq_ignore_case(head, b"netdiag") {
        handle_net(rest, &mut result);
    } else if eq_ignore_case(head, b"reboot") {
        result.add_success(b"Rebooting...");
        result.action = ConsoleAction::Reboot;
    } else if eq_ignore_case(head, b"shutdown") {
        result.add_success(b"Shutting down...");
        result.action = ConsoleAction::Shutdown;
    } else {
        let mut buf = [0u8; 64];
        let mut pos = 0;
        pos += write_str(&mut buf[pos..], b"Unknown: '");
        for &b in head {
            if pos >= 60 { break; }
            buf[pos] = b;
            pos += 1;
        }
        if pos < 63 { buf[pos] = b'\''; pos += 1; }
        result.add_error(&buf[..pos]);
    }

    result
}

// ---- GMT (виправлений) ----
fn handle_gmt(rest: &[u8], result: &mut CommandResult) {
    if rest.is_empty() {
        let off = system::get_gmt_offset();
        let mut buf = [0u8; 32];
        let mut pos = 0;
        pos += write_str(&mut buf[pos..], b"Current GMT offset: ");
        if off >= 0 {
            if pos < 32 { buf[pos] = b'+'; pos += 1; }
            pos += write_i8(&mut buf[pos..], off);
        } else {
            if pos < 32 { buf[pos] = b'-'; pos += 1; }
            pos += write_i8(&mut buf[pos..], -off);
        }
        result.add_success(&buf[..pos]);
        return;
    }

    // Пропускаємо пробіли на початку
    let mut i = 0;
    while i < rest.len() && rest[i] == b' ' {
        i += 1;
    }
    if i >= rest.len() {
        result.add_error(b"Missing offset");
        return;
    }
    let start = i;
    while i < rest.len() && rest[i] != b' ' {
        i += 1;
    }
    let token = &rest[start..i];
    if token.is_empty() {
        result.add_error(b"Missing offset");
        return;
    }

    let mut sign: i8 = 1;
    let mut idx = 0;
    if token[0] == b'+' {
        sign = 1;
        idx = 1;
    } else if token[0] == b'-' {
        sign = -1;
        idx = 1;
    }
    if idx >= token.len() {
        result.add_error(b"Missing number after sign");
        return;
    }

    let mut val: i8 = 0;
    for &b in &token[idx..] {
        if b < b'0' || b > b'9' {
            result.add_error(b"Invalid number");
            return;
        }
        val = val * 10 + (b - b'0') as i8;
        if val > 14 {
            result.add_error(b"Offset out of range (-12..+14)");
            return;
        }
    }
    let new_offset = sign * val;
    if new_offset < -12 || new_offset > 14 {
        result.add_error(b"Offset out of range (-12..+14)");
        return;
    }
    system::set_gmt_offset(new_offset);
    let mut buf = [0u8; 32];
    let mut pos = 0;
    pos += write_str(&mut buf[pos..], b"GMT offset set to ");
    if new_offset >= 0 {
        if pos < 32 { buf[pos] = b'+'; pos += 1; }
        pos += write_i8(&mut buf[pos..], new_offset);
    } else {
        if pos < 32 { buf[pos] = b'-'; pos += 1; }
        pos += write_i8(&mut buf[pos..], -new_offset);
    }
    result.add_success(&buf[..pos]);
}

// ---- Uptime ----
fn handle_uptime(result: &mut CommandResult) {
    if let Some(boot) = system::get_boot_time() {
        if let Some(now) = rtc::read_time() {
            let boot_secs = boot.hour as u32 * 3600 + boot.min as u32 * 60 + boot.sec as u32;
            let now_secs = now.hour as u32 * 3600 + now.min as u32 * 60 + now.sec as u32;
            let mut uptime_secs = if now_secs >= boot_secs {
                now_secs - boot_secs
            } else {
                (24*3600 - boot_secs) + now_secs
            };
            let day_diff = (now.year as i32 - boot.year as i32) * 365 +
                           (now.month as i32 - boot.month as i32) * 30 +
                           (now.day as i32 - boot.day as i32);
            if day_diff > 0 {
                uptime_secs += (day_diff as u32) * 86400;
            }
            let days = uptime_secs / 86400;
            let hours = (uptime_secs % 86400) / 3600;
            let mins = (uptime_secs % 3600) / 60;
            let secs = uptime_secs % 60;
            let mut buf = [0u8; 64];
            let mut pos = 0;
            if days > 0 {
                pos += write_u32(&mut buf[pos..], days);
                pos += write_str(&mut buf[pos..], b" days, ");
            }
            pos += write_u32(&mut buf[pos..], hours);
            pos += write_str(&mut buf[pos..], b":");
            pos += write_two(&mut buf[pos..], mins as u8);
            pos += write_str(&mut buf[pos..], b":");
            pos += write_two(&mut buf[pos..], secs as u8);
            result.add_success(&buf[..pos]);
            return;
        }
    }
    result.add_error(b"Uptime not available (RTC missing)");
}

// ---- Beep ----
fn handle_beep(result: &mut CommandResult) {
    pc_speaker::click();
    result.add_success(b"Beep!");
}

fn handle_sound(rest: &[u8], result: &mut CommandResult) {
    let (arg, tail) = split_words(rest);
    if arg.is_empty() || eq_ignore_case(arg, b"status") {
        result.add_info(b"=== Sound ===");
        result.add_normal(b"Driver: PC speaker");
        add_volume_line(result);
        result.add_normal(b"sound test|demo|vol N|mute|on");
        return;
    }

    if eq_ignore_case(arg, b"test") {
        pc_speaker::play_sweep();
        result.add_success(b"Sound test played");
        return;
    }
    if eq_ignore_case(arg, b"demo") || eq_ignore_case(arg, b"music") {
        pc_speaker::play_demo();
        result.add_success(b"Demo sound played");
        return;
    }
    if eq_ignore_case(arg, b"beep") {
        pc_speaker::click();
        result.add_success(b"Beep!");
        return;
    }
    if eq_ignore_case(arg, b"mute") || eq_ignore_case(arg, b"off") {
        pc_speaker::set_volume(0);
        pc_speaker::silence();
        result.add_success(b"Sound muted");
        return;
    }
    if eq_ignore_case(arg, b"on") || eq_ignore_case(arg, b"unmute") {
        if pc_speaker::get_volume() == 0 {
            pc_speaker::toggle_mute();
        }
        result.add_success(b"Sound enabled");
        add_volume_line(result);
        return;
    }
    if eq_ignore_case(arg, b"up") {
        pc_speaker::volume_up();
        add_volume_line(result);
        return;
    }
    if eq_ignore_case(arg, b"down") {
        pc_speaker::volume_down();
        add_volume_line(result);
        return;
    }
    if eq_ignore_case(arg, b"vol") || eq_ignore_case(arg, b"volume") {
        let (value, _) = split_words(tail);
        if let Some(volume) = parse_percent(value) {
            pc_speaker::set_volume(volume);
            add_volume_line(result);
        } else {
            result.add_error(b"Usage: sound vol 0..100");
        }
        return;
    }

    result.add_error(b"Usage: sound test|demo|vol N|mute|on");
}

fn add_volume_line(result: &mut CommandResult) {
    let mut buf = [0u8; 64];
    let mut pos = 0usize;
    pos += write_str(&mut buf[pos..], b"Volume: ");
    pos += write_u32(&mut buf[pos..], pc_speaker::get_volume() as u32);
    pos += write_str(&mut buf[pos..], b"%");
    result.add_normal(&buf[..pos]);
}

fn handle_disk(rest: &[u8], result: &mut CommandResult) {
    let (arg, _) = split_words(rest);
    if eq_ignore_case(arg, b"rescan") || eq_ignore_case(arg, b"refresh") {
        let disks = ata_pio::rescan();
        let usb_count = usb::rescan().len();
        result.add_success(b"Device rescan complete");
        let mut line = [0u8; 64];
        let mut pos = 0usize;
        pos += write_str(&mut line[pos..], b"ATA disks: ");
        pos += write_u32(&mut line[pos..], disks as u32);
        pos += write_str(&mut line[pos..], b", USB controllers: ");
        pos += write_u32(&mut line[pos..], usb_count as u32);
        result.add_normal(&line[..pos]);
        return;
    }

    let drives = ata_pio::drives();
    let parts = ata_pio::partitions();
    let controllers = usb::controllers();
    result.add_info(b"=== Disk Driver ===");
    result.add_normal(b"Driver: ATA PIO + NTFS/exFAT read-only");
    if drives.is_empty() {
        result.add_error(b"No ATA disks detected");
        result.add_normal(b"Needs IDE/ATA disk in VM for Full Disk");
    }

    for i in 0..drives.len().min(4) {
        let d = drives[i];
        let mib = d.sectors / 2048;
        let mut buf = [0u8; 64];
        let mut pos = 0usize;
        pos += write_str(&mut buf[pos..], b"Disk ");
        pos += write_u32(&mut buf[pos..], i as u32);
        pos += write_str(&mut buf[pos..], if d.slave { b" slave " } else { b" master " });
        pos += write_u32(&mut buf[pos..], mib as u32);
        pos += write_str(&mut buf[pos..], b" MiB");
        if d.supports_lba48 {
            pos += write_str(&mut buf[pos..], b" LBA48");
        } else {
            pos += write_str(&mut buf[pos..], b" LBA28");
        }
        result.add_normal(&buf[..pos]);
    }

    if controllers.is_empty() {
        result.add_info(b"No USB controllers detected");
    } else {
        let mut head = [0u8; 64];
        let mut pos = 0usize;
        pos += write_str(&mut head[pos..], b"USB controllers: ");
        pos += write_u32(&mut head[pos..], controllers.len() as u32);
        result.add_info(&head[..pos]);
        for i in 0..controllers.len().min(8) {
            let c = controllers[i];
            let mut buf = [0u8; 64];
            let mut pos = 0usize;
            pos += write_str(&mut buf[pos..], b"  USB ");
            pos += write_u32(&mut buf[pos..], c.bus as u32);
            pos += write_str(&mut buf[pos..], b":");
            pos += write_u32(&mut buf[pos..], c.dev as u32);
            pos += write_str(&mut buf[pos..], b".");
            pos += write_u32(&mut buf[pos..], c.func as u32);
            pos += write_str(&mut buf[pos..], b" irq ");
            pos += write_u32(&mut buf[pos..], c.irq_line as u32);
            result.add_normal(&buf[..pos]);
        }
        result.add_normal(b"USB flash read needs USB Mass Storage/BOT layer");
    }

    if drives.is_empty() {
        return;
    }

    if parts.is_empty() {
        result.add_info(b"No MBR/GPT partitions found");
        return;
    }
    for i in 0..parts.len().min(6) {
        let p = parts[i];
        let mib = p.sectors / 2048;
        let mut buf = [0u8; 64];
        let mut pos = 0usize;
        pos += write_str(&mut buf[pos..], b"  P");
        pos += write_u32(&mut buf[pos..], p.index as u32 + 1);
        pos += write_str(&mut buf[pos..], b" D");
        pos += write_u32(&mut buf[pos..], p.drive_index as u32);
        pos += write_str(&mut buf[pos..], b" type ");
        pos += write_hex8(&mut buf[pos..], p.part_type);
        pos += write_str(&mut buf[pos..], if p.gpt { b" GPT " } else { b" MBR " });
        if p.name_len > 0 {
            pos += write_str(&mut buf[pos..], &p.name[..p.name_len.min(p.name.len())]);
            if pos < buf.len() {
                buf[pos] = b' ';
                pos += 1;
            }
        }
        let mut volume_label = [0u8; 48];
        let volume_len = partition_volume_label(i, &mut volume_label);
        if volume_len > 0 {
            pos += write_str(&mut buf[pos..], b"\"");
            pos += write_str(&mut buf[pos..], &volume_label[..volume_len]);
            pos += write_str(&mut buf[pos..], b"\" ");
        }
        pos += write_str(&mut buf[pos..], b" ");
        pos += write_u32(&mut buf[pos..], mib as u32);
        pos += write_str(&mut buf[pos..], b" MiB");
        if p.bootable {
            pos += write_str(&mut buf[pos..], b" boot");
        }
        if ntfs::NtfsVolume::open(i).is_some() {
            pos += write_str(&mut buf[pos..], b" NTFS");
        } else if exfat::ExfatVolume::open(i).is_some() {
            pos += write_str(&mut buf[pos..], b" exFAT");
        }
        result.add_normal(&buf[..pos]);
    }
}

fn partition_volume_label(part_index: usize, out: &mut [u8; 48]) -> usize {
    if let Some(vol) = ntfs::NtfsVolume::open(part_index) {
        return vol.volume_label(out).min(out.len());
    }
    if let Some(vol) = exfat::ExfatVolume::open(part_index) {
        return vol.volume_label(out).min(out.len());
    }
    0
}

fn handle_battery(result: &mut CommandResult) {
    battery::update();
    result.add_info(b"=== Battery Driver ===");
    if !battery::has_battery() {
        result.add_error(b"Battery: not exposed");
        result.add_normal(b"Needs ACPI/EC battery device");
        result.add_normal(b"No fake CMOS 100% fallback");
        return;
    }

    let level = battery::get_level().min(100);
    let mut buf = [0u8; 64];
    let mut pos = 0usize;
    pos += write_str(&mut buf[pos..], b"Charge: ");
    pos += write_u32(&mut buf[pos..], level as u32);
    if pos < buf.len() {
        buf[pos] = b'%';
        pos += 1;
    }
    result.add_success(&buf[..pos]);
    result.add_normal(b"Source: QEMU/EC battery port");
}

fn handle_https(rest: &[u8], result: &mut CommandResult) {
    let (arg, _) = split_words(rest);
    let mut url = [0u8; 64];
    let url_len = if arg.is_empty() {
        write_str(&mut url, b"https://example.com/")
    } else if starts_with_ci_local(arg, b"https://") || starts_with_ci_local(arg, b"http://") {
        write_str(&mut url, arg)
    } else {
        let mut pos = write_str(&mut url, b"https://");
        pos += write_str(&mut url[pos..], arg);
        pos
    };

    result.add_info(b"=== HTTPS Probe ===");
    result.add_normal(&url[..url_len]);
    match netstack::https_probe(&url[..url_len]) {
        netstack::FetchCode::TlsHandshakeOnly => {
            result.add_success(b"DNS/TCP/TLS responded");
            result.add_normal(b"TLS decrypt/browser HTTPS pending");
        }
        netstack::FetchCode::Ok => result.add_success(b"HTTPS OK"),
        netstack::FetchCode::NoNic => result.add_error(b"No network card ready"),
        netstack::FetchCode::DnsFailed => result.add_error(b"DNS failed"),
        netstack::FetchCode::ArpFailed => result.add_error(b"ARP gateway failed"),
        netstack::FetchCode::TcpFailed => result.add_error(b"TCP port 443 failed"),
        netstack::FetchCode::Timeout => result.add_error(b"HTTPS timeout"),
        netstack::FetchCode::BadUrl => result.add_error(b"Bad URL"),
        netstack::FetchCode::HttpsUnsupported => result.add_error(b"TLS handshake unsupported"),
        netstack::FetchCode::BufferFull => result.add_error(b"Buffer full"),
    }
}

// ---- Sysfetch ----
fn handle_sysfetch(result: &mut CommandResult, fb_w: usize, fb_h: usize) {
    let info = system::system_info();
    result.add_info(b"    _____                      ");
    result.add_info(b"   /  _  \\___  ___  ___ ______");
    result.add_info(b"  /  /_\\  \\  \\/  / |/ // __/");
    result.add_info(b" /    /    \\    /|   /\\__ \\");
    result.add_info(b" \\____|_  /__/\\_\\ |_| /___/");
    result.add_info(b"        \\/                    ");
    result.add_normal(b"");
    let mut buf = [0u8; 64];
    let mut pos = 0;
    pos += write_str(&mut buf[pos..], b"OS: HouseOS v3.4");
    result.add_normal(&buf[..pos]);
    pos = 0;
    pos += write_str(&mut buf[pos..], b"Resolution: ");
    pos += write_u32(&mut buf[pos..], fb_w as u32);
    if pos < 64 { buf[pos] = b'x'; pos += 1; }
    pos += write_u32(&mut buf[pos..], fb_h as u32);
    result.add_normal(&buf[..pos]);
    pos = 0;
    pos += write_str(&mut buf[pos..], b"RAM: ");
    pos += write_u64(&mut buf[pos..], info.mem_total_kib / 1024);
    pos += write_str(&mut buf[pos..], b" MiB");
    result.add_normal(&buf[..pos]);
    pos = 0;
    pos += write_str(&mut buf[pos..], b"Timezone: GMT");
    let off = system::get_gmt_offset();
    if off >= 0 {
        if pos < 64 { buf[pos] = b'+'; pos += 1; }
        pos += write_i8(&mut buf[pos..], off);
    } else {
        if pos < 64 { buf[pos] = b'-'; pos += 1; }
        pos += write_i8(&mut buf[pos..], -off);
    }
    result.add_normal(&buf[..pos]);
    result.add_success(b"System ready");
}

// ---- Допоміжні функції ----
fn add_mem_line(result: &mut CommandResult, prefix: &[u8], val: u64, suffix: &[u8]) {
    let mut buf = [0u8; 64];
    let mut pos = 0;
    pos += write_str(&mut buf[pos..], prefix);
    pos += write_u64(&mut buf[pos..], val);
    pos += write_str(&mut buf[pos..], suffix);
    result.add_normal(&buf[..pos]);
}

fn add_runtime_mem_line(result: &mut CommandResult) {
    let (used, total, _allocs, blocked) = memory::get_memory_stats();
    let mut buf = [0u8; 64];
    let mut pos = 0usize;
    pos += write_str(&mut buf[pos..], b"Guard: ");
    pos += write_str(
        &mut buf[pos..],
        if memory::memory_protection_enabled() { b"ON " } else { b"OFF " },
    );
    pos += write_u32(&mut buf[pos..], (memory::max_single_allocation() / 1024 / 1024) as u32);
    pos += write_str(&mut buf[pos..], b" MiB max, heap ");
    pos += write_u32(&mut buf[pos..], (used / 1024 / 1024) as u32);
    pos += write_str(&mut buf[pos..], b"/");
    pos += write_u32(&mut buf[pos..], (total / 1024 / 1024) as u32);
    pos += write_str(&mut buf[pos..], b" blocked=");
    pos += write_u32(&mut buf[pos..], blocked as u32);
    result.add_normal(&buf[..pos]);
}

fn handle_scan(rest: &[u8], result: &mut CommandResult) {
    let (arg, _) = split_words(rest);
    let report = if eq_ignore_case(arg, b"what") || eq_ignore_case(arg, b"targets") {
        protection::scan_targets()
    } else if eq_ignore_case(arg, b"last") || eq_ignore_case(arg, b"status") {
        match protection::last_scan() {
            Some(v) => v,
            None => {
                result.add_info(b"No previous scan. Running now.");
                protection::run_scan()
            }
        }
    } else {
        protection::run_scan()
    };

    for i in 0..report.count {
        let line = &report.lines[i];
        let len = report.lens[i];
        if i == 0 {
            result.add_info(&line[..len]);
        } else if starts_with(&line[..len], b"Result: OK") {
            result.add_success(&line[..len]);
        } else if starts_with(&line[..len], b"Result: WARN")
            || starts_with(&line[..len], b"NET:")
            || starts_with(&line[..len], b"USB controllers: 0")
        {
            result.add_error(&line[..len]);
        } else {
            result.add_normal(&line[..len]);
        }
    }
}

fn handle_protect(rest: &[u8], result: &mut CommandResult) {
    let (arg, _) = split_words(rest);
    if arg.is_empty() || eq_ignore_case(arg, b"status") {
        result.add_info(b"=== Protection ===");
        result.add_normal(if protection::enabled() { b"Protection: ON" } else { b"Protection: OFF" });
        add_runtime_mem_line(result);
        result.add_normal(b"Use: protect on|off");
        return;
    }
    if eq_ignore_case(arg, b"on") || eq_ignore_case(arg, b"enable") {
        protection::set_enabled(true);
        result.add_success(b"Protection enabled");
        return;
    }
    if eq_ignore_case(arg, b"off") || eq_ignore_case(arg, b"disable") {
        protection::set_enabled(false);
        result.add_error(b"Protection disabled");
        return;
    }
    result.add_error(b"Usage: protect on|off|status");
}

fn handle_net(rest: &[u8], result: &mut CommandResult) {
    let (arg, _) = split_words(rest);
    if eq_ignore_case(arg, b"reset") {
        netstack::reset_dhcp_probe();
        result.add_success(b"Network probe reset");
        return;
    }

    let status = netstack::tick();
    result.add_info(b"=== Network Stack ===");
    result.add_normal(if status.nic_ready { b"NIC: ready" } else { b"NIC: offline" });
    result.add_normal(if status.dhcp_discover_sent {
        b"DHCP: discover sent"
    } else {
        b"DHCP: not sent"
    });
    result.add_normal(if status.packet_io {
        b"Packet I/O: active"
    } else {
        b"Packet I/O: waiting"
    });

    let mut line = [0u8; 64];
    let mut pos = 0usize;
    pos += write_str(&mut line[pos..], b"Packets TX=");
    pos += write_u32(&mut line[pos..], status.tx_packets);
    pos += write_str(&mut line[pos..], b" RX=");
    pos += write_u32(&mut line[pos..], status.rx_packets);
    pos += write_str(&mut line[pos..], b" ERR=");
    pos += write_u32(&mut line[pos..], status.tx_errors + status.rx_drops);
    result.add_normal(&line[..pos]);

    result.add_normal(if status.tcp_ready { b"TCP: ready" } else { b"TCP: missing" });
    result.add_normal(if status.dns_ready { b"DNS: ready" } else { b"DNS: missing" });
    result.add_normal(if status.tls_ready { b"TLS probe: ready" } else { b"TLS probe: missing" });
    result.add_normal(if status.html_ready { b"HTML: ready" } else { b"HTML: missing" });
    result.add_normal(if status.js_ready { b"JS: ready" } else { b"JS: missing" });
    result.add_normal(if status.video_ready { b"Video: ready" } else { b"Video: missing" });
}

fn split_first_word(buf: &[u8], len: usize) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < len && buf[i] == b' ' {
        i += 1;
    }
    let start = i;
    while i < len && buf[i] != b' ' {
        i += 1;
    }
    let end = i;
    while i < len && buf[i] == b' ' {
        i += 1;
    }
    (&buf[start..end], &buf[i..len])
}

fn split_words(rest: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < rest.len() && rest[i] == b' ' {
        i += 1;
    }
    let start = i;
    while i < rest.len() && rest[i] != b' ' {
        i += 1;
    }
    let end = i;
    while i < rest.len() && rest[i] == b' ' {
        i += 1;
    }
    (&rest[start..end], &rest[i..])
}

fn starts_with(buf: &[u8], prefix: &[u8]) -> bool {
    if buf.len() < prefix.len() {
        return false;
    }
    for i in 0..prefix.len() {
        if buf[i] != prefix[i] {
            return false;
        }
    }
    true
}

fn starts_with_ci_local(buf: &[u8], prefix: &[u8]) -> bool {
    if buf.len() < prefix.len() {
        return false;
    }
    eq_ignore_case(&buf[..prefix.len()], prefix)
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        let mut ca = a[i];
        let mut cb = b[i];
        if ca >= b'A' && ca <= b'Z' {
            ca = ca + 32;
        }
        if cb >= b'A' && cb <= b'Z' {
            cb = cb + 32;
        }
        if ca != cb {
            return false;
        }
    }
    true
}

fn write_str(buf: &mut [u8], s: &[u8]) -> usize {
    let mut pos = 0;
    for &b in s {
        if pos >= buf.len() {
            break;
        }
        buf[pos] = b;
        pos += 1;
    }
    pos
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
    let mut n = 0;
    while val > 0 && n < tmp.len() {
        tmp[n] = (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    let mut out = 0;
    while n > 0 && out < buf.len() {
        n -= 1;
        buf[out] = b'0' + tmp[n];
        out += 1;
    }
    out
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
    let mut n = 0;
    while val > 0 && n < tmp.len() {
        tmp[n] = (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    let mut out = 0;
    while n > 0 && out < buf.len() {
        n -= 1;
        buf[out] = b'0' + tmp[n];
        out += 1;
    }
    out
}

fn write_hex8(buf: &mut [u8], val: u8) -> usize {
    if buf.len() < 4 {
        return 0;
    }
    buf[0] = b'0';
    buf[1] = b'x';
    buf[2] = hex_digit((val >> 4) & 0x0F);
    buf[3] = hex_digit(val & 0x0F);
    4
}

fn hex_digit(v: u8) -> u8 {
    if v < 10 {
        b'0' + v
    } else {
        b'A' + (v - 10)
    }
}

fn write_i8(buf: &mut [u8], val: i8) -> usize {
    if val < 0 {
        return 0;
    }
    write_u32(buf, val as u32)
}

fn write_two(buf: &mut [u8], val: u8) -> usize {
    if buf.len() < 2 {
        return 0;
    }
    buf[0] = b'0' + (val / 10);
    buf[1] = b'0' + (val % 10);
    2
}

fn parse_percent(buf: &[u8]) -> Option<u8> {
    if buf.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for &b in buf {
        if b < b'0' || b > b'9' {
            return None;
        }
        value = value * 10 + (b - b'0') as u32;
        if value > 100 {
            return None;
        }
    }
    Some(value as u8)
}

fn handle_set(rest: &[u8], result: &mut CommandResult) {
    let (key, value) = split_words(rest);
    if key.is_empty() {
        result.add_info(b"=== Settings ===");
        result.add_command_help(b"set clock 24|12", b"time format");
        result.add_command_help(b"set statusbar on|off", b"taskbar visibility");
        result.add_command_help(b"set accent blue|green|orange|gray", b"accent color");
        result.add_command_help(b"set mouse 1|2|3|4", b"mouse speed");
        result.add_command_help(b"set theme dark|light", b"window theme");
        return;
    }

    if eq_ignore_case(key, b"clock") {
        if eq_ignore_case(value, b"24") {
            system::set_clock_24h(true);
            result.add_success(b"Clock set to 24h format");
        } else if eq_ignore_case(value, b"12") {
            system::set_clock_24h(false);
            result.add_success(b"Clock set to 12h format");
        } else {
            result.add_error(b"Usage: set clock 24|12");
        }
        return;
    }

    if eq_ignore_case(key, b"statusbar") {
        if eq_ignore_case(value, b"on") {
            system::set_status_bar(true);
            result.add_success(b"Status bar enabled");
        } else if eq_ignore_case(value, b"off") {
            system::set_status_bar(false);
            result.add_success(b"Status bar disabled");
        } else {
            result.add_error(b"Usage: set statusbar on|off");
        }
        return;
    }

    if eq_ignore_case(key, b"accent") {
        if eq_ignore_case(value, b"blue") {
            system::set_accent(0x003A8FE5);
            result.add_success(b"Accent color: Blue");
        } else if eq_ignore_case(value, b"green") {
            system::set_accent(0x003AA973);
            result.add_success(b"Accent color: Green");
        } else if eq_ignore_case(value, b"orange") {
            system::set_accent(0x00D98A33);
            result.add_success(b"Accent color: Orange");
        } else if eq_ignore_case(value, b"gray") {
            system::set_accent(0x00718393);
            result.add_success(b"Accent color: Gray");
        } else {
            result.add_error(b"Usage: set accent blue|green|orange|gray");
        }
        return;
    }

    if eq_ignore_case(key, b"mouse") {
        if value.len() == 1 {
            let v = value[0];
            if v >= b'1' && v <= b'4' {
                let scale = (v - b'0') as i32;
                system::set_mouse_scale(scale);
                result.add_success(b"Mouse speed updated");
                return;
            }
        }
        result.add_error(b"Usage: set mouse 1|2|3|4");
        return;
    }

    if eq_ignore_case(key, b"theme") {
        if eq_ignore_case(value, b"dark") {
            system::set_theme(true);
            result.add_success(b"Theme: Dark mode");
        } else if eq_ignore_case(value, b"light") {
            system::set_theme(false);
            result.add_success(b"Theme: Light mode");
        } else {
            result.add_error(b"Usage: set theme dark|light");
        }
        return;
    }

    result.add_error(b"Unknown setting");
}
