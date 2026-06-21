// ============================================================================
// Файл: netstack.rs
// ============================================================================

use core::hint::spin_loop;

use crate::drivers::net::{self, NetDevice};

const ETH_TYPE_IPV4: u16 = 0x0800;
const ETH_TYPE_ARP: u16 = 0x0806;
const IP_PROTO_TCP: u8 = 6;
const IP_PROTO_UDP: u8 = 17;
const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_XID: u32 = 0x484F_5345;
const DNS_PORT: u16 = 53;
const HTTP_PORT: u16 = 80;
const HTTPS_PORT: u16 = 443;
const LOCAL_IP: [u8; 4] = [10, 0, 2, 15];
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];
const DNS_IP: [u8; 4] = [10, 0, 2, 3];

const ARP_WAIT: usize = 180_000;
const DNS_WAIT: usize = 260_000;
const TCP_WAIT: usize = 520_000;
const HTTP_WAIT: usize = 820_000;

const ANY_IP: [u8; 4] = [0, 0, 0, 0];

#[derive(Copy, Clone)]
pub struct StackStatus {
    pub nic_ready: bool,
    pub packet_io: bool,
    pub dhcp_discover_sent: bool,
    pub rx_packets: u32,
    pub tx_packets: u32,
    pub tx_errors: u32,
    pub rx_drops: u32,
    pub dns_ready: bool,
    pub tcp_ready: bool,
    pub tls_ready: bool,
    pub html_ready: bool,
    pub js_ready: bool,
    pub video_ready: bool,
}

#[derive(Copy, Clone, PartialEq)]
pub enum FetchCode {
    Ok,
    BufferFull,
    BadUrl,
    HttpsUnsupported,
    TlsHandshakeOnly,
    NoNic,
    ArpFailed,
    DnsFailed,
    TcpFailed,
    Timeout,
}

#[derive(Copy, Clone)]
pub struct HttpFetch {
    pub code: FetchCode,
    pub status_code: u16,
    pub bytes: usize,
    pub body_offset: usize,
    pub remote_ip: [u8; 4],
    pub local_ip: [u8; 4],
    pub dns_used: bool,
    pub tcp_connected: bool,
}

struct ParsedUrl {
    host: [u8; 128],
    host_len: usize,
    path: [u8; 256],
    path_len: usize,
    port: u16,
}

impl ParsedUrl {
    const EMPTY: ParsedUrl = ParsedUrl {
        host: [0; 128],
        host_len: 0,
        path: [0; 256],
        path_len: 0,
        port: HTTP_PORT,
    };
}

struct TcpPacket {
    flags: u8,
    seq: u32,
    ack: u32,
    payload_len: usize,
    copied_len: usize,
}

static mut DHCP_SENT: bool = false;
static mut RX_SCRATCH: [u8; 1536] = [0; 1536];
static mut NEXT_LOCAL_PORT: u16 = 40000;
static mut NEXT_IP_ID: u16 = 0x2400;
static mut LAST_PACKET_IO: bool = false;
static mut LAST_DNS_READY: bool = false;
static mut LAST_TCP_READY: bool = false;
static mut LAST_TLS_READY: bool = false;
static mut LAST_HTML_READY: bool = false;

pub fn tick() -> StackStatus {
    let mut nic_ready = false;
    let mut packet_io = false;

    let devices = net::devices();
    if let Some(dev) = devices.first() {
        nic_ready = dev.driver_online;
        if nic_ready {
            unsafe {
                if !DHCP_SENT {
                    DHCP_SENT = send_dhcp_discover(dev.mac);
                }
                if let Some(_) = net::poll_frame(&mut RX_SCRATCH) {
                    packet_io = true;
                    LAST_PACKET_IO = true;
                }
            }
        }
    }

    let stats = net::stats();
    StackStatus {
        nic_ready,
        packet_io: packet_io || unsafe { LAST_PACKET_IO },
        dhcp_discover_sent: unsafe { DHCP_SENT },
        rx_packets: stats.rx_packets,
        tx_packets: stats.tx_packets,
        tx_errors: stats.tx_errors,
        rx_drops: stats.rx_drops,
        dns_ready: unsafe { LAST_DNS_READY },
        tcp_ready: unsafe { LAST_TCP_READY },
        tls_ready: unsafe { LAST_TLS_READY },
        html_ready: unsafe { LAST_HTML_READY },
        js_ready: false,
        video_ready: false,
    }
}

pub fn reset_dhcp_probe() {
    unsafe {
        DHCP_SENT = false;
        LAST_PACKET_IO = false;
        LAST_DNS_READY = false;
        LAST_TCP_READY = false;
        LAST_TLS_READY = false;
        LAST_HTML_READY = false;
    }
}

