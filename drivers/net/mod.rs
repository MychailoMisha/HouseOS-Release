#![allow(dead_code)]

use core::hint::spin_loop;

use crate::drivers::pci;
use crate::drivers::port_io::{inb, outb, outl, outw};

const MAX_NET_DEVICES: usize = 16;
const RTL8139_RX_BUFFER_SIZE: usize = 8192 + 16 + 1500;
const RTL8139_RING_SIZE: usize = 8192;
const RTL8139_TX_BUFFER_SIZE: usize = 1536;

#[derive(Copy, Clone, PartialEq)]
pub enum NetKind {
    IntelE1000,
    Realtek8139,
    VirtioNet,
    Unknown,
}

#[derive(Copy, Clone, PartialEq)]
pub enum NetState {
    Detected,
    Ready,
    Error,
}

#[derive(Copy, Clone)]
pub struct NetDevice {
    pub kind: NetKind,
    pub state: NetState,
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub bars: [u32; 6],
    pub io_base: u16,
    pub mmio_base: u32,
    pub irq_line: u8,
    pub driver_online: bool,
    pub mac: [u8; 6],
}

#[derive(Copy, Clone)]
pub struct NetStats {
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub tx_errors: u32,
    pub rx_drops: u32,
}

impl NetDevice {
    const EMPTY: NetDevice = NetDevice {
        kind: NetKind::Unknown,
        state: NetState::Detected,
        bus: 0,
        dev: 0,
        func: 0,
        vendor_id: 0xFFFF,
        device_id: 0xFFFF,
        bars: [0; 6],
        io_base: 0,
        mmio_base: 0,
        irq_line: 0,
        driver_online: false,
        mac: [0; 6],
    };
}

static mut DEVICES: [NetDevice; MAX_NET_DEVICES] = [NetDevice::EMPTY; MAX_NET_DEVICES];
static mut DEVICE_COUNT: usize = 0;
static mut SCANNED: bool = false;
static mut RTL8139_RX_BUFFER: [u8; RTL8139_RX_BUFFER_SIZE] = [0; RTL8139_RX_BUFFER_SIZE];
static mut RTL8139_TX_BUFFERS: [[u8; RTL8139_TX_BUFFER_SIZE]; 4] = [[0; RTL8139_TX_BUFFER_SIZE]; 4];
static mut RTL8139_RX_OFFSET: usize = 0;
static mut RTL8139_TX_INDEX: usize = 0;
static mut TX_PACKETS: u32 = 0;
static mut RX_PACKETS: u32 = 0;
static mut TX_ERRORS: u32 = 0;
static mut RX_DROPS: u32 = 0;

pub fn init() -> &'static [NetDevice] {
    unsafe {
        if SCANNED {
            return &DEVICES[..DEVICE_COUNT];
        }

        DEVICE_COUNT = 0;
        let pci_devices = pci::scan();
        for pci_dev in pci_devices {
            if pci_dev.class_code != 0x02 {
                continue;
            }
            if DEVICE_COUNT >= MAX_NET_DEVICES {
                break;
            }

            let mut net_dev = NetDevice {
                kind: classify(pci_dev.vendor_id, pci_dev.device_id),
                state: NetState::Detected,
                bus: pci_dev.bus,
                dev: pci_dev.dev,
                func: pci_dev.func,
                vendor_id: pci_dev.vendor_id,
                device_id: pci_dev.device_id,
                bars: pci_dev.bars,
                io_base: parse_io_base(&pci_dev.bars),
                mmio_base: parse_mmio_base(&pci_dev.bars),
                irq_line: pci_dev.irq_line,
                driver_online: false,
                mac: [0; 6],
            };

            match net_dev.kind {
                NetKind::Realtek8139 => init_rtl8139(&mut net_dev),
                NetKind::IntelE1000 | NetKind::VirtioNet | NetKind::Unknown => {}
            }

            DEVICES[DEVICE_COUNT] = net_dev;
            DEVICE_COUNT += 1;
        }
        SCANNED = true;
        &DEVICES[..DEVICE_COUNT]
    }
}

