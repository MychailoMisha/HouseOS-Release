#![allow(dead_code)]

use core::arch::asm;

use crate::drivers::port_io::{inb, outb};

const PIT_BASE: u32 = 1_193_180;
static mut VOLUME: u8 = 70;
static mut LAST_VOLUME: u8 = 70;

pub fn get_volume() -> u8 {
    unsafe { VOLUME }
}

pub fn set_volume(value: u8) {
    unsafe {
        VOLUME = value.min(100);
        if VOLUME > 0 {
            LAST_VOLUME = VOLUME;
        }
    }
}

pub fn volume_up() {
    let next = get_volume().saturating_add(10).min(100);
    set_volume(next);
}

pub fn volume_down() {
    let current = get_volume();
    set_volume(current.saturating_sub(10));
}

pub fn toggle_mute() {
    unsafe {
        if VOLUME == 0 {
            VOLUME = LAST_VOLUME.max(10);
        } else {
            LAST_VOLUME = VOLUME;
            VOLUME = 0;
        }
    }
}

pub fn play_tone(freq: u32, loops: u32) {
    let volume = get_volume();
    if volume == 0 {
        return;
    }
    let loops = ((loops as u64 * volume as u64) / 100).max(1) as u32;
    if freq == 0 {
        delay(loops);
        return;
    }
    let div = (PIT_BASE / freq).max(1).min(u16::MAX as u32);
    unsafe {
        outb(0x43, 0xB6);
        outb(0x42, (div & 0xFF) as u8);
        outb(0x42, ((div >> 8) & 0xFF) as u8);
        let status = inb(0x61);
        outb(0x61, status | 0x03);
        delay(loops);
        outb(0x61, status & !0x03);
    }
}

pub fn silence() {
    unsafe {
        let status = inb(0x61);
        outb(0x61, status & !0x03);
    }
}

pub fn click() {
    play_tone(880, 90_000);
}

pub fn play_sweep() {
    let notes = [196, 247, 294, 349, 440, 523, 659, 784, 659, 523, 440, 349, 294, 247, 196];
    for &note in &notes {
        play_tone(note, 55_000);
        delay(12_000);
    }
    silence();
}

pub fn play_demo() {
    let notes = [262, 330, 392, 523, 392, 330, 262];
    for &note in &notes {
        play_tone(note, 120_000);
        delay(35_000);
    }
    silence();
}

pub fn play_bytes(data: &[u8]) {
    if data.is_empty() {
        play_demo();
        return;
    }
    let mut i = 0usize;
    let step = (data.len() / 96).max(1);
    while i < data.len() {
        let sample = data[i] as u32;
        let freq = 180 + sample * 5;
        play_tone(freq.min(1800), 18_000);
        i += step;
    }
    silence();
}

pub fn play_pcm_u8(data: &[u8], step: usize) {
    if data.is_empty() {
        play_demo();
        return;
    }
    let mut i = 0usize;
    let stride = step.max(1);
    let mut played = 0usize;
    while i < data.len() && played < 160 {
        let sample = data[i] as i32 - 128;
        let amp = if sample < 0 { -sample } else { sample } as u32;
        let freq = 160 + amp * 12;
        play_tone(freq.min(2200), 12_000);
        i += stride;
        played += 1;
    }
    silence();
}

pub fn play_pcm_i16_le(data: &[u8], step: usize) {
    if data.len() < 2 {
        play_demo();
        return;
    }
    let mut i = 0usize;
    let stride = step.max(2);
    let mut played = 0usize;
    while i + 1 < data.len() && played < 160 {
        let raw = (data[i] as u16) | ((data[i + 1] as u16) << 8);
        let sample = raw as i16 as i32;
        let amp = if sample < 0 { -sample } else { sample } as u32;
        let freq = 160 + (amp / 32);
        play_tone(freq.min(2400), 12_000);
        i += stride;
        played += 1;
    }
    silence();
}

fn delay(loops: u32) {
    for _ in 0..loops {
        unsafe {
            asm!("pause");
        }
    }
}