pub fn http_get(url: &[u8], out: &mut [u8]) -> HttpFetch {
    unsafe {
        LAST_DNS_READY = false;
        LAST_TCP_READY = false;
        LAST_HTML_READY = false;
    }
    clear(out);

    if starts_with_ci(url, b"https://") {
        return fetch_result(FetchCode::HttpsUnsupported, 0, 0, 0, [0; 4], false, false);
    }

    let mut parsed = ParsedUrl::EMPTY;
    let parse_code = parse_url(url, &mut parsed);
    if parse_code != FetchCode::Ok {
        return fetch_result(parse_code, 0, 0, 0, [0; 4], false, false);
    }
    let remote_port = parsed.port;

    let dev = match ready_device() {
        Some(v) => v,
        None => return fetch_result(FetchCode::NoNic, 0, 0, 0, [0; 4], false, false),
    };

    let mut remote_ip = [0u8; 4];
    let mut dns_used = false;
    if parse_ipv4(&parsed.host[..parsed.host_len], &mut remote_ip) {
        unsafe {
            LAST_DNS_READY = true;
        }
    } else if resolve_dns(&dev, &parsed.host[..parsed.host_len], &mut remote_ip) {
        dns_used = true;
        unsafe {
            LAST_DNS_READY = true;
        }
    } else {
        return fetch_result(FetchCode::DnsFailed, 0, 0, 0, [0; 4], true, false);
    }

    let route_ip = if same_subnet(remote_ip) {
        remote_ip
    } else {
        GATEWAY_IP
    };
    let mut route_mac = [0u8; 6];
    if !resolve_mac(&dev, route_ip, &mut route_mac) {
        return fetch_result(FetchCode::ArpFailed, 0, 0, 0, remote_ip, dns_used, false);
    }

    let local_port = next_local_port();
    let mut client_seq = 0x484F_0000u32.wrapping_add(local_port as u32);
    if !send_tcp(
        &dev,
        route_mac,
        remote_ip,
        local_port,
        remote_port,
        client_seq,
        0,
        tcp::SYN,
        &[],
    ) {
        return fetch_result(FetchCode::TcpFailed, 0, 0, 0, remote_ip, dns_used, false);
    }

    let mut payload = [0u8; 1460];
    let syn_ack = match wait_tcp(remote_ip, remote_port, local_port, &mut payload, TCP_WAIT) {
        Some(pkt) if (pkt.flags & (tcp::SYN | tcp::ACK)) == (tcp::SYN | tcp::ACK)
            && pkt.ack == client_seq.wrapping_add(1) =>
        {
            pkt
        }
        _ => return fetch_result(FetchCode::TcpFailed, 0, 0, 0, remote_ip, dns_used, false),
    };

    client_seq = client_seq.wrapping_add(1);
    let mut server_seq = syn_ack.seq.wrapping_add(1);
    send_tcp(
        &dev,
        route_mac,
        remote_ip,
        local_port,
        remote_port,
        client_seq,
        server_seq,
        tcp::ACK,
        &[],
    );
    unsafe {
        LAST_TCP_READY = true;
    }

    let mut request = [0u8; 512];
    let req_len = build_http_request(&parsed, &mut request);
    if req_len == 0 {
        return fetch_result(FetchCode::BadUrl, 0, 0, 0, remote_ip, dns_used, true);
    }
    if !send_tcp(
        &dev,
        route_mac,
        remote_ip,
        local_port,
        remote_port,
        client_seq,
        server_seq,
        tcp::ACK | tcp::PSH,
        &request[..req_len],
    ) {
        return fetch_result(FetchCode::TcpFailed, 0, 0, 0, remote_ip, dns_used, true);
    }
    client_seq = client_seq.wrapping_add(req_len as u32);

    let mut copied = 0usize;
    let mut got_data = false;
    let mut code = FetchCode::Ok;
    for _ in 0..HTTP_WAIT {
        if let Some(pkt) = poll_tcp(remote_ip, remote_port, local_port, &mut payload) {
            unsafe {
                LAST_PACKET_IO = true;
            }
            if (pkt.flags & tcp::RST) != 0 {
                return fetch_result(FetchCode::TcpFailed, 0, copied, 0, remote_ip, dns_used, true);
            }
            if pkt.payload_len > 0 {
                got_data = true;
                if pkt.seq == server_seq {
                    let take = pkt.copied_len.min(out.len().saturating_sub(copied));
                    for i in 0..take {
                        out[copied + i] = payload[i];
                    }
                    copied += take;
                    server_seq = server_seq.wrapping_add(pkt.payload_len as u32);
                    send_tcp(
                        &dev,
                        route_mac,
                        remote_ip,
                        local_port,
                        remote_port,
                        client_seq,
                        server_seq,
                        tcp::ACK,
                        &[],
                    );
                    if copied >= out.len() {
                        code = FetchCode::BufferFull;
                        break;
                    }
                } else {
                    send_tcp(
                        &dev,
                        route_mac,
                        remote_ip,
                        local_port,
                        remote_port,
                        client_seq,
                        server_seq,
                        tcp::ACK,
                        &[],
                    );
                }
            }
            if (pkt.flags & tcp::FIN) != 0 {
                server_seq = server_seq.wrapping_add(1);
                send_tcp(
                    &dev,
                    route_mac,
                    remote_ip,
                    local_port,
                    remote_port,
                    client_seq,
                    server_seq,
                    tcp::ACK,
                    &[],
                );
                break;
            }
        } else {
            spin_loop();
        }
    }

    if !got_data {
        return fetch_result(FetchCode::Timeout, 0, copied, 0, remote_ip, dns_used, true);
    }

    let body_offset = find_body_offset(&out[..copied]).unwrap_or(copied);
    let status = parse_status_code(&out[..copied]);
    unsafe {
        LAST_HTML_READY = body_offset < copied;
    }
    fetch_result(code, status, copied, body_offset, remote_ip, dns_used, true)
}