pub fn devices() -> &'static [NetDevice] {
    init()
}

pub fn any() -> bool {
    !devices().is_empty()
}

pub fn stats() -> NetStats {
    unsafe {
        NetStats {
            tx_packets: TX_PACKETS,
            rx_packets: RX_PACKETS,
            tx_errors: TX_ERRORS,
            rx_drops: RX_DROPS,
        }
    }
}

pub fn send_frame(frame: &[u8]) -> bool {
    let dev = match first_ready_rtl8139() {
        Some(v) => v,
        None => return false,
    };
    if frame.is_empty() || frame.len() > RTL8139_TX_BUFFER_SIZE {
        unsafe {
            TX_ERRORS = TX_ERRORS.wrapping_add(1);
        }
        return false;
    }

    unsafe {
        let idx = RTL8139_TX_INDEX & 3;
        let tx_len = frame.len().max(60).min(RTL8139_TX_BUFFER_SIZE);
        for i in 0..tx_len {
            RTL8139_TX_BUFFERS[idx][i] = if i < frame.len() { frame[i] } else { 0 };
        }
        let ptr = core::ptr::addr_of_mut!(RTL8139_TX_BUFFERS[idx]) as *mut u8 as usize as u32;
        outl(dev.io_base + rtl::TSAD0 + (idx as u16) * 4, ptr);
        outl(dev.io_base + rtl::TSD0 + (idx as u16) * 4, tx_len as u32);
        RTL8139_TX_INDEX = (RTL8139_TX_INDEX + 1) & 3;
        TX_PACKETS = TX_PACKETS.wrapping_add(1);
    }
    true
}

pub fn poll_frame(out: &mut [u8]) -> Option<usize> {
    let dev = first_ready_rtl8139()?;
    if out.is_empty() {
        return None;
    }

    unsafe {
        if (inb(dev.io_base + rtl::CR) & rtl::CR_BUFE) != 0 {
            return None;
        }

        let offset = RTL8139_RX_OFFSET % RTL8139_RING_SIZE;
        let status = read_rx_u16(offset);
        let rx_len = read_rx_u16(offset + 2) as usize;
        if status == 0 || rx_len < 4 || rx_len > 1800 {
            RX_DROPS = RX_DROPS.wrapping_add(1);
            RTL8139_RX_OFFSET = (offset + 4) % RTL8139_RING_SIZE;
            outw(dev.io_base + rtl::CAPR, capr_value(RTL8139_RX_OFFSET));
            return None;
        }

        let frame_len = rx_len.saturating_sub(4).min(out.len());
        for i in 0..frame_len {
            out[i] = RTL8139_RX_BUFFER[(offset + 4 + i) % RTL8139_RING_SIZE];
        }

        let next = (offset + rx_len + 4 + 3) & !3;
        RTL8139_RX_OFFSET = next % RTL8139_RING_SIZE;
        outw(dev.io_base + rtl::CAPR, capr_value(RTL8139_RX_OFFSET));
        RX_PACKETS = RX_PACKETS.wrapping_add(1);
        Some(frame_len)
    }
}

fn first_ready_rtl8139() -> Option<NetDevice> {
    let devs = devices();
    for dev in devs {
        if dev.kind == NetKind::Realtek8139 && dev.driver_online && dev.io_base != 0 {
            return Some(*dev);
        }
    }
    None
}

fn classify(vendor: u16, device: u16) -> NetKind {
    match (vendor, device) {
        (0x8086, 0x100E) | (0x8086, 0x100F) | (0x8086, 0x10D3) => NetKind::IntelE1000,
        (0x10EC, 0x8139) => NetKind::Realtek8139,
        (0x1AF4, 0x1000) | (0x1AF4, 0x1041) => NetKind::VirtioNet,
        _ => NetKind::Unknown,
    }
}

