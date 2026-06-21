#![allow(dead_code)]

use crate::drivers::input::ps2_controller;
use crate::drivers::port_io::inb;

const PS2_STATUS: u16 = 0x64;

#[derive(Clone, Copy, Debug)]
pub struct MousePacket {
    pub buttons: u8,
    pub dx: i8,
    pub dy: i8,
    pub wheel: i8,
}

static mut PACKET_LEN: usize = 3;

pub unsafe fn init() {
    // Disable ports and flush stale data.
    ps2_controller::disable_ports();
    flush_output();
    PACKET_LEN = 3;

    // Enable IRQ12 and ensure the aux port isn't disabled.
    ps2_controller::write_cmd(0x20);
    let mut status = ps2_controller::read_data();
    status |= 0x03; // IRQ1 + IRQ12 enable
    status &= !0x30; // clear keyboard + mouse disable
    ps2_controller::write_cmd(0x60);
    ps2_controller::write_data(status);

    // Enable auxiliary device.
    ps2_controller::enable_ports();

    // Reset mouse and enable streaming.
    write_mouse(0xFF);
    let _ = read_mouse_ack();
    let _ = read_mouse_response(); // self-test (0xAA)
    let _ = read_mouse_response(); // device id

    write_mouse(0xF6); // defaults
    let _ = read_mouse_ack();

    // IntelliMouse unlock: sample rates 200, 100, 80, then ask for device id.
    // A wheel-capable PS/2 mouse answers id 3/4 and sends 4-byte packets.
    if set_sample_rate(200) && set_sample_rate(100) && set_sample_rate(80) {
        write_mouse(0xF2); // get device id
        if read_mouse_ack() {
            if let Some(id) = read_mouse_response() {
                if id == 3 || id == 4 {
                    PACKET_LEN = 4;
                } else {
                    PACKET_LEN = 3;
                }
            }
        }
    }

    write_mouse(0xF4); // data reporting
    let _ = read_mouse_ack();
}

pub unsafe fn read_packet() -> MousePacket {
    let b0 = ps2_controller::read_data();
    let b1 = ps2_controller::read_data();
    let b2 = ps2_controller::read_data();
    let b3 = if PACKET_LEN >= 4 { ps2_controller::read_data() } else { 0 };
    MousePacket {
        buttons: b0 & 0x07,
        dx: b1 as i8,
        dy: b2 as i8,
        wheel: wheel_delta_from_byte(b3),
    }
}

pub unsafe fn packet_len() -> usize {
    PACKET_LEN
}

pub fn wheel_delta_from_byte(b: u8) -> i8 {
    let nibble = (b & 0x0F) as i8;
    if (nibble & 0x08) != 0 {
        nibble - 16
    } else {
        nibble
    }
}

pub unsafe fn read_byte() -> Option<u8> {
    let status = inb(PS2_STATUS);
    if (status & 0x01) == 0 {
        return None;
    }
    if (status & 0x20) == 0 {
        return None;
    }
    Some(ps2_controller::read_data())
}

unsafe fn write_mouse(val: u8) {
    ps2_controller::write_cmd(0xD4);
    ps2_controller::write_data(val);
}

unsafe fn set_sample_rate(rate: u8) -> bool {
    write_mouse(0xF3);
    if !read_mouse_ack() {
        return false;
    }
    write_mouse(rate);
    read_mouse_ack()
}

fn flush_output() {
    for _ in 0..10000 {
        let status = unsafe { inb(PS2_STATUS) };
        if (status & 0x01) == 0 {
            break;
        }
        let _ = unsafe { ps2_controller::read_data() };
    }
}

fn read_mouse_response() -> Option<u8> {
    for _ in 0..10000 {
        if let Some(b) = unsafe { read_byte() } {
            return Some(b);
        }
    }
    None
}

fn read_mouse_ack() -> bool {
    for _ in 0..10000 {
        if let Some(b) = unsafe { read_byte() } {
            return b == 0xFA;
        }
    }
    false
}