pub fn https_probe(url: &[u8]) -> FetchCode {
    unsafe {
        LAST_DNS_READY = false;
        LAST_TCP_READY = false;
        LAST_TLS_READY = false;
        LAST_HTML_READY = false;
    }

    let mut parsed = ParsedUrl::EMPTY;
    let parse_code = parse_url(url, &mut parsed);
    if parse_code != FetchCode::Ok {
        return parse_code;
    }

    let dev = match ready_device() {
        Some(v) => v,
        None => return FetchCode::NoNic,
    };

    let mut remote_ip = [0u8; 4];
    if parse_ipv4(&parsed.host[..parsed.host_len], &mut remote_ip) {
        unsafe {
            LAST_DNS_READY = true;
        }
    } else if resolve_dns(&dev, &parsed.host[..parsed.host_len], &mut remote_ip) {
        unsafe {
            LAST_DNS_READY = true;
        }
    } else {
        return FetchCode::DnsFailed;
    }

    let route_ip = if same_subnet(remote_ip) {
        remote_ip
    } else {
        GATEWAY_IP
    };
    let mut route_mac = [0u8; 6];
    if !resolve_mac(&dev, route_ip, &mut route_mac) {
        return FetchCode::ArpFailed;
    }

    let local_port = next_local_port();
    let mut client_seq = 0x4854_0000u32.wrapping_add(local_port as u32);
    if !send_tcp(
        &dev,
        route_mac,
        remote_ip,
        local_port,
        HTTPS_PORT,
        client_seq,
        0,
        tcp::SYN,
        &[],
    ) {
        return FetchCode::TcpFailed;
    }

    let mut payload = [0u8; 1460];
    let syn_ack = match wait_tcp(remote_ip, HTTPS_PORT, local_port, &mut payload, TCP_WAIT) {
        Some(pkt) if (pkt.flags & (tcp::SYN | tcp::ACK)) == (tcp::SYN | tcp::ACK)
            && pkt.ack == client_seq.wrapping_add(1) =>
        {
            pkt
        }
        _ => return FetchCode::TcpFailed,
    };

    client_seq = client_seq.wrapping_add(1);
    let mut server_seq = syn_ack.seq.wrapping_add(1);
    send_tcp(
        &dev,
        route_mac,
        remote_ip,
        local_port,
        HTTPS_PORT,
        client_seq,
        server_seq,
        tcp::ACK,
        &[],
    );
    unsafe {
        LAST_TCP_READY = true;
    }

    let mut hello = [0u8; 512];
    let hello_len = build_tls_client_hello(&parsed.host[..parsed.host_len], &mut hello);
    if hello_len == 0 {
        return FetchCode::HttpsUnsupported;
    }
    if !send_tcp(
        &dev,
        route_mac,
        remote_ip,
        local_port,
        HTTPS_PORT,
        client_seq,
        server_seq,
        tcp::ACK | tcp::PSH,
        &hello[..hello_len],
    ) {
        return FetchCode::TcpFailed;
    }
    client_seq = client_seq.wrapping_add(hello_len as u32);

    for _ in 0..TCP_WAIT {
        if let Some(pkt) = poll_tcp(remote_ip, HTTPS_PORT, local_port, &mut payload) {
            unsafe {
                LAST_PACKET_IO = true;
            }
            if pkt.payload_len > 0 {
                server_seq = server_seq.wrapping_add(pkt.payload_len as u32);
                send_tcp(
                    &dev,
                    route_mac,
                    remote_ip,
                    local_port,
                    HTTPS_PORT,
                    client_seq,
                    server_seq,
                    tcp::ACK,
                    &[],
                );
                if payload[0] == 0x16 || payload[0] == 0x15 {
                    unsafe {
                        LAST_TLS_READY = true;
                    }
                    return FetchCode::TlsHandshakeOnly;
                }
                return FetchCode::HttpsUnsupported;
            }
        } else {
            spin_loop();
        }
    }

    FetchCode::Timeout
}

fn fetch_result(
    code: FetchCode,
    status_code: u16,
    bytes: usize,
    body_offset: usize,
    remote_ip: [u8; 4],
    dns_used: bool,
    tcp_connected: bool,
) -> HttpFetch {
    HttpFetch {
        code,
        status_code,
        bytes,
        body_offset,
        remote_ip,
        local_ip: LOCAL_IP,
        dns_used,
        tcp_connected,
    }
}

fn ready_device() -> Option<NetDevice> {
    let devices = net::devices();
    for dev in devices {
        if dev.driver_online && dev.mac != [0; 6] {
            return Some(*dev);
        }
    }
    None
}