fn parse_io_base(bars: &[u32; 6]) -> u16 {
    for &bar in bars {
        if bar == 0 {
            continue;
        }
        if (bar & 0x1) != 0 {
            return (bar & !0x3) as u16;
        }
    }
    0
}

fn parse_mmio_base(bars: &[u32; 6]) -> u32 {
    for &bar in bars {
        if bar == 0 {
            continue;
        }
        if (bar & 0x1) == 0 {
            return bar & !0xF;
        }
    }
    0
}

fn enable_pci_io_bus_master(bus: u8, dev: u8, func: u8) {
    let bus = bus as u16;
    let dev = dev as u16;
    let func = func as u16;
    let mut command = pci::read_config_word(bus, dev, func, 0x04);
    command |= 0x0001; // I/O space enable
    command |= 0x0004; // bus master enable
    pci::write_config_word(bus, dev, func, 0x04, command);
}

fn init_rtl8139(dev: &mut NetDevice) {
    if dev.io_base == 0 {
        dev.state = NetState::Error;
        return;
    }

    enable_pci_io_bus_master(dev.bus, dev.dev, dev.func);

    let io = dev.io_base;
    unsafe {
        outb(io + rtl::CONFIG1, 0x00);
        outb(io + rtl::CR, rtl::CR_RST);

        let mut reset_ok = false;
        for _ in 0..100_000 {
            if (inb(io + rtl::CR) & rtl::CR_RST) == 0 {
                reset_ok = true;
                break;
            }
            spin_loop();
        }
        if !reset_ok {
            dev.state = NetState::Error;
            return;
        }

        let rx_ptr = core::ptr::addr_of_mut!(RTL8139_RX_BUFFER) as *mut u8 as usize as u32;
        outl(io + rtl::RBSTART, rx_ptr);

        outw(io + rtl::IMR, 0x0000);
        outw(io + rtl::ISR, 0xFFFF);
        outl(io + rtl::RCR, rtl::RCR_ACCEPT_ALL | rtl::RCR_WRAP);
        outw(io + rtl::CAPR, capr_value(0));
        RTL8139_RX_OFFSET = 0;
        RTL8139_TX_INDEX = 0;
        outb(io + rtl::CR, rtl::CR_RE | rtl::CR_TE);

        for i in 0..6 {
            dev.mac[i] = inb(io + i as u16);
        }
        let command = inb(io + rtl::CR);
        dev.driver_online = (command & (rtl::CR_RE | rtl::CR_TE)) == (rtl::CR_RE | rtl::CR_TE);
        dev.state = if dev.driver_online {
            NetState::Ready
        } else {
            NetState::Error
        };
    }
}

unsafe fn read_rx_u16(offset: usize) -> u16 {
    let a = RTL8139_RX_BUFFER[offset % RTL8139_RING_SIZE] as u16;
    let b = RTL8139_RX_BUFFER[(offset + 1) % RTL8139_RING_SIZE] as u16;
    a | (b << 8)
}

fn capr_value(offset: usize) -> u16 {
    offset.wrapping_sub(16) as u16
}

mod rtl {
    pub const TSD0: u16 = 0x10;
    pub const TSAD0: u16 = 0x20;
    pub const CR: u16 = 0x37;
    pub const RBSTART: u16 = 0x30;
    pub const CAPR: u16 = 0x38;
    pub const IMR: u16 = 0x3C;
    pub const ISR: u16 = 0x3E;
    pub const RCR: u16 = 0x44;
    pub const CONFIG1: u16 = 0x52;

    pub const CR_TE: u8 = 1 << 2;
    pub const CR_RE: u8 = 1 << 3;
    pub const CR_RST: u8 = 1 << 4;
    pub const CR_BUFE: u8 = 1 << 0;

    pub const RCR_ACCEPT_ALL: u32 = 0x0000_000F;
    pub const RCR_WRAP: u32 = 1 << 7;
}