fn parse_url(url: &[u8], parsed: &mut ParsedUrl) -> FetchCode {
    parsed.host_len = 0;
    parsed.path_len = 0;
    parsed.port = HTTP_PORT;

    let mut start = 0usize;
    if starts_with_ci(url, b"https://") {
        start = 8;
        parsed.port = HTTPS_PORT;
    }
    if starts_with_ci(url, b"http://") {
        start = 7;
        parsed.port = HTTP_PORT;
    }
    while start < url.len() && url[start] == b' ' {
        start += 1;
    }
    if start >= url.len() {
        return FetchCode::BadUrl;
    }

    let mut i = start;
    while i < url.len() && url[i] != b'/' && url[i] != b' ' && url[i] != 0 {
        if url[i] == b':' {
            break;
        }
        if parsed.host_len >= parsed.host.len() {
            return FetchCode::BadUrl;
        }
        parsed.host[parsed.host_len] = ascii_lower(url[i]);
        parsed.host_len += 1;
        i += 1;
    }
    if parsed.host_len == 0 {
        return FetchCode::BadUrl;
    }
    if i < url.len() && url[i] == b':' {
        i += 1;
        let mut port = 0u32;
        let mut digits = 0usize;
        while i < url.len() && url[i] >= b'0' && url[i] <= b'9' {
            port = port * 10 + (url[i] - b'0') as u32;
            if port > 65535 {
                return FetchCode::BadUrl;
            }
            digits += 1;
            i += 1;
        }
        if digits == 0 {
            return FetchCode::BadUrl;
        }
        parsed.port = port as u16;
    }
    while i < url.len() && url[i] != b'/' && url[i] != b' ' && url[i] != 0 {
        i += 1;
    }
    if i < url.len() && url[i] == b'/' {
        while i < url.len() && url[i] != b' ' && url[i] != 0 {
            if parsed.path_len + 1 >= parsed.path.len() {
                break;
            }
            parsed.path[parsed.path_len] = url[i];
            parsed.path_len += 1;
            i += 1;
        }
    } else {
        parsed.path[0] = b'/';
        parsed.path_len = 1;
    }
    FetchCode::Ok
}

fn build_tls_client_hello(host: &[u8], out: &mut [u8]) -> usize {
    if host.is_empty() || host.len() > 120 || out.len() < 128 + host.len() {
        return 0;
    }

    let mut p = 9usize;
    write_u16_be(&mut out[p..p + 2], 0x0303);
    p += 2;
    for i in 0..32 {
        out[p + i] = (0x48u8).wrapping_add((i as u8).wrapping_mul(13));
    }
    p += 32;
    out[p] = 0;
    p += 1;

    write_u16_be(&mut out[p..p + 2], 8);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0xC02F);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0xC030);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0x009C);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0x002F);
    p += 2;

    out[p] = 1;
    p += 1;
    out[p] = 0;
    p += 1;

    let ext_len_pos = p;
    p += 2;
    let ext_start = p;

    write_u16_be(&mut out[p..p + 2], 0x0000);
    p += 2;
    write_u16_be(&mut out[p..p + 2], (host.len() + 5) as u16);
    p += 2;
    write_u16_be(&mut out[p..p + 2], (host.len() + 3) as u16);
    p += 2;
    out[p] = 0;
    p += 1;
    write_u16_be(&mut out[p..p + 2], host.len() as u16);
    p += 2;
    for &b in host {
        out[p] = b;
        p += 1;
    }

    write_u16_be(&mut out[p..p + 2], 0x000A);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 6);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 4);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0x001D);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0x0017);
    p += 2;

    write_u16_be(&mut out[p..p + 2], 0x000B);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 2);
    p += 2;
    out[p] = 1;
    p += 1;
    out[p] = 0;
    p += 1;

    write_u16_be(&mut out[p..p + 2], 0x000D);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 6);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 4);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0x0401);
    p += 2;
    write_u16_be(&mut out[p..p + 2], 0x0403);
    p += 2;

    let ext_len = p - ext_start;
    write_u16_be(&mut out[ext_len_pos..ext_len_pos + 2], ext_len as u16);

    let body_len = p - 9;
    out[0] = 0x16;
    out[1] = 0x03;
    out[2] = 0x01;
    write_u16_be(&mut out[3..5], (body_len + 4) as u16);
    out[5] = 0x01;
    write_u24_be(&mut out[6..9], body_len as u32);
    p
}

fn build_http_request(parsed: &ParsedUrl, out: &mut [u8]) -> usize {
    let mut p = 0usize;
    p += write_bytes(&mut out[p..], b"GET ");
    p += write_bytes(&mut out[p..], &parsed.path[..parsed.path_len]);
    p += write_bytes(&mut out[p..], b" HTTP/1.1\r\nHost: ");
    p += write_bytes(&mut out[p..], &parsed.host[..parsed.host_len]);
    p += write_bytes(
        &mut out[p..],
        b"\r\nUser-Agent: HouseOS/0.2\r\nAccept: text/html,text/plain,*/*\r\nConnection: close\r\n\r\n",
    );
    p
}

fn resolve_dns(dev: &NetDevice, host: &[u8], out_ip: &mut [u8; 4]) -> bool {
    let mut dns_mac = [0u8; 6];
    if !resolve_mac(dev, DNS_IP, &mut dns_mac) {
        return false;
    }

    let query_id = 0x484Fu16;
    let src_port = next_local_port();
    let mut packet = [0u8; 512];
    write_u16_be(&mut packet[0..2], query_id);
    write_u16_be(&mut packet[2..4], 0x0100);
    write_u16_be(&mut packet[4..6], 1);
    write_u16_be(&mut packet[6..8], 0);
    write_u16_be(&mut packet[8..10], 0);
    write_u16_be(&mut packet[10..12], 0);
    let mut p = 12usize;
    let mut label_start = 0usize;
    while label_start < host.len() {
        let mut label_len = 0usize;
        while label_start + label_len < host.len() && host[label_start + label_len] != b'.' {
            label_len += 1;
        }
        if label_len == 0 || label_len > 63 || p + label_len + 1 >= packet.len() {
            return false;
        }
        packet[p] = label_len as u8;
        p += 1;
        for i in 0..label_len {
            packet[p + i] = host[label_start + i];
        }
        p += label_len;
        label_start += label_len + 1;
    }
    if p + 5 >= packet.len() {
        return false;
    }
    packet[p] = 0;
    p += 1;
    write_u16_be(&mut packet[p..p + 2], 1);
    p += 2;
    write_u16_be(&mut packet[p..p + 2], 1);
    p += 2;

    if !send_udp(dev, dns_mac, DNS_IP, src_port, DNS_PORT, &packet[..p]) {
        return false;
    }

    let mut rx = [0u8; 1536];
    for _ in 0..DNS_WAIT {
        if let Some(len) = net::poll_frame(&mut rx) {
            unsafe {
                LAST_PACKET_IO = true;
            }
            if let Some((udp_start, udp_len)) = udp_payload(&rx[..len], DNS_IP, DNS_PORT, src_port) {
                let msg = &rx[udp_start..udp_start + udp_len];
                if parse_dns_response(msg, query_id, out_ip) {
                    return true;
                }
            }
        } else {
            spin_loop();
        }
    }
    false
}

fn parse_dns_response(msg: &[u8], query_id: u16, out_ip: &mut [u8; 4]) -> bool {
    if msg.len() < 12 || read_u16_be(&msg[0..2]) != query_id {
        return false;
    }
    let flags = read_u16_be(&msg[2..4]);
    if (flags & 0x8000) == 0 || (flags & 0x000F) != 0 {
        return false;
    }
    let qd = read_u16_be(&msg[4..6]) as usize;
    let an = read_u16_be(&msg[6..8]) as usize;
    let mut p = 12usize;
    for _ in 0..qd {
        p = match skip_dns_name(msg, p) {
            Some(v) => v,
            None => return false,
        };
        if p + 4 > msg.len() {
            return false;
        }
        p += 4;
    }
    for _ in 0..an {
        p = match skip_dns_name(msg, p) {
            Some(v) => v,
            None => return false,
        };
        if p + 10 > msg.len() {
            return false;
        }
        let typ = read_u16_be(&msg[p..p + 2]);
        let class = read_u16_be(&msg[p + 2..p + 4]);
        let rd_len = read_u16_be(&msg[p + 8..p + 10]) as usize;
        p += 10;
        if p + rd_len > msg.len() {
            return false;
        }
        if typ == 1 && class == 1 && rd_len == 4 {
            out_ip.copy_from_slice(&msg[p..p + 4]);
            return true;
        }
        p += rd_len;
    }
    false
}

fn skip_dns_name(msg: &[u8], mut p: usize) -> Option<usize> {
    loop {
        if p >= msg.len() {
            return None;
        }
        let len = msg[p];
        if (len & 0xC0) == 0xC0 {
            if p + 1 >= msg.len() {
                return None;
            }
            return Some(p + 2);
        }
        if len == 0 {
            return Some(p + 1);
        }
        p += 1 + len as usize;
    }
}

fn resolve_mac(dev: &NetDevice, target_ip: [u8; 4], out_mac: &mut [u8; 6]) -> bool {
    let mut frame = [0u8; 42];
    for i in 0..6 {
        frame[i] = 0xFF;
        frame[6 + i] = dev.mac[i];
    }
    write_u16_be(&mut frame[12..14], ETH_TYPE_ARP);
    write_u16_be(&mut frame[14..16], 1);
    write_u16_be(&mut frame[16..18], ETH_TYPE_IPV4);
    frame[18] = 6;
    frame[19] = 4;
    write_u16_be(&mut frame[20..22], 1);
    frame[22..28].copy_from_slice(&dev.mac);
    frame[28..32].copy_from_slice(&LOCAL_IP);
    frame[32..38].fill(0);
    frame[38..42].copy_from_slice(&target_ip);
    if !net::send_frame(&frame) {
        return false;
    }

    let mut rx = [0u8; 1536];
    for _ in 0..ARP_WAIT {
        if let Some(len) = net::poll_frame(&mut rx) {
            unsafe {
                LAST_PACKET_IO = true;
            }
            if len >= 42
                && read_u16_be(&rx[12..14]) == ETH_TYPE_ARP
                && read_u16_be(&rx[20..22]) == 2
                && rx[28..32] == target_ip
            {
                out_mac.copy_from_slice(&rx[22..28]);
                return true;
            }
        } else {
            spin_loop();
        }
    }
    false
}

fn send_udp(
    dev: &NetDevice,
    dst_mac: [u8; 6],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> bool {
    if payload.len() + 42 > 1514 {
        return false;
    }
    let mut frame = [0u8; 1514];
    write_eth(&mut frame[0..14], dst_mac, dev.mac, ETH_TYPE_IPV4);
    let ip = 14usize;
    let udp = ip + 20;
    let udp_len = 8 + payload.len();
    write_ipv4_header(
        &mut frame[ip..ip + 20],
        IP_PROTO_UDP,
        (20 + udp_len) as u16,
        LOCAL_IP,
        dst_ip,
    );
    write_u16_be(&mut frame[udp..udp + 2], src_port);
    write_u16_be(&mut frame[udp + 2..udp + 4], dst_port);
    write_u16_be(&mut frame[udp + 4..udp + 6], udp_len as u16);
    write_u16_be(&mut frame[udp + 6..udp + 8], 0);
    frame[udp + 8..udp + 8 + payload.len()].copy_from_slice(payload);
    net::send_frame(&frame[..14 + 20 + udp_len])
}

fn send_tcp(
    dev: &NetDevice,
    dst_mac: [u8; 6],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> bool {
    if payload.len() + 54 > 1514 {
        return false;
    }
    let mut frame = [0u8; 1514];
    write_eth(&mut frame[0..14], dst_mac, dev.mac, ETH_TYPE_IPV4);
    let ip = 14usize;
    let tcp_start = ip + 20;
    let tcp_len = 20 + payload.len();
    write_ipv4_header(
        &mut frame[ip..ip + 20],
        IP_PROTO_TCP,
        (20 + tcp_len) as u16,
        LOCAL_IP,
        dst_ip,
    );
    write_u16_be(&mut frame[tcp_start..tcp_start + 2], src_port);
    write_u16_be(&mut frame[tcp_start + 2..tcp_start + 4], dst_port);
    write_u32_be(&mut frame[tcp_start + 4..tcp_start + 8], seq);
    write_u32_be(&mut frame[tcp_start + 8..tcp_start + 12], ack);
    frame[tcp_start + 12] = 5 << 4;
    frame[tcp_start + 13] = flags;
    write_u16_be(&mut frame[tcp_start + 14..tcp_start + 16], 4096);
    write_u16_be(&mut frame[tcp_start + 16..tcp_start + 18], 0);
    write_u16_be(&mut frame[tcp_start + 18..tcp_start + 20], 0);
    frame[tcp_start + 20..tcp_start + 20 + payload.len()].copy_from_slice(payload);
    let csum = tcp_checksum(LOCAL_IP, dst_ip, &frame[tcp_start..tcp_start + tcp_len]);
    write_u16_be(&mut frame[tcp_start + 16..tcp_start + 18], csum);
    net::send_frame(&frame[..14 + 20 + tcp_len])
}

fn wait_tcp(
    remote_ip: [u8; 4],
    remote_port: u16,
    local_port: u16,
    payload: &mut [u8],
    tries: usize,
) -> Option<TcpPacket> {
    for _ in 0..tries {
        if let Some(pkt) = poll_tcp(remote_ip, remote_port, local_port, payload) {
            return Some(pkt);
        }
        spin_loop();
    }
    None
}

fn poll_tcp(
    remote_ip: [u8; 4],
    remote_port: u16,
    local_port: u16,
    payload: &mut [u8],
) -> Option<TcpPacket> {
    // Дрейнуємо всі фрейми з буфера NIC поки не знайдемо наш TCP пакет.
    // Без цього один чужий фрейм (ARP, DNS тощо) «з'їдає» слот і наш пакет губиться.
    let mut rx = [0u8; 1536];
    loop {
        let len = net::poll_frame(&mut rx)?;
        unsafe {
            LAST_PACKET_IO = true;
        }
        let (ip_start, ip_len) = match ipv4_packet(&rx[..len], ANY_IP, LOCAL_IP, IP_PROTO_TCP) {
            Some(v) => v,
            None => continue, // чужий протокол — читаємо далі
        };
        if ip_len < 40 {
            continue;
        }
        let ihl = ((rx[ip_start] & 0x0F) as usize) * 4;
        let tcp_start = ip_start + ihl;
        if tcp_start + 20 > ip_start + ip_len {
            continue;
        }
        if read_u16_be(&rx[tcp_start..tcp_start + 2]) != remote_port
            || read_u16_be(&rx[tcp_start + 2..tcp_start + 4]) != local_port
        {
            continue; // TCP пакет але не наш порт — читаємо далі
        }
        let tcp_h = ((rx[tcp_start + 12] >> 4) as usize) * 4;
        if tcp_h < 20 || tcp_start + tcp_h > ip_start + ip_len {
            continue;
        }
        let data_start = tcp_start + tcp_h;
        let payload_len = ip_start + ip_len - data_start;
        let take = payload_len.min(payload.len());
        for i in 0..take {
            payload[i] = rx[data_start + i];
        }
        return Some(TcpPacket {
            flags: rx[tcp_start + 13],
            seq: read_u32_be(&rx[tcp_start + 4..tcp_start + 8]),
            ack: read_u32_be(&rx[tcp_start + 8..tcp_start + 12]),
            payload_len,
            copied_len: take,
        });
    }
}

fn udp_payload(
    frame: &[u8],
    remote_ip: [u8; 4],
    remote_port: u16,
    local_port: u16,
) -> Option<(usize, usize)> {
    let (ip_start, ip_len) = ipv4_packet(frame, ANY_IP, LOCAL_IP, IP_PROTO_UDP)?;
    let ihl = ((frame[ip_start] & 0x0F) as usize) * 4;
    let udp = ip_start + ihl;
    if udp + 8 > ip_start + ip_len {
        return None;
    }
    if read_u16_be(&frame[udp..udp + 2]) != remote_port
        || read_u16_be(&frame[udp + 2..udp + 4]) != local_port
    {
        return None;
    }
    let udp_len = read_u16_be(&frame[udp + 4..udp + 6]) as usize;
    if udp_len < 8 || udp + udp_len > ip_start + ip_len {
        return None;
    }
    Some((udp + 8, udp_len - 8))
}

fn ipv4_packet(
    frame: &[u8],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    proto: u8,
) -> Option<(usize, usize)> {
    if frame.len() < 34 || read_u16_be(&frame[12..14]) != ETH_TYPE_IPV4 {
        return None;
    }
    let ip = 14usize;
    let ihl = ((frame[ip] & 0x0F) as usize) * 4;
    if ihl < 20 || ip + ihl > frame.len() {
        return None;
    }
    let total = read_u16_be(&frame[ip + 2..ip + 4]) as usize;
    if total < ihl || ip + total > frame.len() {
        return None;
    }
    if frame[ip + 9] != proto {
        return None;
    }
    if frame[ip + 16..ip + 20] != dst_ip {
        return None;
    }
    if src_ip != ANY_IP && frame[ip + 12..ip + 16] != src_ip {
        return None;
    }
    let frag = read_u16_be(&frame[ip + 6..ip + 8]);
    if (frag & 0x3FFF) != 0 {
        return None;
    }
    Some((ip, total))
}

fn send_dhcp_discover(mac: [u8; 6]) -> bool {
    if mac == [0; 6] {
        return false;
    }

    let mut frame = [0u8; 342];
    let eth_len = 14usize;
    let ip_len = 20usize;
    let udp_len = 8usize;
    let dhcp_len = 300usize;
    let total_len = eth_len + ip_len + udp_len + dhcp_len;

    for i in 0..6 {
        frame[i] = 0xFF;
        frame[6 + i] = mac[i];
    }
    write_u16_be(&mut frame[12..14], ETH_TYPE_IPV4);

    let ip = eth_len;
    frame[ip] = 0x45;
    frame[ip + 1] = 0;
    write_u16_be(&mut frame[ip + 2..ip + 4], (ip_len + udp_len + dhcp_len) as u16);
    write_u16_be(&mut frame[ip + 4..ip + 6], next_ip_id());
    write_u16_be(&mut frame[ip + 6..ip + 8], 0);
    frame[ip + 8] = 64;
    frame[ip + 9] = IP_PROTO_UDP;
    frame[ip + 12..ip + 16].copy_from_slice(&[0, 0, 0, 0]);
    frame[ip + 16..ip + 20].copy_from_slice(&[255, 255, 255, 255]);
    let csum = ipv4_checksum(&frame[ip..ip + ip_len]);
    write_u16_be(&mut frame[ip + 10..ip + 12], csum);

    let udp = ip + ip_len;
    write_u16_be(&mut frame[udp..udp + 2], DHCP_CLIENT_PORT);
    write_u16_be(&mut frame[udp + 2..udp + 4], DHCP_SERVER_PORT);
    write_u16_be(&mut frame[udp + 4..udp + 6], (udp_len + dhcp_len) as u16);
    write_u16_be(&mut frame[udp + 6..udp + 8], 0);

    let dhcp = udp + udp_len;
    frame[dhcp] = 1;
    frame[dhcp + 1] = 1;
    frame[dhcp + 2] = 6;
    frame[dhcp + 3] = 0;
    write_u32_be(&mut frame[dhcp + 4..dhcp + 8], DHCP_XID);
    write_u16_be(&mut frame[dhcp + 10..dhcp + 12], 0x8000);
    frame[dhcp + 28..dhcp + 34].copy_from_slice(&mac);
    frame[dhcp + 236..dhcp + 240].copy_from_slice(&[99, 130, 83, 99]);
    frame[dhcp + 240] = 53;
    frame[dhcp + 241] = 1;
    frame[dhcp + 242] = 1;
    frame[dhcp + 243] = 55;
    frame[dhcp + 244] = 4;
    frame[dhcp + 245] = 1;
    frame[dhcp + 246] = 3;
    frame[dhcp + 247] = 6;
    frame[dhcp + 248] = 15;
    frame[dhcp + 249] = 255;

    net::send_frame(&frame[..total_len])
}

fn write_eth(buf: &mut [u8], dst: [u8; 6], src: [u8; 6], typ: u16) {
    if buf.len() < 14 {
        return;
    }
    buf[0..6].copy_from_slice(&dst);
    buf[6..12].copy_from_slice(&src);
    write_u16_be(&mut buf[12..14], typ);
}

fn write_ipv4_header(buf: &mut [u8], proto: u8, total_len: u16, src: [u8; 4], dst: [u8; 4]) {
    if buf.len() < 20 {
        return;
    }
    clear(&mut buf[..20]);
    buf[0] = 0x45;
    write_u16_be(&mut buf[2..4], total_len);
    write_u16_be(&mut buf[4..6], next_ip_id());
    write_u16_be(&mut buf[6..8], 0x4000);
    buf[8] = 64;
    buf[9] = proto;
    buf[12..16].copy_from_slice(&src);
    buf[16..20].copy_from_slice(&dst);
    let csum = ipv4_checksum(&buf[..20]);
    write_u16_be(&mut buf[10..12], csum);
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    checksum_finish(checksum_bytes(0, header))
}

fn tcp_checksum(src: [u8; 4], dst: [u8; 4], tcp_seg: &[u8]) -> u16 {
    let mut sum = 0u32;
    sum = checksum_bytes(sum, &src);
    sum = checksum_bytes(sum, &dst);
    sum = sum.wrapping_add(IP_PROTO_TCP as u32);
    sum = sum.wrapping_add(tcp_seg.len() as u32);
    sum = checksum_bytes(sum, tcp_seg);
    checksum_finish(sum)
}

fn checksum_bytes(mut sum: u32, bytes: &[u8]) -> u32 {
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        sum = sum.wrapping_add(((bytes[i] as u32) << 8) | bytes[i + 1] as u32);
        i += 2;
    }
    if i < bytes.len() {
        sum = sum.wrapping_add((bytes[i] as u32) << 8);
    }
    sum
}

fn checksum_finish(mut sum: u32) -> u16 {
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn parse_ipv4(s: &[u8], out: &mut [u8; 4]) -> bool {
    let mut part = 0usize;
    let mut value = 0u32;
    let mut digits = 0usize;
    for &b in s {
        if b == b'.' {
            if digits == 0 || value > 255 || part >= 4 {
                return false;
            }
            out[part] = value as u8;
            part += 1;
            value = 0;
            digits = 0;
        } else if b'0' <= b && b <= b'9' {
            value = value * 10 + (b - b'0') as u32;
            digits += 1;
            if value > 255 {
                return false;
            }
        } else {
            return false;
        }
    }
    if digits == 0 || value > 255 || part != 3 {
        return false;
    }
    out[3] = value as u8;
    true
}

fn same_subnet(ip: [u8; 4]) -> bool {
    ip[0] == LOCAL_IP[0] && ip[1] == LOCAL_IP[1] && ip[2] == LOCAL_IP[2]
}

fn find_body_offset(buf: &[u8]) -> Option<usize> {
    let mut i = 0usize;
    while i + 3 < buf.len() {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i + 4);
        }
        i += 1;
    }
    None
}

fn parse_status_code(buf: &[u8]) -> u16 {
    if buf.len() < 12 || !starts_with_ci(buf, b"HTTP/") {
        return 0;
    }
    let mut i = 0usize;
    while i < buf.len() && buf[i] != b' ' {
        i += 1;
    }
    while i < buf.len() && buf[i] == b' ' {
        i += 1;
    }
    let mut code = 0u16;
    let mut digits = 0usize;
    while i < buf.len() && digits < 3 && b'0' <= buf[i] && buf[i] <= b'9' {
        code = code * 10 + (buf[i] - b'0') as u16;
        i += 1;
        digits += 1;
    }
    if digits == 3 {
        code
    } else {
        0
    }
}

fn next_local_port() -> u16 {
    unsafe {
        let p = NEXT_LOCAL_PORT;
        NEXT_LOCAL_PORT = if NEXT_LOCAL_PORT >= 60000 {
            40000
        } else {
            NEXT_LOCAL_PORT + 1
        };
        p
    }
}

fn next_ip_id() -> u16 {
    unsafe {
        NEXT_IP_ID = NEXT_IP_ID.wrapping_add(1);
        NEXT_IP_ID
    }
}

fn write_bytes(buf: &mut [u8], s: &[u8]) -> usize {
    let mut n = 0usize;
    while n < s.len() && n < buf.len() {
        buf[n] = s[n];
        n += 1;
    }
    n
}

fn starts_with_ci(buf: &[u8], pref: &[u8]) -> bool {
    if buf.len() < pref.len() {
        return false;
    }
    for i in 0..pref.len() {
        if ascii_lower(buf[i]) != ascii_lower(pref[i]) {
            return false;
        }
    }
    true
}

fn ascii_lower(b: u8) -> u8 {
    if b'A' <= b && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

fn clear(buf: &mut [u8]) {
    for b in buf {
        *b = 0;
    }
}

fn read_u16_be(buf: &[u8]) -> u16 {
    if buf.len() < 2 {
        0
    } else {
        ((buf[0] as u16) << 8) | buf[1] as u16
    }
}

fn read_u32_be(buf: &[u8]) -> u32 {
    if buf.len() < 4 {
        0
    } else {
        ((buf[0] as u32) << 24)
            | ((buf[1] as u32) << 16)
            | ((buf[2] as u32) << 8)
            | buf[3] as u32
    }
}

fn write_u16_be(buf: &mut [u8], value: u16) {
    if buf.len() >= 2 {
        buf[0] = (value >> 8) as u8;
        buf[1] = value as u8;
    }
}

fn write_u24_be(buf: &mut [u8], value: u32) {
    if buf.len() >= 3 {
        buf[0] = (value >> 16) as u8;
        buf[1] = (value >> 8) as u8;
        buf[2] = value as u8;
    }
}

fn write_u32_be(buf: &mut [u8], value: u32) {
    if buf.len() >= 4 {
        buf[0] = (value >> 24) as u8;
        buf[1] = (value >> 16) as u8;
        buf[2] = (value >> 8) as u8;
        buf[3] = value as u8;
    }
}

mod tcp {
    pub const FIN: u8 = 0x01;
    pub const SYN: u8 = 0x02;
    pub const RST: u8 = 0x04;
    pub const PSH: u8 = 0x08;
    pub const ACK: u8 = 0x10;
}
