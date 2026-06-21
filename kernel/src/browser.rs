// ============================================================================
// Файл: browser.rs (виправлена версія з покращеним дизайном)
// ============================================================================

use crate::display::{self, Framebuffer};
use crate::drivers::net::{self, NetKind, NetState};
use crate::image;
use crate::netstack::{self, FetchCode};
use crate::system;
use crate::window;

const MAX_URL: usize = 256;
const MAX_LINES: usize = 1024;
const MAX_COLS: usize = 88;
const LINE_HEIGHT: usize = 18;
const PAD: usize = 12;
const SCROLL_W: usize = 14;
const BTN_H: usize = 16;
const FETCH_BUF_SIZE: usize = 131072;
const CSS_FETCH_BUF_SIZE: usize = 131072;
const IMAGE_FETCH_BUF_SIZE: usize = 262144;
const HTTP_RETRY_ATTEMPTS: usize = 18;
const NETWORK_WARM_TICKS: usize = 96;
const MAX_LINKS: usize = 64;
const MAX_STYLE_STACK: usize = 24;
const MAX_PAD_LINES: u8 = 8;
const MAX_FONT_NAME: usize = 32;
const NO_LINK: u8 = 0xFF;
const STYLE_NONE: u32 = 0xFFFF_FFFE;
const KIND_NORMAL: u8 = 0;
const KIND_HEADING_1: u8 = 1;
const KIND_HEADING_2: u8 = 2;
const KIND_HEADING_3: u8 = 3;
const KIND_HEADING_4: u8 = 4;
const KIND_BLOCK: u8 = 5;
const KIND_LINK: u8 = 6;
const KIND_BUTTON: u8 = 7;
const KIND_IMAGE: u8 = 8;
const KIND_MUTED: u8 = 9;
const KIND_STRONG: u8 = 10;
const KIND_IMAGE_SPACER: u8 = 11;
const IMAGE_BLOCK_LINES: usize = 6;
const IMAGE_BLOCK_H: usize = LINE_HEIGHT * IMAGE_BLOCK_LINES;
const MAX_CSS_RULES: usize = 256;
const MAX_CSS_NAME: usize = 24;
const MAX_CSS_VARS: usize = 32;
const MAX_CSS_VAR_VALUE: usize = 48;
const CSS_SEL_TAG: u8 = 0;
const CSS_SEL_CLASS: u8 = 1;
const CSS_SEL_ID: u8 = 2;
const ALIGN_LEFT: u8 = 0;
const ALIGN_CENTER: u8 = 1;
const ALIGN_RIGHT: u8 = 2;

static mut FETCH_BUF: [u8; FETCH_BUF_SIZE] = [0; FETCH_BUF_SIZE];
static mut CSS_FETCH_BUF: [u8; CSS_FETCH_BUF_SIZE] = [0; CSS_FETCH_BUF_SIZE];
static mut IMAGE_FETCH_BUF: [u8; IMAGE_FETCH_BUF_SIZE] = [0; IMAGE_FETCH_BUF_SIZE];
fn warm_network() {
    for _ in 0..NETWORK_WARM_TICKS {
        let _ = netstack::tick();
    }
}

// ============================================================================
// CSS support
// ============================================================================

#[derive(Copy, Clone, Default)]
struct CssStyles {
    body_fg: Option<u32>,
    body_bg: Option<u32>,
    heading_fg: Option<u32>,
    heading_bg: Option<u32>,
    link_fg: Option<u32>,
    link_bg: Option<u32>,
    button_fg: Option<u32>,
    button_bg: Option<u32>,
    code_fg: Option<u32>,
    code_bg: Option<u32>,
    body_align: Option<u8>,
}

#[derive(Copy, Clone)]
struct CssRule {
    selector_kind: u8,
    name: [u8; MAX_CSS_NAME],
    name_len: usize,
    fg: Option<u32>,
    bg: Option<u32>,
    kind: u8,
    has_kind: bool,
    hidden: bool,
    align: u8,
    has_align: bool,
    pad_before: u8,
    pad_after: u8,
    image_url: [u8; MAX_URL],
    image_url_len: usize,
}

#[derive(Copy, Clone)]
struct CssVar {
    name: [u8; MAX_CSS_NAME],
    name_len: usize,
    value: [u8; MAX_CSS_VAR_VALUE],
    value_len: usize,
}

impl CssVar {
    const EMPTY: CssVar = CssVar {
        name: [0; MAX_CSS_NAME],
        name_len: 0,
        value: [0; MAX_CSS_VAR_VALUE],
        value_len: 0,
    };
}

impl CssRule {
    const EMPTY: CssRule = CssRule {
        selector_kind: CSS_SEL_TAG,
        name: [0; MAX_CSS_NAME],
        name_len: 0,
        fg: None,
        bg: None,
        kind: KIND_NORMAL,
        has_kind: false,
        hidden: false,
        align: ALIGN_LEFT,
        has_align: false,
        pad_before: 0,
        pad_after: 0,
        image_url: [0; MAX_URL],
        image_url_len: 0,
    };
}

#[derive(Copy, Clone)]
struct CssDecl {
    fg: Option<u32>,
    bg: Option<u32>,
    kind: u8,
    has_kind: bool,
    hidden: bool,
    align: u8,
    has_align: bool,
    pad_before: u8,
    pad_after: u8,
    image_url: [u8; MAX_URL],
    image_url_len: usize,
}

impl Default for CssDecl {
    fn default() -> Self {
        Self {
            fg: None,
            bg: None,
            kind: KIND_NORMAL,
            has_kind: false,
            hidden: false,
            align: ALIGN_LEFT,
            has_align: false,
            pad_before: 0,
            pad_after: 0,
            image_url: [0; MAX_URL],
            image_url_len: 0,
        }
    }
}

static mut CSS_STYLES: CssStyles = CssStyles {
    body_fg: None,
    body_bg: None,
    heading_fg: None,
    heading_bg: None,
    link_fg: None,
    link_bg: None,
    button_fg: None,
    button_bg: None,
    code_fg: None,
    code_bg: None,
    body_align: None,
};

static mut CSS_RULES: [CssRule; MAX_CSS_RULES] = [CssRule::EMPTY; MAX_CSS_RULES];
static mut CSS_RULE_COUNT: usize = 0;
static mut CSS_VARS: [CssVar; MAX_CSS_VARS] = [CssVar::EMPTY; MAX_CSS_VARS];
static mut CSS_VAR_COUNT: usize = 0;
static mut CSS_WEB_FONT_DECLARED: bool = false;
static mut CSS_WEB_FONT_READY: bool = false;
static mut CSS_WEB_FONT_NAME: [u8; MAX_FONT_NAME] = [0; MAX_FONT_NAME];
static mut CSS_WEB_FONT_NAME_LEN: usize = 0;
static mut CSS_WEB_FONT_URL: [u8; MAX_URL] = [0; MAX_URL];
static mut CSS_WEB_FONT_URL_LEN: usize = 0;
static mut CSS_WEB_FONT_BASE: [u8; MAX_URL] = [0; MAX_URL];
static mut CSS_WEB_FONT_BASE_LEN: usize = 0;

fn parse_css_hex_color(s: &[u8]) -> Option<u32> {
    let hex = if !s.is_empty() && s[0] == b'#' { &s[1..] } else { s };
    let mut len = 0usize;
    while len < hex.len() && hex_nibble(hex[len]).is_some() {
        len += 1;
    }
    if len >= 6 {
        let r = hex_pair(hex[0], hex[1])? as u32;
        let g = hex_pair(hex[2], hex[3])? as u32;
        let b = hex_pair(hex[4], hex[5])? as u32;
        return Some((r << 16) | (g << 8) | b);
    }
    if len >= 3 {
        let r = hex_nibble(hex[0])?;
        let g = hex_nibble(hex[1])?;
        let b = hex_nibble(hex[2])?;
        return Some((((r * 17) as u32) << 16) | (((g * 17) as u32) << 8) | ((b * 17) as u32));
    }
    None
}

fn parse_css_value(value: &[u8]) -> Option<u32> {
    let v = trim_ascii(value);
    if v.is_empty() {
        return None;
    }
    if starts_with_ci(v, b"var(") {
        if let Some(c) = parse_css_var_color(v) {
            return Some(c);
        }
    }
    let mut var_scan = 0usize;
    while let Some(pos) = find_subslice_ci(v, b"var(", var_scan) {
        if let Some(c) = parse_css_var_color(&v[pos..]) {
            return Some(c);
        }
        var_scan = pos + 4;
    }
    parse_css_color_literal(v)
}

fn parse_css_color_literal(v: &[u8]) -> Option<u32> {
    let v = trim_ascii(v);
    if v.is_empty() {
        return None;
    }
    if v[0] == b'#' {
        parse_css_hex_color(v)
    } else if starts_with_ci(v, b"rgb(") {
        parse_rgb_color(&v[4..])
    } else if starts_with_ci(v, b"rgba(") {
        parse_rgb_color(&v[5..])
    } else {
        let token = first_css_token(v);
        if let Some(c) = named_color(token) {
            return Some(c);
        }
        let mut i = 0usize;
        while i < v.len() {
            if v[i] == b'#' {
                if let Some(c) = parse_css_hex_color(&v[i..]) {
                    return Some(c);
                }
            }
            if starts_with_ci(&v[i..], b"rgb(") {
                if let Some(c) = parse_rgb_color(&v[i + 4..]) {
                    return Some(c);
                }
            }
            if starts_with_ci(&v[i..], b"rgba(") {
                if let Some(c) = parse_rgb_color(&v[i + 5..]) {
                    return Some(c);
                }
            }
            i += 1;
        }
        None
    }
}

fn parse_css_var_color(value: &[u8]) -> Option<u32> {
    if !starts_with_ci(value, b"var(") {
        return None;
    }
    let mut i = 4usize;
    while i < value.len() && is_css_ws(value[i]) {
        i += 1;
    }
    let name_start = i;
    while i < value.len() && value[i] != b',' && value[i] != b')' {
        i += 1;
    }
    let name = trim_ascii(&value[name_start..i]);
    if !name.is_empty() {
        if let Some(c) = lookup_css_var_color(name) {
            return Some(c);
        }
    }
    if i < value.len() && value[i] == b',' {
        i += 1;
        let fallback_start = i;
        let mut depth = 0usize;
        while i < value.len() {
            if value[i] == b'(' {
                depth += 1;
            } else if value[i] == b')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            i += 1;
        }
        return parse_css_color_literal(&value[fallback_start..i]);
    }
    None
}

fn lookup_css_var_color(name: &[u8]) -> Option<u32> {
    unsafe {
        for i in (0..CSS_VAR_COUNT).rev() {
            let var = CSS_VARS[i];
            if var.name_len == name.len() && slice_eq_ci(&var.name[..var.name_len], name) {
                return parse_css_color_literal(&var.value[..var.value_len]);
            }
        }
    }
    None
}

fn store_css_var(name: &[u8], value: &[u8]) {
    let name = trim_ascii(name);
    let value = trim_ascii(value);
    if name.len() < 3 || !name.starts_with(b"--") || value.is_empty() {
        return;
    }
    unsafe {
        for i in 0..CSS_VAR_COUNT {
            if CSS_VARS[i].name_len == name.len()
                && slice_eq_ci(&CSS_VARS[i].name[..CSS_VARS[i].name_len], name)
            {
                CSS_VARS[i].value_len = value.len().min(MAX_CSS_VAR_VALUE);
                CSS_VARS[i].value = [0; MAX_CSS_VAR_VALUE];
                CSS_VARS[i].value[..CSS_VARS[i].value_len].copy_from_slice(&value[..CSS_VARS[i].value_len]);
                return;
            }
        }
        if CSS_VAR_COUNT >= MAX_CSS_VARS {
            return;
        }
        let idx = CSS_VAR_COUNT;
        CSS_VAR_COUNT += 1;
        CSS_VARS[idx] = CssVar::EMPTY;
        CSS_VARS[idx].name_len = name.len().min(MAX_CSS_NAME);
        for i in 0..CSS_VARS[idx].name_len {
            CSS_VARS[idx].name[i] = ascii_lower(name[i]);
        }
        CSS_VARS[idx].value_len = value.len().min(MAX_CSS_VAR_VALUE);
        CSS_VARS[idx].value[..CSS_VARS[idx].value_len].copy_from_slice(&value[..CSS_VARS[idx].value_len]);
    }
}

fn parse_rgb_color(value: &[u8]) -> Option<u32> {
    let mut nums = [0u32; 3];
    let mut count = 0;
    let mut i = 0;
    while i < value.len() && count < 3 {
        while i < value.len() && (value[i] == b' ' || value[i] == b',' || value[i] == b'(') {
            i += 1;
        }
        let mut n = 0u32;
        while i < value.len() && value[i] >= b'0' && value[i] <= b'9' {
            n = n * 10 + (value[i] - b'0') as u32;
            i += 1;
        }
        nums[count] = n.min(255);
        count += 1;
        while i < value.len() && value[i] != b',' && value[i] != b')' {
            i += 1;
        }
        if i < value.len() && value[i] == b')' {
            break;
        }
        i += 1;
    }
    if count == 3 {
        Some((nums[0] << 16) | (nums[1] << 8) | nums[2])
    } else {
        None
    }
}

fn named_color(value: &[u8]) -> Option<u32> {
    match value {
        b"transparent" => None,
        b"black" => Some(0x000000),
        b"white" => Some(0xFFFFFF),
        b"red" => Some(0xFF4444),
        b"green" => Some(0x44FF44),
        b"blue" => Some(0x4444FF),
        b"yellow" => Some(0xFFFF44),
        b"cyan" => Some(0x44FFFF),
        b"magenta" => Some(0xFF44FF),
        b"gray" | b"grey" => Some(0x888888),
        b"darkgray" => Some(0x444444),
        b"lightgray" => Some(0xCCCCCC),
        b"orange" => Some(0xFF8844),
        b"purple" => Some(0x8844FF),
        b"brown" => Some(0x884400),
        b"pink" => Some(0xFF88CC),
        b"navy" => Some(0x000080),
        b"teal" => Some(0x008080),
        b"lime" => Some(0x00FF00),
        b"olive" => Some(0x808000),
        b"maroon" => Some(0x800000),
        b"silver" => Some(0xC0C0C0),
        b"darkgrey" => Some(0x444444),
        b"dimgray" | b"dimgrey" => Some(0x696969),
        b"slategray" | b"slategrey" => Some(0x708090),
        b"lightslategray" | b"lightslategrey" => Some(0x778899),
        b"whitesmoke" => Some(0xF5F5F5),
        b"gainsboro" => Some(0xDCDCDC),
        b"aliceblue" => Some(0xF0F8FF),
        b"azure" => Some(0xF0FFFF),
        b"beige" => Some(0xF5F5DC),
        b"ivory" => Some(0xFFFFF0),
        b"lavender" => Some(0xE6E6FA),
        b"gold" => Some(0xFFD700),
        b"coral" => Some(0xFF7F50),
        b"tomato" => Some(0xFF6347),
        b"orangered" => Some(0xFF4500),
        b"indigo" => Some(0x4B0082),
        b"violet" => Some(0xEE82EE),
        b"skyblue" => Some(0x87CEEB),
        b"deepskyblue" => Some(0x00BFFF),
        b"dodgerblue" => Some(0x1E90FF),
        b"royalblue" => Some(0x4169E1),
        b"steelblue" => Some(0x4682B4),
        b"cornflowerblue" => Some(0x6495ED),
        b"crimson" => Some(0xDC143C),
        b"seagreen" => Some(0x2E8B57),
        b"mediumseagreen" => Some(0x3CB371),
        b"forestgreen" => Some(0x228B22),
        _ => None,
    }
}

fn first_css_token(value: &[u8]) -> &[u8] {
    let mut end = 0usize;
    while end < value.len() {
        let b = value[end];
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' || b == b';' || b == b',' || b == b'!'
            || b == b')'
        {
            break;
        }
        end += 1;
    }
    &value[..end]
}

fn parse_css_declarations(decls: &[u8]) -> CssDecl {
    let mut i = 0;
    let mut out = CssDecl::default();
    while i < decls.len() {
        while i < decls.len()
            && (decls[i] == b' ' || decls[i] == b'\n' || decls[i] == b'\r' || decls[i] == b'\t')
        {
            i += 1;
        }
        let prop_start = i;
        while i < decls.len() && decls[i] != b':' && decls[i] != b';' && decls[i] != b'}' {
            i += 1;
        }
        let prop_end = i;
        while i < decls.len() && decls[i] != b':' {
            i += 1;
        }
        if i >= decls.len() {
            break;
        }
        i += 1;
        while i < decls.len()
            && (decls[i] == b' ' || decls[i] == b'\n' || decls[i] == b'\r' || decls[i] == b'\t')
        {
            i += 1;
        }
        let val_start = i;
        while i < decls.len() && decls[i] != b';' && decls[i] != b'}' {
            i += 1;
        }
        let val_end = i;
        if i < decls.len() && decls[i] == b';' {
            i += 1;
        }

        let prop = trim_ascii(&decls[prop_start..prop_end]);
        let val = trim_ascii(&decls[val_start..val_end]);

        if prop.starts_with(b"--") {
            store_css_var(prop, val);
        } else if slice_eq_ci(prop, b"color") {
            if let Some(c) = parse_css_value(val) {
                out.fg = Some(c);
            }
        } else if slice_eq_ci(prop, b"background-color") || slice_eq_ci(prop, b"background") {
            if let Some(c) = parse_css_value(val) {
                out.bg = Some(c);
            }
            let url_len = extract_css_url(val, &mut out.image_url);
            if url_len > 0 {
                out.image_url_len = url_len;
            }
        } else if slice_eq_ci(prop, b"background-image") || slice_eq_ci(prop, b"list-style-image") {
            let url_len = extract_css_url(val, &mut out.image_url);
            if url_len > 0 {
                out.image_url_len = url_len;
            }
        } else if slice_eq_ci(prop, b"display") {
            if contains_word_ci(val, b"none") {
                out.hidden = true;
            } else if contains_word_ci(val, b"inline-block") || contains_word_ci(val, b"button") {
                out.kind = KIND_BUTTON;
                out.has_kind = true;
            }
        } else if slice_eq_ci(prop, b"visibility") {
            if contains_word_ci(val, b"hidden") || contains_word_ci(val, b"collapse") {
                out.hidden = true;
            }
        } else if slice_eq_ci(prop, b"text-decoration") {
            if contains_word_ci(val, b"underline") {
                out.kind = KIND_LINK;
                out.has_kind = true;
            }
        } else if slice_eq_ci(prop, b"text-align") {
            if contains_word_ci(val, b"center") {
                out.align = ALIGN_CENTER;
                out.has_align = true;
            } else if contains_word_ci(val, b"right") {
                out.align = ALIGN_RIGHT;
                out.has_align = true;
            } else if contains_word_ci(val, b"left") || contains_word_ci(val, b"start") {
                out.align = ALIGN_LEFT;
                out.has_align = true;
            }
        } else if slice_eq_ci(prop, b"font-size") {
            let px = css_px_value(val);
            if px >= 32 {
                out.kind = KIND_HEADING_1;
                out.has_kind = true;
            } else if px >= 24 {
                out.kind = KIND_HEADING_2;
                out.has_kind = true;
            } else if px >= 18 {
                out.kind = KIND_HEADING_3;
                out.has_kind = true;
            }
        } else if slice_eq_ci(prop, b"font-weight") {
            if contains_word_ci(val, b"bold")
                || starts_with_ci(val, b"7")
                || starts_with_ci(val, b"8")
                || starts_with_ci(val, b"9")
            {
                out.kind = KIND_STRONG;
                out.has_kind = true;
            }
        } else if slice_eq_ci(prop, b"font-style") {
            if contains_word_ci(val, b"italic") || contains_word_ci(val, b"oblique") {
                out.kind = KIND_STRONG;
                out.has_kind = true;
            }
        } else if slice_eq_ci(prop, b"font-family") {
            unsafe {
                if CSS_WEB_FONT_DECLARED && CSS_WEB_FONT_NAME_LEN > 0
                    && contains_subslice_ci(val, &CSS_WEB_FONT_NAME[..CSS_WEB_FONT_NAME_LEN])
                {
                    out.kind = KIND_STRONG;
                    out.has_kind = true;
                }
            }
        } else if slice_eq_ci(prop, b"padding") {
            let pad = css_pad_lines(val);
            out.pad_before = pad;
            out.pad_after = pad;
        } else if slice_eq_ci(prop, b"padding-top") {
            out.pad_before = css_pad_lines(val);
        } else if slice_eq_ci(prop, b"padding-bottom") {
            out.pad_after = css_pad_lines(val);
        } else if slice_eq_ci(prop, b"margin") {
            let pad = css_pad_lines(val).min(3);
            out.pad_before = out.pad_before.max(pad);
            out.pad_after = out.pad_after.max(pad);
        } else if slice_eq_ci(prop, b"margin-top") {
            out.pad_before = out.pad_before.max(css_pad_lines(val).min(3));
        } else if slice_eq_ci(prop, b"margin-bottom") {
            out.pad_after = out.pad_after.max(css_pad_lines(val).min(3));
        } else if slice_eq_ci(prop, b"height") || slice_eq_ci(prop, b"min-height") {
            let pad = (css_pad_lines(val) / 2).min(MAX_PAD_LINES);
            out.pad_before = pad;
            out.pad_after = pad;
        } else if slice_eq_ci(prop, b"line-height") {
            let pad = (css_pad_lines(val) / 2).min(2);
            out.pad_before = out.pad_before.max(pad);
            out.pad_after = out.pad_after.max(pad);
        } else if slice_eq_ci(prop, b"border")
            || slice_eq_ci(prop, b"border-top")
            || slice_eq_ci(prop, b"border-bottom")
            || slice_eq_ci(prop, b"border-radius")
            || slice_eq_ci(prop, b"box-shadow")
        {
            out.kind = KIND_BLOCK;
            out.has_kind = true;
        }
    }
    out
}

fn parse_css_rules(sheet: &[u8], base_url: Option<&[u8]>) {
    parse_font_faces(sheet, base_url);
    parse_css_rule_block(sheet, 0, base_url);
}

fn parse_css_rule_block(sheet: &[u8], depth: usize, base_url: Option<&[u8]>) {
    if depth > 2 {
        return;
    }
    let mut i = 0usize;
    while i < sheet.len() {
        while i < sheet.len() && is_css_ws(sheet[i]) {
            i += 1;
        }
        if i + 1 < sheet.len() && sheet[i] == b'/' && sheet[i + 1] == b'*' {
            i = skip_css_comment(sheet, i + 2);
            continue;
        }
        if i >= sheet.len() {
            break;
        }

        if sheet[i] == b'@' {
            let at_start = i;
            while i < sheet.len() && sheet[i] != b'{' && sheet[i] != b';' {
                i += 1;
            }
            if i >= sheet.len() {
                break;
            }
            if sheet[i] == b';' {
                i += 1;
                continue;
            }
            let at_rule = trim_ascii(&sheet[at_start..i]);
            let open = i;
            let close = match find_matching_css_brace(sheet, open) {
                Some(v) => v,
                None => break,
            };
            if at_rule_allows_nested_rules(at_rule) && open + 1 < close {
                parse_css_rule_block(&sheet[open + 1..close], depth + 1, base_url);
            }
            i = close + 1;
            continue;
        }

        let sel_start = i;
        while i < sheet.len() && sheet[i] != b'{' {
            if i + 1 < sheet.len() && sheet[i] == b'/' && sheet[i + 1] == b'*' {
                i = skip_css_comment(sheet, i + 2);
                continue;
            }
            i += 1;
        }
        if i >= sheet.len() {
            break;
        }
        let sel_end = i;
        let open = i;
        let close = match find_matching_css_brace(sheet, open) {
            Some(v) => v,
            None => break,
        };
        let selector = &sheet[sel_start..sel_end];
        let decls = &sheet[open + 1..close];
        let mut decl = parse_css_declarations(decls);
        if decl.image_url_len > 0 {
            if let Some(base) = base_url {
                let mut resolved = [0u8; MAX_URL];
                let resolved_len = build_url_with_base(base, &decl.image_url[..decl.image_url_len], &mut resolved);
                if resolved_len > 0 {
                    decl.image_url = [0; MAX_URL];
                    decl.image_url_len = resolved_len.min(MAX_URL);
                    decl.image_url[..decl.image_url_len].copy_from_slice(&resolved[..decl.image_url_len]);
                }
            }
        }
        add_css_selectors(selector, decl);
        i = close + 1;
    }
}

fn at_rule_allows_nested_rules(rule: &[u8]) -> bool {
    starts_with_ci(rule, b"@media")
        || starts_with_ci(rule, b"@supports")
        || starts_with_ci(rule, b"@document")
        || starts_with_ci(rule, b"@layer")
        || starts_with_ci(rule, b"@container")
}

fn find_matching_css_brace(sheet: &[u8], open: usize) -> Option<usize> {
    if open >= sheet.len() || sheet[open] != b'{' {
        return None;
    }
    let mut depth = 1usize;
    let mut i = open + 1;
    let mut quote = 0u8;
    while i < sheet.len() {
        let b = sheet[i];
        if quote != 0 {
            if b == b'\\' {
                i = i.saturating_add(2);
                continue;
            }
            if b == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if i + 1 < sheet.len() && b == b'/' && sheet[i + 1] == b'*' {
            i = skip_css_comment(sheet, i + 2);
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = b;
        } else if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn parse_font_faces(sheet: &[u8], base_url: Option<&[u8]>) {
    let mut i = 0usize;
    while let Some(face) = find_subslice_ci(sheet, b"@font-face", i) {
        let open = match find_byte(sheet, face, b'{') {
            Some(v) => v,
            None => break,
        };
        let close = match find_byte(sheet, open + 1, b'}') {
            Some(v) => v,
            None => break,
        };
        let decls = &sheet[open + 1..close];
        unsafe {
            CSS_WEB_FONT_DECLARED = true;
        }
        if let Some((start, len)) = find_style_property(decls, b"font-family") {
            let value = &decls[start..start + len];
            unsafe {
                CSS_WEB_FONT_NAME_LEN = copy_css_string(&mut CSS_WEB_FONT_NAME, value);
            }
        }
        if let Some((start, len)) = find_style_property(decls, b"src") {
            let value = &decls[start..start + len];
            unsafe {
                CSS_WEB_FONT_URL_LEN = extract_css_url(value, &mut CSS_WEB_FONT_URL);
                if CSS_WEB_FONT_URL_LEN > 0 {
                    if let Some(base) = base_url {
                        CSS_WEB_FONT_BASE = [0; MAX_URL];
                        CSS_WEB_FONT_BASE_LEN = copy_limited(&mut CSS_WEB_FONT_BASE, base);
                    } else {
                        CSS_WEB_FONT_BASE = [0; MAX_URL];
                        CSS_WEB_FONT_BASE_LEN = 0;
                    }
                }
            }
        }
        i = close + 1;
    }
}

fn copy_css_string(out: &mut [u8], value: &[u8]) -> usize {
    let v = trim_ascii(value);
    let mut start = 0usize;
    let mut end = v.len();
    if start < end && (v[start] == b'"' || v[start] == b'\'') {
        let quote = v[start];
        start += 1;
        end = start;
        while end < v.len() && v[end] != quote {
            end += 1;
        }
    } else {
        while end > start && (v[end - 1] == b',' || v[end - 1] == b';') {
            end -= 1;
        }
    }
    copy_limited(out, &v[start..end])
}

fn extract_css_url(value: &[u8], out: &mut [u8; MAX_URL]) -> usize {
    let url_pos = match find_subslice_ci(value, b"url(", 0) {
        Some(v) => v + 4,
        None => return 0,
    };
    let mut i = url_pos;
    while i < value.len() && (value[i] == b' ' || value[i] == b'\t' || value[i] == b'\r' || value[i] == b'\n') {
        i += 1;
    }
    let quote = if i < value.len() && (value[i] == b'"' || value[i] == b'\'') {
        let q = value[i];
        i += 1;
        q
    } else {
        0
    };
    let start = i;
    while i < value.len() {
        if quote != 0 {
            if value[i] == quote {
                break;
            }
        } else if value[i] == b')' || value[i] == b' ' || value[i] == b'\t' || value[i] == b'\r' || value[i] == b'\n' {
            break;
        }
        i += 1;
    }
    copy_limited(out, &value[start..i])
}

fn apply_decl_to_role(styles: &mut CssStyles, selector: &[u8], decl: CssDecl) {
    if slice_eq_ci(selector, b"body") || slice_eq_ci(selector, b"html") || slice_eq_ci(selector, b"*")
    {
        if let Some(c) = decl.fg {
            styles.body_fg = Some(c);
        }
        if let Some(c) = decl.bg {
            styles.body_bg = Some(c);
        }
        if decl.has_align {
            styles.body_align = Some(decl.align);
        }
    } else if is_heading_selector(selector) {
        if let Some(c) = decl.fg {
            styles.heading_fg = Some(c);
        }
        if let Some(c) = decl.bg {
            styles.heading_bg = Some(c);
        }
    } else if slice_eq_ci(selector, b"a") {
        if let Some(c) = decl.fg {
            styles.link_fg = Some(c);
        }
        if let Some(c) = decl.bg {
            styles.link_bg = Some(c);
        }
    } else if slice_eq_ci(selector, b"button")
        || slice_eq_ci(selector, b"input")
        || slice_eq_ci(selector, b"select")
        || slice_eq_ci(selector, b"textarea")
    {
        if let Some(c) = decl.fg {
            styles.button_fg = Some(c);
        }
        if let Some(c) = decl.bg {
            styles.button_bg = Some(c);
        }
    } else if slice_eq_ci(selector, b"code") || slice_eq_ci(selector, b"pre") || slice_eq_ci(selector, b"kbd")
    {
        if let Some(c) = decl.fg {
            styles.code_fg = Some(c);
        }
        if let Some(c) = decl.bg {
            styles.code_bg = Some(c);
        }
    }
}

fn add_css_selectors(selectors: &[u8], decl: CssDecl) {
    let mut start = 0usize;
    while start < selectors.len() {
        let mut end = start;
        while end < selectors.len() && selectors[end] != b',' {
            end += 1;
        }
        let sel = selector_tail(trim_ascii(&selectors[start..end]));
        if !sel.is_empty() {
            add_css_selector(sel, decl);
        }
        start = end.saturating_add(1);
    }
}

fn add_css_selector(selector: &[u8], decl: CssDecl) {
    let mut styles = unsafe { CSS_STYLES };
    apply_decl_to_role(&mut styles, selector_tag_part(selector), decl);
    unsafe {
        CSS_STYLES = styles;
    }

    if selector[0] == b'.' {
        add_css_rule(CSS_SEL_CLASS, selector_name(&selector[1..]), decl);
        return;
    }
    if selector[0] == b'#' {
        add_css_rule(CSS_SEL_ID, selector_name(&selector[1..]), decl);
        return;
    }
    let tag = selector_tag_part(selector);
    if !tag.is_empty() {
        add_css_rule(CSS_SEL_TAG, tag, decl);
    }
    if let Some(dot) = find_selector_marker(selector, b'.') {
        add_css_rule(CSS_SEL_CLASS, selector_name(&selector[dot + 1..]), decl);
    }
    if let Some(hash) = find_selector_marker(selector, b'#') {
        add_css_rule(CSS_SEL_ID, selector_name(&selector[hash + 1..]), decl);
    }
}

fn add_css_rule(kind: u8, name: &[u8], decl: CssDecl) {
    if name.is_empty() {
        return;
    }
    unsafe {
        if CSS_RULE_COUNT >= MAX_CSS_RULES {
            return;
        }
        let idx = CSS_RULE_COUNT;
        CSS_RULE_COUNT += 1;
        CSS_RULES[idx] = CssRule::EMPTY;
        CSS_RULES[idx].selector_kind = kind;
        CSS_RULES[idx].name_len = name.len().min(MAX_CSS_NAME);
        for i in 0..CSS_RULES[idx].name_len {
            CSS_RULES[idx].name[i] = ascii_lower(name[i]);
        }
        CSS_RULES[idx].fg = decl.fg;
        CSS_RULES[idx].bg = decl.bg;
        CSS_RULES[idx].kind = decl.kind;
        CSS_RULES[idx].has_kind = decl.has_kind;
        CSS_RULES[idx].hidden = decl.hidden;
        CSS_RULES[idx].align = decl.align;
        CSS_RULES[idx].has_align = decl.has_align;
        CSS_RULES[idx].pad_before = decl.pad_before;
        CSS_RULES[idx].pad_after = decl.pad_after;
        CSS_RULES[idx].image_url_len = decl.image_url_len.min(MAX_URL);
        CSS_RULES[idx].image_url[..CSS_RULES[idx].image_url_len]
            .copy_from_slice(&decl.image_url[..CSS_RULES[idx].image_url_len]);
    }
}

fn reset_css() {
    unsafe {
        CSS_STYLES = CssStyles::default();
        CSS_RULE_COUNT = 0;
        CSS_VAR_COUNT = 0;
        CSS_WEB_FONT_DECLARED = false;
        CSS_WEB_FONT_READY = false;
        CSS_WEB_FONT_NAME_LEN = 0;
        CSS_WEB_FONT_URL_LEN = 0;
        CSS_WEB_FONT_BASE_LEN = 0;
        CSS_WEB_FONT_NAME = [0; MAX_FONT_NAME];
        CSS_WEB_FONT_URL = [0; MAX_URL];
        CSS_WEB_FONT_BASE = [0; MAX_URL];
        for i in 0..MAX_CSS_RULES {
            CSS_RULES[i] = CssRule::EMPTY;
        }
        for i in 0..MAX_CSS_VARS {
            CSS_VARS[i] = CssVar::EMPTY;
        }
    }
}

fn skip_css_comment(sheet: &[u8], mut i: usize) -> usize {
    while i + 1 < sheet.len() {
        if sheet[i] == b'*' && sheet[i + 1] == b'/' {
            return i + 2;
        }
        i += 1;
    }
    sheet.len()
}

fn contains_word_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    let mut i = 0usize;
    while i + needle.len() <= haystack.len() {
        let left_ok = i == 0 || !is_name_byte(haystack[i - 1]);
        let right = i + needle.len();
        let right_ok = right >= haystack.len() || !is_name_byte(haystack[right]);
        if left_ok && right_ok && slice_eq_ci(&haystack[i..right], needle) {
            return true;
        }
        i += 1;
    }
    false
}

fn contains_subslice_ci(haystack: &[u8], needle: &[u8]) -> bool {
    find_subslice_ci(haystack, needle, 0).is_some()
}

fn css_px_value(value: &[u8]) -> u32 {
    let mut i = 0usize;
    while i < value.len() && (value[i] < b'0' || value[i] > b'9') {
        i += 1;
    }
    let mut out = 0u32;
    while i < value.len() && value[i] >= b'0' && value[i] <= b'9' {
        out = out.saturating_mul(10).saturating_add((value[i] - b'0') as u32);
        i += 1;
    }
    out
}

fn css_pad_lines(value: &[u8]) -> u8 {
    if find_subslice_ci(value, b"px", 0).is_none() {
        return 0;
    }
    let px = css_px_value(value);
    if px < 16 {
        return 0;
    }
    ((px / LINE_HEIGHT as u32).min(MAX_PAD_LINES as u32)) as u8
}

fn is_heading_selector(sel: &[u8]) -> bool {
    slice_eq_ci(sel, b"h1")
        || slice_eq_ci(sel, b"h2")
        || slice_eq_ci(sel, b"h3")
        || slice_eq_ci(sel, b"h4")
        || slice_eq_ci(sel, b"h5")
        || slice_eq_ci(sel, b"h6")
}

fn selector_tail(selector: &[u8]) -> &[u8] {
    let mut start = 0usize;
    for i in 0..selector.len() {
        let b = selector[i];
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' || b == b'>' || b == b'+' || b == b'~'
        {
            start = i + 1;
        }
    }
    let mut tail = trim_ascii(&selector[start..]);
    let mut end = 0usize;
    while end < tail.len() && tail[end] != b':' && tail[end] != b'[' {
        end += 1;
    }
    tail = trim_ascii(&tail[..end]);
    tail
}

fn selector_tag_part(selector: &[u8]) -> &[u8] {
    let mut end = 0usize;
    while end < selector.len() {
        let b = selector[end];
        if !is_name_byte(b) || b == b'.' || b == b'#' {
            break;
        }
        end += 1;
    }
    &selector[..end]
}

fn selector_name(selector: &[u8]) -> &[u8] {
    let mut end = 0usize;
    while end < selector.len() && is_name_byte(selector[end]) {
        end += 1;
    }
    &selector[..end]
}

fn find_selector_marker(selector: &[u8], marker: u8) -> Option<usize> {
    let mut i = 0usize;
    while i < selector.len() {
        if selector[i] == marker {
            return Some(i);
        }
        if selector[i] == b':' || selector[i] == b'[' {
            return None;
        }
        i += 1;
    }
    None
}

fn trim_ascii(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && (s[start] == b' ' || s[start] == b'\t' || s[start] == b'\n' || s[start] == b'\r') {
        start += 1;
    }
    while end > start && (s[end - 1] == b' ' || s[end - 1] == b'\t' || s[end - 1] == b'\n' || s[end - 1] == b'\r')
    {
        end -= 1;
    }
    &s[start..end]
}

fn is_css_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' || b == 0x0C
}

fn find_subslice_ci(buf: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || buf.len() < needle.len() || start >= buf.len() {
        return None;
    }
    let mut i = start;
    while i + needle.len() <= buf.len() {
        if starts_with_ci(&buf[i..], needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_css_in_html(html: &[u8]) {
    let mut i = 0;
    while i < html.len() {
        if html[i] == b'<' {
            if let Some(tag_end) = find_byte(html, i + 1, b'>') {
                let tag = &html[i + 1..tag_end];
                let parsed = parse_tag(tag);
                if tag_eq(&parsed, b"style") {
                    let content_start = tag_end + 1;
                    if let Some(close) = find_subslice_ci(html, b"</style>", content_start) {
                        let css = &html[content_start..close];
                        parse_css_rules(css, None);
                        i = close + 8;
                        continue;
                    }
                }
                i = tag_end + 1;
                continue;
            }
        }
        i += 1;
    }
}

#[derive(Copy, Clone)]
struct RenderStyle {
    fg: u32,
    bg: u32,
    kind: u8,
    hidden: bool,
    align: u8,
    link_idx: u8,
    pad_before: u8,
    pad_after: u8,
    image_url: [u8; MAX_URL],
    image_url_len: usize,
}

pub struct Browser {
    visible: bool,
    win_x: usize,
    win_y: usize,
    win_w: usize,
    win_h: usize,
    url: [u8; MAX_URL],
    url_len: usize,
    lines: [[u8; MAX_COLS]; MAX_LINES],
    lens: [usize; MAX_LINES],
    fgs: [u32; MAX_LINES],
    bgs: [u32; MAX_LINES],
    kinds: [u8; MAX_LINES],
    aligns: [u8; MAX_LINES],
    line_links: [u8; MAX_LINES],
    line_image_urls: [[u8; MAX_URL]; MAX_LINES],
    line_image_lens: [usize; MAX_LINES],
    link_urls: [[u8; MAX_URL]; MAX_LINKS],
    link_lens: [usize; MAX_LINKS],
    link_count: usize,
    count: usize,
    scroll: usize,
    pending_load: bool,
    // Додатковий стан для банерів
    banner_active: bool,
    banner_text: [u8; MAX_COLS],
    banner_len: usize,
    banner_bg: u32,
    banner_fg: u32,
}

impl Browser {
    pub fn new(_fb: Framebuffer) -> Self {
        let mut url = [0u8; MAX_URL];
        let default = b"http://example.com";
        let len = default.len().min(MAX_URL);
        url[..len].copy_from_slice(&default[..len]);
        Self {
            visible: false,
            win_x: 0,
            win_y: 0,
            win_w: 0,
            win_h: 0,
            url,
            url_len: len,
            lines: [[0u8; MAX_COLS]; MAX_LINES],
            lens: [0; MAX_LINES],
            fgs: [STYLE_NONE; MAX_LINES],
            bgs: [STYLE_NONE; MAX_LINES],
            kinds: [KIND_NORMAL; MAX_LINES],
            aligns: [ALIGN_LEFT; MAX_LINES],
            line_links: [NO_LINK; MAX_LINES],
            line_image_urls: [[0; MAX_URL]; MAX_LINES],
            line_image_lens: [0; MAX_LINES],
            link_urls: [[0; MAX_URL]; MAX_LINKS],
            link_lens: [0; MAX_LINKS],
            link_count: 0,
            count: 0,
            scroll: 0,
            pending_load: false,
            banner_active: false,
            banner_text: [0; MAX_COLS],
            banner_len: 0,
            banner_bg: 0x002D4A7A,
            banner_fg: 0x00FFFFFF,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn show(&mut self, fb: &Framebuffer) {
        self.visible = true;
        if self.win_w == 0 || self.win_h == 0 {
            let (x, y, w, h) = calc_rect(fb);
            self.win_x = x;
            self.win_y = y;
            self.win_w = w;
            self.win_h = h;
        }
        if self.count == 0 {
            self.queue_navigation();
        }
        self.redraw(fb);
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn handle_click(&mut self, fb: &Framebuffer, x: usize, y: usize) -> bool {
        if !self.visible {
            return false;
        }

        let (wx, wy, ww, wh) = self.rect(fb);
        let body_y = wy + window::HEADER_H + 2;

        let url_y = body_y + PAD;
        let go_w = 46;
        let go_h = 22;
        let go_x = wx + ww.saturating_sub(PAD + go_w);

        if hit(x, y, go_x, url_y, go_w, go_h) {
            self.queue_navigation();
            self.redraw(fb);
            return true;
        }

        let scroll_x = wx + ww.saturating_sub(PAD + SCROLL_W);
        let scroll_y = url_y + go_h + PAD;
        let text_h = wh.saturating_sub(window::HEADER_H + 52);
        let max_lines = (text_h / LINE_HEIGHT).max(1);
        let max_scroll = self.count.saturating_sub(max_lines);

        let up_rect = (scroll_x, scroll_y, SCROLL_W, BTN_H);
        let down_rect = (scroll_x, scroll_y + text_h.saturating_sub(BTN_H), SCROLL_W, BTN_H);

        if hit(x, y, up_rect.0, up_rect.1, up_rect.2, up_rect.3) {
            if self.scroll > 0 {
                self.scroll -= 1;
                self.redraw(fb);
            }
            return true;
        }
        if hit(x, y, down_rect.0, down_rect.1, down_rect.2, down_rect.3) {
            if self.scroll < max_scroll {
                self.scroll += 1;
                self.redraw(fb);
            }
            return true;
        }

        let track_h = text_h.saturating_sub(BTN_H * 2);
        if track_h > 0
            && x >= scroll_x
            && x < scroll_x + SCROLL_W
            && y >= scroll_y + BTN_H
            && y < scroll_y + BTN_H + track_h
        {
            let thumb_h = ((max_lines as f32 / self.count as f32) * track_h as f32) as usize;
            let thumb_h = thumb_h.max(16).min(track_h);
            let click_pos = y.saturating_sub(scroll_y + BTN_H);
            let track_usable = track_h.saturating_sub(thumb_h);
            if track_usable > 0 {
                let new_scroll = (click_pos as f32 / track_usable as f32 * max_scroll as f32) as usize;
                self.scroll = new_scroll.min(max_scroll);
                self.redraw(fb);
            }
            return true;
        }

        let text_x = wx + PAD;
        let text_y = scroll_y;
        let line_w = scroll_x.saturating_sub(text_x + 8);
        if x >= text_x && x < text_x + line_w && y >= text_y && y < text_y + text_h {
            let row = (y - text_y) / LINE_HEIGHT;
            let line_idx = self.scroll + row;
            if line_idx < self.count {
                let link_idx = self.line_links[line_idx];
                if link_idx != NO_LINK && (link_idx as usize) < self.link_count {
                    let link_len = self.link_lens[link_idx as usize].min(MAX_URL);
                    self.url.fill(0);
                    self.url[..link_len].copy_from_slice(&self.link_urls[link_idx as usize][..link_len]);
                    self.url_len = link_len;
                    self.queue_navigation();
                    self.redraw(fb);
                    return true;
                }
            }
        }

        false
    }

    pub fn handle_char(&mut self, ch: u8) {
        if !self.visible {
            return;
        }
        match ch {
            b'\n' => {
                self.queue_navigation();
            }
            0x08 => {
                if self.url_len > 0 {
                    self.url_len -= 1;
                }
            }
            _ if (32..=126).contains(&ch) => {
                if self.url_len < MAX_URL {
                    self.url[self.url_len] = ch;
                    self.url_len += 1;
                }
            }
            _ => {}
        }
    }

    pub fn scroll_up(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }
        if self.scroll > 0 {
            self.scroll -= 1;
            self.redraw(fb);
        }
    }

    pub fn scroll_down(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }
        let (_, _, _, h) = self.rect(fb);
        let text_h = h.saturating_sub(window::HEADER_H + 52);
        let max_lines = (text_h / LINE_HEIGHT).max(1);
        let max_scroll = self.count.saturating_sub(max_lines);
        if self.scroll < max_scroll {
            self.scroll += 1;
            self.redraw(fb);
        }
    }

    pub fn service_pending(&mut self, fb: &Framebuffer) -> bool {
        if !self.pending_load {
            return false;
        }
        self.pending_load = false;
        warm_network();
        self.navigate_current();
        self.redraw(fb);
        true
    }

    fn queue_navigation(&mut self) {
        self.pending_load = true;
        self.clear_lines();
        self.push_line(b"=== HouseOS Browser ===");
        self.push_line(b"");
        self.push_line(b"[*] Loading queued...");
        self.push_line(b"    Mouse stays responsive before network work starts.");
    }

    // ============================================================
    // ОСНОВНИЙ МЕТОД ВІДМАЛЮВАННЯ — ТУТ ПОКРАЩЕНО ДИЗАЙН
    // ============================================================
    pub fn redraw(&mut self, fb: &Framebuffer) {
        if !self.visible {
            return;
        }

        let (x, y, w, h) = self.rect(fb);
        let ui = system::ui_settings();
        let is_dark = ui.dark;

        // 1. Вікно
        let chrome = window::draw_window(fb, x, y, w, h, b"Browser");
        fill_vertical_gradient(
            fb,
            chrome.content_x,
            chrome.content_y,
            chrome.content_w,
            chrome.content_h,
            if is_dark { 0x001D1D1D } else { 0x00FFFFFF },
            if is_dark { 0x00181818 } else { 0x00F6FAFF },
        );

        let mut writer = crate::TextWriter::new(*fb);
        let text_color = if is_dark { 0x00F2F5F8 } else { 0x00131A28 };
        let detail = if is_dark { 0x00B7C0CC } else { 0x004D5D72 };

        // 2. Рядок URL
        let url_y = chrome.content_y + PAD;
        let go_w = 46;
        let go_h = 22;
        let go_x = chrome.content_x + chrome.content_w.saturating_sub(PAD + go_w);
        let url_x = chrome.content_x + PAD;
        let url_w = go_x.saturating_sub(url_x + 8);

        display::fill_rect(
            fb,
            url_x,
            url_y,
            url_w,
            go_h,
            if is_dark { 0x002C333D } else { 0x00EAF1FA },
        );
        display::fill_rect(
            fb,
            url_x,
            url_y,
            url_w,
            1,
            if is_dark { 0x00424D5D } else { 0x00B8CCE6 },
        );
        display::fill_rect(
            fb,
            url_x,
            url_y + go_h - 1,
            url_w,
            1,
            if is_dark { 0x00424D5D } else { 0x00B8CCE6 },
        );

        fill_vertical_gradient(
            fb,
            go_x,
            url_y,
            go_w,
            go_h,
            if is_dark { 0x00494848 } else { 0x00EAF0F8 },
            if is_dark { 0x003F3F3F } else { 0x00D8E2EE },
        );
        writer.set_color(text_color);
        writer.set_pos(go_x + 11, url_y + 6);
        writer.write_bytes(b"Go");

        writer.set_color(text_color);
        writer.set_pos(url_x + 6, url_y + 6);
        let max_url_display = url_w.saturating_sub(12) / 8;
        let len = self.url_len.min(max_url_display);
        writer.write_bytes(&self.url[..len]);

        // 3. Область контенту
        let text_x = chrome.content_x + PAD;
        let text_y = url_y + go_h + PAD;
        let text_h = chrome.content_h.saturating_sub((text_y - chrome.content_y) + PAD);
        let scroll_x = chrome.content_x + chrome.content_w.saturating_sub(PAD + SCROLL_W);
        let line_w = scroll_x.saturating_sub(text_x + 8);

        // 3a. Фон сторінки (з CSS)
        if let Some(page_bg) = unsafe { CSS_STYLES.body_bg } {
            display::fill_rect(
                fb,
                text_x.saturating_sub(4),
                text_y.saturating_sub(4),
                line_w + 8,
                text_h + 4,
                page_bg,
            );
        }

        let max_lines = (text_h / LINE_HEIGHT).max(1);
        let max_scroll = self.count.saturating_sub(max_lines);

        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }

        let start = self.scroll;
        let end = (start + max_lines).min(self.count);

        // 4. Відмальовування рядків
        for (row, i) in (start..end).enumerate() {
            let y_pos = text_y + row * LINE_HEIGHT;
            let mut fg = if self.fgs[i] == STYLE_NONE {
                text_color
            } else {
                self.fgs[i]
            };
            let bg = self.bgs[i];
            let effective_bg = if bg != STYLE_NONE {
                bg
            } else if let Some(page_bg) = unsafe { CSS_STYLES.body_bg } {
                page_bg
            } else if is_dark {
                0x001B1B1B
            } else {
                0x00FFFFFF
            };
            if low_contrast(fg, effective_bg) {
                fg = readable_text_for(effective_bg);
            }
            let text_px_w = self.lens[i].saturating_mul(8).min(line_w);

            let draw_x = match self.aligns[i] {
                ALIGN_CENTER => text_x + line_w.saturating_sub(text_px_w) / 2,
                ALIGN_RIGHT => text_x + line_w.saturating_sub(text_px_w),
                _ => text_x,
            };

            // --- ФОН РЯДКА (якщо заданий) ---
            if bg != STYLE_NONE {
                display::fill_rect(
                    fb,
                    text_x.saturating_sub(4),
                    y_pos.saturating_sub(2),
                    line_w + 8,
                    LINE_HEIGHT + 2,
                    bg,
                );
            }

            // --- ОБРОБКА ЗА ТИПОМ (KIND) ---
            match self.kinds[i] {
                // ========== ЗАГОЛОВКИ ==========
                KIND_HEADING_1 | KIND_HEADING_2 | KIND_HEADING_3 | KIND_HEADING_4 => {
                    let is_h1 = self.kinds[i] == KIND_HEADING_1;
                    let is_h2 = self.kinds[i] == KIND_HEADING_2;

                    // Висота заголовка: h1 — 32px, h2 — 24px, h3 — 20px
                    let heading_h = LINE_HEIGHT;
                    let line_h = 8;

                    // Зміщення по Y для центрування тексту у висоті заголовка
                    let y_off = (heading_h - line_h) / 2;

                    // Повноширинний фон для заголовка
                    let bg_color = if bg != STYLE_NONE { bg } else {
                        if is_h1 {
                            0x0042BCEB
                        } else if is_h2 {
                            0x003B5A8A
                        } else {
                            0x004A6A9A
                        }
                    };
                    let fg_color = if fg != STYLE_NONE {
                        fg
                    } else {
                        if is_h1 {
                            0x00FFFFFF
                        } else if is_h2 {
                            0x00F0F4FF
                        } else {
                            0x00E0E8F8
                        }
                    };

                    // Малюємо банер для h1 або фон для інших
                    display::fill_rect(
                        fb,
                        text_x.saturating_sub(4),
                        y_pos.saturating_sub(2),
                        line_w + 8,
                        heading_h,
                        bg_color,
                    );

                    // Декоративна лінія знизу
                    if is_h2 {
                        display::fill_rect(
                            fb,
                            draw_x,
                            y_pos + heading_h - 1,
                            text_px_w,
                            1,
                            fg_color,
                        );
                    }

                    // Текст заголовка (збільшений)
                    writer.set_color(fg_color);
                    writer.set_pos(draw_x, y_pos + y_off);
                    writer.write_bytes(&self.lines[i][..self.lens[i]]);

                    // Для h1 — додаємо тінь (світлу лінію зверху)
                    if is_h1 {
                        display::fill_rect(
                            fb,
                            text_x.saturating_sub(4),
                            y_pos.saturating_sub(2),
                            line_w + 8,
                            1,
                            0x006A8AB0,
                        );
                    }
                }

                // ========== ПОСИЛАННЯ ==========
                KIND_LINK => {
                    writer.set_color(fg);
                    writer.set_pos(draw_x + 2, y_pos);
                    writer.write_bytes(&self.lines[i][..self.lens[i]]);
                    // Підкреслення
                    display::fill_rect(
                        fb,
                        draw_x + 2,
                        y_pos + 10,
                        text_px_w,
                        1,
                        fg,
                    );
                }

                // ========== КНОПКИ ==========
                KIND_BUTTON => {
                    let bw = (self.lens[i].saturating_mul(8) + 18).min(line_w);
                    let btn_bg = bg_if_none(
                        bg,
                        if is_dark { 0x00375C76 } else { 0x00CDEAFF },
                    );
                    display::fill_rect(
                        fb,
                        draw_x,
                        y_pos.saturating_sub(2),
                        bw,
                        LINE_HEIGHT,
                        btn_bg,
                    );
                    display::fill_rect(
                        fb,
                        draw_x,
                        y_pos.saturating_sub(2),
                        bw,
                        1,
                        if is_dark { 0x006C93AC } else { 0x0088B8D6 },
                    );
                    display::fill_rect(
                        fb,
                        draw_x,
                        y_pos + LINE_HEIGHT - 2,
                        bw,
                        1,
                        if is_dark { 0x004C6A7C } else { 0x006090B0 },
                    );
                    writer.set_color(fg);
                    writer.set_pos(draw_x + 8, y_pos);
                    writer.write_bytes(&self.lines[i][..self.lens[i]]);
                }

                // ========== ЗОБРАЖЕННЯ ==========
                KIND_IMAGE => {
                    let iw = line_w.min(260);
                    let img_bg = if is_dark { 0x002A313A } else { 0x00E9EEF5 };
                    display::fill_rect(
                        fb,
                        draw_x,
                        y_pos.saturating_sub(2),
                        iw,
                        LINE_HEIGHT,
                        img_bg,
                    );
                    display::fill_rect(
                        fb,
                        draw_x,
                        y_pos.saturating_sub(2),
                        iw,
                        1,
                        if is_dark { 0x00536170 } else { 0x00B8C6D6 },
                    );
                    let image_len = self.line_image_lens[i];
                    let mut drew_image = false;
                    if image_len > 0 {
                        let mut image_url = [0u8; MAX_URL];
                        image_url[..image_len].copy_from_slice(&self.line_image_urls[i][..image_len]);
                        drew_image = self.draw_remote_image(
                            fb,
                            &image_url[..image_len],
                            draw_x,
                            y_pos.saturating_sub(2),
                            line_w.min(360),
                            IMAGE_BLOCK_H.saturating_sub(4),
                        );
                    }
                    if !drew_image {
                        writer.set_color(fg);
                        writer.set_pos(draw_x + 8, y_pos);
                        writer.write_bytes(&self.lines[i][..self.lens[i]]);
                    }
                }

                KIND_IMAGE_SPACER => {}

                // ========== БЛОКИ ==========
                KIND_BLOCK => {
                    let block_bg = bg_if_none(bg, 0x00F4F8FC);
                    display::fill_rect(
                        fb,
                        text_x.saturating_sub(4),
                        y_pos.saturating_sub(2),
                        line_w + 8,
                        LINE_HEIGHT + 2,
                        block_bg,
                    );
                    display::fill_rect(
                        fb,
                        text_x.saturating_sub(4),
                        y_pos.saturating_sub(2),
                        3,
                        LINE_HEIGHT + 2,
                        if is_dark { 0x00545D68 } else { 0x00B6C4D4 },
                    );
                    writer.set_color(fg);
                    writer.set_pos(draw_x, y_pos);
                    writer.write_bytes(&self.lines[i][..self.lens[i]]);
                }

                // ========== ЗВИЧАЙНИЙ ТЕКСТ ==========
                _ => {
                    writer.set_color(fg);
                    writer.set_pos(draw_x, y_pos);
                    writer.write_bytes(&self.lines[i][..self.lens[i]]);
                }
            }
        }

        // 5. Смуга прокрутки
        let scroll_y = text_y;
        let scroll_h = text_h;
        display::fill_rect(
            fb,
            scroll_x,
            scroll_y,
            SCROLL_W,
            scroll_h,
            if is_dark { 0x00323232 } else { 0x00E1EAF5 },
        );
        fill_vertical_gradient(
            fb,
            scroll_x,
            scroll_y,
            SCROLL_W,
            BTN_H,
            if is_dark { 0x00484848 } else { 0x00D8E2EE },
            if is_dark { 0x003E3E3E } else { 0x00CBD8E8 },
        );
        fill_vertical_gradient(
            fb,
            scroll_x,
            scroll_y + scroll_h - BTN_H,
            SCROLL_W,
            BTN_H,
            if is_dark { 0x00484848 } else { 0x00D8E2EE },
            if is_dark { 0x003E3E3E } else { 0x00CBD8E8 },
        );
        writer.set_color(detail);
        writer.set_pos(scroll_x + 4, scroll_y + 3);
        writer.write_bytes(b"^");
        writer.set_pos(scroll_x + 4, scroll_y + scroll_h - BTN_H + 3);
        writer.write_bytes(b"v");

        if max_scroll > 0 && self.count > 0 {
            let track_h = scroll_h.saturating_sub(BTN_H * 2);
            let thumb_h = ((max_lines as f32 / self.count as f32) * track_h as f32) as usize;
            let thumb_h = thumb_h.max(16).min(track_h);
            let thumb_y = scroll_y + BTN_H
                + ((self.scroll as f32 / max_scroll as f32) * (track_h.saturating_sub(thumb_h)) as f32)
                    as usize;
            display::fill_rect(
                fb,
                scroll_x,
                thumb_y,
                SCROLL_W,
                thumb_h,
                if is_dark { 0x00777F8A } else { 0x0093A9C0 },
            );
        }
    }

    pub fn rect(&self, fb: &Framebuffer) -> (usize, usize, usize, usize) {
        if self.win_w == 0 || self.win_h == 0 {
            calc_rect(fb)
        } else {
            (self.win_x, self.win_y, self.win_w, self.win_h)
        }
    }

    pub fn set_pos(&mut self, x: usize, y: usize) {
        self.win_x = x;
        self.win_y = y;
    }

    pub fn set_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        self.win_x = x;
        self.win_y = y;
        self.win_w = w;
        self.win_h = h;
    }

    // ============================================================
    // НАВІГАЦІЯ ТА ЗАВАНТАЖЕННЯ
    // ============================================================

    fn navigate_current(&mut self) {
        self.clear_lines();
        self.banner_active = false;

        let devices = net::devices();
        if devices.is_empty() {
            self.push_line(b"[!] No network adapter detected.");
            self.push_line(b"    Check QEMU NIC or PCI passthrough.");
            return;
        }

        let mut request_url = [0u8; MAX_URL];
        let request_len = prepare_request_url(&self.url[..self.url_len], &mut request_url);
        let url = &request_url[..request_len];

        if is_plain_search_query(&self.url[..self.url_len]) {
            let mut query = [0u8; MAX_URL];
            let query_len = self.url_len.min(MAX_URL);
            query[..query_len].copy_from_slice(&self.url[..query_len]);
            self.push_search_suggestions(&query[..query_len]);
        }

        if starts_with_ci(&self.url[..self.url_len], b"https://") {
            if request_len > 0 && starts_with_ci(url, b"http://") {
                self.load_http_page(url);
                if self.count == 0 {
                    self.push_line(b"[!] HTTPS proxy returned an empty page.");
                }
                return;
            }
            self.push_line(b"[!] HTTPS needs TLS decryptor.");
            return;
        }

        if request_len > 0 && (starts_with_ci(url, b"http://") || !contains_byte(url, b':')) {
            self.load_http_page(url);
        } else {
            self.push_line(b"[!] Unknown URL scheme.");
        }
    }

    fn load_http_page(&mut self, url: &[u8]) {
        warm_network();
        let mut fetch = unsafe { netstack::http_get(url, &mut FETCH_BUF) };
        let mut attempts = 1usize;
        while fetch.code != FetchCode::Ok && fetch.code != FetchCode::BufferFull && attempts < HTTP_RETRY_ATTEMPTS {
            warm_network();
            fetch = unsafe { netstack::http_get(url, &mut FETCH_BUF) };
            attempts += 1;
        }

        if fetch.code != FetchCode::Ok && fetch.code != FetchCode::BufferFull {
            let mut line = [0u8; MAX_COLS];
            let mut p = 0;
            p += write_str(&mut line[p..], b"HTTP failed after retries: ");
            p += write_str(&mut line[p..], fetch_code_name(fetch.code));
            self.push_bytes(&line[..p]);
            if fetch.code == FetchCode::TcpFailed && is_https_proxy_url(url) {
                self.push_line(b"[!] HTTPS proxy is not reachable.");
                self.push_line(b"    Run HouseOS with run.ps1 so host port 18080 starts.");
                self.push_line(b"    Check build\\https-proxy.log if it still fails.");
                return;
            }
            self.push_line(b"[!] Failed to load page.");
            self.push_line(b"    Try: http://example.com");
            return;
        }

        reset_css();
        self.banner_active = false;

        let data =
            unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(FETCH_BUF) as *const u8, fetch.bytes) };
        self.render_html(data, fetch.body_offset);

        if self.count >= MAX_LINES {
            self.push_line(b"[...] Page truncated");
        }
    }

    fn push_search_suggestions(&mut self, query: &[u8]) {
        let clean = trim_ascii(query);
        if clean.is_empty() {
            return;
        }
        self.push_line(b"");
        self.push_styled_aligned(
            b"Search suggestions",
            0x00101B2D,
            0x00DCEBFF,
            KIND_HEADING_2,
            ALIGN_CENTER,
        );
        let mut query_line = [0u8; MAX_COLS];
        let mut qp = 0usize;
        qp += write_str(&mut query_line[qp..], b"Query: ");
        qp += write_str(&mut query_line[qp..], clean);
        self.push_styled_aligned(
            &query_line[..qp],
            0x004D5D72,
            STYLE_NONE,
            KIND_MUTED,
            ALIGN_CENTER,
        );
        self.push_search_link(
            b"YouTube",
            b"https://www.youtube.com/results?search_query=",
            clean,
        );
        self.push_search_link(
            b"Wikipedia",
            b"https://en.wikipedia.org/wiki/Special:Search?search=",
            clean,
        );
        self.push_search_link(
            b"DuckDuckGo",
            b"https://duckduckgo.com/?q=",
            clean,
        );
        self.push_search_link(
            b"Google",
            b"https://www.google.com/search?q=",
            clean,
        );
        self.push_line(b"");
    }

    fn push_search_link(&mut self, label: &[u8], prefix: &[u8], query: &[u8]) {
        let mut line = [0u8; MAX_COLS];
        let mut url = [0u8; MAX_URL];
        let mut up = 0usize;
        up += write_str(&mut url[up..], prefix);
        up += write_query_encoded(&mut url[up..], query);
        let link_idx = self.add_link(&url[..up]);

        let mut p = 0usize;
        p += write_str(&mut line[p..], label);
        p += write_str(&mut line[p..], b" - ");
        p += write_str(&mut line[p..], query);
        self.push_styled_link(&line[..p], 0x001A5FB4, STYLE_NONE, KIND_LINK, link_idx);
    }

    fn add_link(&mut self, url: &[u8]) -> u8 {
        let clean = trim_ascii(url);
        if clean.is_empty() || self.link_count >= MAX_LINKS {
            return NO_LINK;
        }
        let idx = self.link_count;
        let len = clean.len().min(MAX_URL);
        self.link_urls[idx].fill(0);
        self.link_urls[idx][..len].copy_from_slice(&clean[..len]);
        self.link_lens[idx] = len;
        self.link_count += 1;
        idx as u8
    }

    fn add_link_from_href(&mut self, href: &[u8]) -> u8 {
        let mut url = [0u8; MAX_URL];
        let len = self.build_href_url(href, &mut url);
        if len == 0 {
            return NO_LINK;
        }
        self.add_link(&url[..len])
    }

    fn build_href_url(&self, href: &[u8], url: &mut [u8; MAX_URL]) -> usize {
        let href = trim_ascii(href);
        if href.is_empty() || starts_with_ci(href, b"javascript:") || href[0] == b'#' {
            return 0;
        }
        let mut p = 0usize;
        if starts_with_ci(href, b"http://") || starts_with_ci(href, b"https://") {
            p += copy_limited(&mut url[p..], href);
        } else if href.len() > 2 && href[0] == b'/' && href[1] == b'/' {
            p += copy_limited(&mut url[p..], b"https:");
            p += copy_limited(&mut url[p..], href);
        } else if href[0] == b'/' {
            p += copy_current_origin(&self.url[..self.url_len], &mut url[p..]);
            p += copy_limited(&mut url[p..], href);
        } else {
            p += copy_current_base(&self.url[..self.url_len], &mut url[p..]);
            p += copy_limited(&mut url[p..], href);
        }
        p
    }

    fn render_html(&mut self, data: &[u8], body_offset: usize) {
        if data.is_empty() || body_offset >= data.len() {
            self.push_line(b"[!] Empty page body.");
            return;
        }

        let body = &data[body_offset..];
        parse_css_in_html(body);
        self.load_external_css(body);
        self.render_html_body(body);
        self.extract_javascript_content(body);
    }

    fn load_external_css(&mut self, html: &[u8]) {
        let mut i = 0usize;
        let mut loaded = 0usize;
        while i < html.len() && loaded < 8 {
            if html[i] == b'<' {
                if let Some(tag_end) = find_byte(html, i + 1, b'>') {
                    let tag = &html[i + 1..tag_end];
                    let parsed = parse_tag(tag);
                    if tag_eq(&parsed, b"link") && link_is_stylesheet(tag) {
                        if let Some((start, len)) = find_attr(tag, b"href") {
                            let mut css_url = [0u8; MAX_URL];
                            let url_len = self.build_href_url(&tag[start..start + len], &mut css_url);
                            if url_len > 0 {
                                let mut request_url = [0u8; MAX_URL];
                                let req_len = prepare_request_url(&css_url[..url_len], &mut request_url);
                                if req_len > 0 && starts_with_ci(&request_url[..req_len], b"http://") {
                                    warm_network();
                                    let mut fetch =
                                        unsafe { netstack::http_get(&request_url[..req_len], &mut CSS_FETCH_BUF) };
                                    let mut attempts = 1usize;
                                    while fetch.code != FetchCode::Ok
                                        && fetch.code != FetchCode::BufferFull
                                        && attempts < 4
                                    {
                                        warm_network();
                                        fetch = unsafe {
                                            netstack::http_get(&request_url[..req_len], &mut CSS_FETCH_BUF)
                                        };
                                        attempts += 1;
                                    }
                                    if fetch.code == FetchCode::Ok || fetch.code == FetchCode::BufferFull {
                                        let css = unsafe {
                                            core::slice::from_raw_parts(
                                                core::ptr::addr_of!(CSS_FETCH_BUF) as *const u8,
                                                fetch.bytes,
                                            )
                                        };
                                        if fetch.body_offset < css.len() {
                                            parse_css_rules(&css[fetch.body_offset..], Some(&css_url[..url_len]));
                                            loaded += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    i = tag_end + 1;
                    continue;
                }
            }
            i += 1;
        }
    }

    fn extract_javascript_content(&mut self, html: &[u8]) {
        let mut i = 0usize;
        let mut shown = 0usize;
        while i < html.len() && shown < 8 && self.count < MAX_LINES {
            if html[i] == b'<' {
                if let Some(tag_end) = find_byte(html, i + 1, b'>') {
                    let tag = &html[i + 1..tag_end];
                    let parsed = parse_tag(tag);
                    if tag_eq(&parsed, b"script") {
                        let content_start = tag_end + 1;
                        let content_end = skip_element(html, content_start, parsed.name, parsed.name_len);
                        let close_start = find_subslice_ci(html, b"</script>", content_start).unwrap_or(content_end);
                        let script = &html[content_start..close_start.min(html.len())];
                        shown += self.extract_javascript_strings(script, shown == 0, 8 - shown);
                        i = content_end;
                        continue;
                    }
                    i = tag_end + 1;
                    continue;
                }
            }
            i += 1;
        }
    }

    fn extract_javascript_strings(&mut self, script: &[u8], first: bool, limit: usize) -> usize {
        let mut shown = 0usize;
        let mut pos = 0usize;
        while pos < script.len() && shown < limit && self.count < MAX_LINES {
            let next = find_next_js_visible_op(script, pos);
            let op = match next {
                Some(v) => v,
                None => break,
            };
            let end = find_byte(script, op, b';').unwrap_or((op + 768).min(script.len()));
            let mut fragment = [0u8; 512];
            let mut html_like = false;
            let len = copy_js_quoted_text(&script[op..end], &mut fragment, &mut html_like);
            if len > 2 && useful_js_text(&fragment[..len]) {
                if first && shown == 0 {
                    self.push_styled(
                        b"JavaScript extracted content",
                        0x00616E7C,
                        STYLE_NONE,
                        KIND_MUTED,
                    );
                }
                if html_like {
                    self.render_html_body(&fragment[..len]);
                } else {
                    self.push_styled(&fragment[..len], STYLE_NONE, STYLE_NONE, KIND_NORMAL);
                }
                shown += 1;
            }
            pos = end.saturating_add(1);
        }
        shown
    }

    fn render_html_body(&mut self, html: &[u8]) {
        let mut style = RenderStyle {
            fg: unsafe { CSS_STYLES.body_fg.unwrap_or(0x00192533) },
            bg: unsafe { CSS_STYLES.body_bg.unwrap_or(STYLE_NONE) },
            kind: KIND_NORMAL,
            hidden: false,
            align: unsafe { CSS_STYLES.body_align.unwrap_or(ALIGN_LEFT) },
            link_idx: NO_LINK,
            pad_before: 0,
            pad_after: 0,
            image_url: [0; MAX_URL],
            image_url_len: 0,
        };
        let base_style = style;
        let mut style_stack = [base_style; MAX_STYLE_STACK];
        let mut style_depth = 0usize;

        let mut line = [0u8; MAX_COLS];
        let mut line_len = 0;
        let mut prev_space = true;
        let mut i = 0;

        while i < html.len() && self.count < MAX_LINES {
            let b = html[i];

            if b == b'<' {
                if let Some(tag_end) = find_byte(html, i + 1, b'>') {
                    let tag = &html[i + 1..tag_end];
                    let parsed = parse_tag(tag);

                    if parsed.name_len > 0 && !parsed.closing {
                        if tag_eq(&parsed, b"script") {
                            self.flush_line(&mut line, &mut line_len, &mut prev_space, style);
                            i = skip_element(html, tag_end + 1, parsed.name, parsed.name_len);
                            continue;
                        }
                        if tag_eq(&parsed, b"style") {
                            i = skip_element(html, tag_end + 1, parsed.name, parsed.name_len);
                            continue;
                        }
                        if tag_eq(&parsed, b"br") || tag_eq(&parsed, b"hr") {
                            self.flush_line(&mut line, &mut line_len, &mut prev_space, style);
                            if tag_eq(&parsed, b"hr") {
                                self.push_styled(
                                    b"--------------------------------------------------",
                                    0x007A8796,
                                    STYLE_NONE,
                                    KIND_MUTED,
                                );
                            }
                            i = tag_end + 1;
                            continue;
                        }
                        if tag_eq(&parsed, b"img") {
                            self.flush_line(&mut line, &mut line_len, &mut prev_space, style);
                            self.push_image_tag(tag);
                            i = tag_end + 1;
                            continue;
                        }
                    }

                    if parsed.closing {
                        self.flush_line(&mut line, &mut line_len, &mut prev_space, style);
                        if tag_supports_padding(&parsed) {
                            self.push_spacer_lines(style.bg, style.pad_after);
                        }
                        if style_depth > 0 {
                            style_depth -= 1;
                            style = style_stack[style_depth];
                        } else {
                            style = base_style;
                        }
                    } else {
                        self.flush_line(&mut line, &mut line_len, &mut prev_space, style);
                        let next_style = self.style_for_tag(tag, &parsed, style);
                        if !parsed.self_closing && !tag_is_void(&parsed) {
                            if style_depth < MAX_STYLE_STACK {
                                style_stack[style_depth] = style;
                                style_depth += 1;
                            }
                            if next_style.image_url_len > 0 && tag_supports_padding(&parsed) {
                                self.push_image_url(&next_style.image_url[..next_style.image_url_len]);
                            }
                            style = next_style;
                            if tag_supports_padding(&parsed) {
                                self.push_spacer_lines(style.bg, style.pad_before);
                            }
                        }
                    }

                    i = tag_end + 1;
                    continue;
                }
            }

            if b == b'&' {
                let (entity, next) = decode_entity(html, i);
                self.append_to_line(&mut line, &mut line_len, entity, &mut prev_space, style);
                i = next;
                continue;
            }

            if b == b'\r' || b == b'\n' || b == b'\t' || b == b' ' {
                self.append_to_line(&mut line, &mut line_len, b' ', &mut prev_space, style);
            } else if (32..=126).contains(&b) {
                self.append_to_line(&mut line, &mut line_len, b, &mut prev_space, style);
            } else if b >= 0x80 {
                let (ascii, ascii_len, next) = decode_utf8_to_ascii(html, i);
                for j in 0..ascii_len {
                    self.append_to_line(&mut line, &mut line_len, ascii[j], &mut prev_space, style);
                }
                i = next;
                continue;
            }

            i += 1;
        }

        self.flush_line(&mut line, &mut line_len, &mut prev_space, style);
    }

    fn push_image_tag(&mut self, tag: &[u8]) {
        if let Some((start, len)) = find_attr(tag, b"src") {
            let mut img_url = [0u8; MAX_URL];
            let img_len = self.build_href_url(&tag[start..start + len], &mut img_url);
            if img_len > 0 {
                self.push_image_url(&img_url[..img_len]);
                return;
            }
        }

        let mut text = [0u8; MAX_COLS];
        let mut p = 0usize;
        p += write_str(&mut text[p..], b"[image] ");
        if let Some((start, len)) = find_attr(tag, b"alt") {
            p += write_str(&mut text[p..], &tag[start..start + len]);
        } else if let Some((start, len)) = find_attr(tag, b"src") {
            p += write_str(&mut text[p..], &tag[start..start + len]);
        } else {
            p += write_str(&mut text[p..], b"embedded image");
        }
        self.push_styled(&text[..p], 0x00616E7C, STYLE_NONE, KIND_IMAGE);
    }

    fn push_image_url(&mut self, url: &[u8]) {
        let clean = trim_ascii(url);
        if clean.is_empty() {
            return;
        }
        let mut line = [0u8; MAX_COLS];
        let mut p = 0usize;
        p += write_str(&mut line[p..], b"[image] ");
        p += write_str(&mut line[p..], clean);
        let line_idx = self.push_styled_full(
            &line[..p],
            0x00616E7C,
            STYLE_NONE,
            KIND_IMAGE,
            ALIGN_LEFT,
            NO_LINK,
        );
        if line_idx < MAX_LINES {
            let len = clean.len().min(MAX_URL);
            self.line_image_urls[line_idx] = [0; MAX_URL];
            self.line_image_urls[line_idx][..len].copy_from_slice(&clean[..len]);
            self.line_image_lens[line_idx] = len;
        }
        for _ in 1..IMAGE_BLOCK_LINES {
            self.push_styled_full(b"", STYLE_NONE, STYLE_NONE, KIND_IMAGE_SPACER, ALIGN_LEFT, NO_LINK);
        }
    }

    fn draw_remote_image(
        &mut self,
        fb: &Framebuffer,
        url: &[u8],
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> bool {
        if w == 0 || h == 0 {
            return false;
        }
        let mut request_url = [0u8; MAX_URL];
        let req_len = prepare_image_request_url(url, &mut request_url);
        if req_len == 0 {
            return false;
        }
        warm_network();
        let mut fetch = unsafe { netstack::http_get(&request_url[..req_len], &mut IMAGE_FETCH_BUF) };
        let mut attempts = 1usize;
        while fetch.code != FetchCode::Ok && fetch.code != FetchCode::BufferFull && attempts < 4 {
            warm_network();
            fetch = unsafe { netstack::http_get(&request_url[..req_len], &mut IMAGE_FETCH_BUF) };
            attempts += 1;
        }
        if fetch.code != FetchCode::Ok && fetch.code != FetchCode::BufferFull {
            return false;
        }
        let bytes = fetch.bytes.min(IMAGE_FETCH_BUF_SIZE);
        if fetch.body_offset >= bytes {
            return false;
        }
        let data = unsafe {
            core::slice::from_raw_parts(
                core::ptr::addr_of!(IMAGE_FETCH_BUF) as *const u8,
                bytes,
            )
        };
        let body = &data[fetch.body_offset..bytes];
        if !image::decode_jpeg(body) {
            return false;
        }
        display::draw_bgra_image_fit_rect(
            fb,
            image::get_bgra_ptr(),
            image::get_bgra_len(),
            x,
            y,
            w,
            h,
        )
    }

    fn push_spacer_lines(&mut self, bg: u32, count: u8) {
        let lines = count.min(MAX_PAD_LINES);
        for _ in 0..lines {
            self.push_styled_full(b"", STYLE_NONE, bg, KIND_NORMAL, ALIGN_LEFT, NO_LINK);
        }
    }

    fn style_for_tag(&mut self, tag: &[u8], parsed: &ParsedTag, current: RenderStyle) -> RenderStyle {
        let mut style = current;
        style.pad_before = 0;
        style.pad_after = 0;
        style.image_url = [0; MAX_URL];
        style.image_url_len = 0;

        // Визначаємо тип заголовка за тегом
        let heading_kind = if tag_eq(parsed, b"h1") {
            KIND_HEADING_1
        } else if tag_eq(parsed, b"h2") {
            KIND_HEADING_2
        } else if tag_eq(parsed, b"h3") {
            KIND_HEADING_3
        } else if tag_eq(parsed, b"h4") || tag_eq(parsed, b"h5") || tag_eq(parsed, b"h6") {
            KIND_HEADING_4
        } else {
            KIND_NORMAL
        };

        if heading_kind != KIND_NORMAL {
            style.kind = heading_kind;
            style.align = ALIGN_CENTER;
            if let Some(fg) = unsafe { CSS_STYLES.heading_fg } {
                style.fg = fg;
            }
            if let Some(bg) = unsafe { CSS_STYLES.heading_bg } {
                style.bg = bg;
            }
            // Для h1 — додатковий відступ
            if heading_kind == KIND_HEADING_1 {
                style.pad_before = 2;
                style.pad_after = 1;
            } else if heading_kind == KIND_HEADING_2 {
                style.pad_before = 1;
                style.pad_after = 1;
            }
        } else if tag_eq(parsed, b"a") {
            style.kind = KIND_LINK;
            style.fg = unsafe { CSS_STYLES.link_fg.unwrap_or(0x001A5FB4) };
            style.bg = unsafe { CSS_STYLES.link_bg.unwrap_or(STYLE_NONE) };
            if let Some((start, len)) = find_attr(tag, b"href") {
                style.link_idx = self.add_link_from_href(&tag[start..start + len]);
            }
        } else if tag_eq(parsed, b"button") || tag_eq(parsed, b"input") {
            style.kind = KIND_BUTTON;
            style.fg = unsafe { CSS_STYLES.button_fg.unwrap_or(0x000F2433) };
            style.bg = unsafe { CSS_STYLES.button_bg.unwrap_or(0x00CDEAFF) };
        } else if tag_eq(parsed, b"code") || tag_eq(parsed, b"pre") {
            style.kind = KIND_BLOCK;
            style.fg = unsafe { CSS_STYLES.code_fg.unwrap_or(0x002D4A22) };
            style.bg = unsafe { CSS_STYLES.code_bg.unwrap_or(0x00F0F4F8) };
        } else if tag_eq(parsed, b"script")
            || tag_eq(parsed, b"style")
            || tag_eq(parsed, b"noscript")
            || tag_eq(parsed, b"template")
        {
            style.hidden = true;
        } else if tag_eq(parsed, b"div") || tag_eq(parsed, b"section") || tag_eq(parsed, b"p") {
            style.kind = KIND_NORMAL;
        } else if tag_eq(parsed, b"small") {
            style.kind = KIND_MUTED;
            style.fg = 0x00616E7C;
        } else if tag_eq(parsed, b"strong") || tag_eq(parsed, b"b") {
            style.kind = KIND_STRONG;
        } else if tag_eq(parsed, b"img") {
            style.kind = KIND_IMAGE;
            style.fg = 0x00616E7C;
        } else if tag_eq(parsed, b"body") {
            style.kind = KIND_NORMAL;
            if let Some(fg) = unsafe { CSS_STYLES.body_fg } {
                style.fg = fg;
            }
            if let Some(bg) = unsafe { CSS_STYLES.body_bg } {
                style.bg = bg;
            }
            if let Some(align) = unsafe { CSS_STYLES.body_align } {
                style.align = align;
            }
        }

        apply_css_rules_to_tag(tag, parsed, &mut style);

        // Inline стилі
        if let Some((start, len)) = find_attr(tag, b"style") {
            let style_attr = &tag[start..start + len];
            let mut inline_decl = parse_css_declarations(style_attr);
            if inline_decl.image_url_len > 0 {
                let mut resolved = [0u8; MAX_URL];
                let resolved_len = build_url_with_base(
                    &self.url[..self.url_len],
                    &inline_decl.image_url[..inline_decl.image_url_len],
                    &mut resolved,
                );
                if resolved_len > 0 {
                    inline_decl.image_url = [0; MAX_URL];
                    inline_decl.image_url_len = resolved_len.min(MAX_URL);
                    inline_decl.image_url[..inline_decl.image_url_len]
                        .copy_from_slice(&resolved[..inline_decl.image_url_len]);
                }
            }
            apply_decl_to_render_style(inline_decl, &mut style);
            if let Some(color) = parse_style_color(style_attr, b"color") {
                style.fg = color;
            }
            if let Some(bg) = parse_style_color(style_attr, b"background-color") {
                style.bg = bg;
            }
            if let Some(bg) = parse_style_color(style_attr, b"background") {
                style.bg = bg;
            }
            if style_property_has_word(style_attr, b"display", b"none")
                || style_property_has_word(style_attr, b"visibility", b"hidden")
            {
                style.hidden = true;
            }
            if style_property_has_word(style_attr, b"text-decoration", b"underline") {
                style.kind = KIND_LINK;
            }
            if style_property_has_word(style_attr, b"text-align", b"center") {
                style.align = ALIGN_CENTER;
            } else if style_property_has_word(style_attr, b"text-align", b"right") {
                style.align = ALIGN_RIGHT;
            } else if style_property_has_word(style_attr, b"text-align", b"left") {
                style.align = ALIGN_LEFT;
            }
            if let Some((start, len)) = find_style_property(style_attr, b"padding") {
                let pad = css_pad_lines(&style_attr[start..start + len]);
                style.pad_before = pad;
                style.pad_after = pad;
            }
            if let Some((start, len)) = find_style_property(style_attr, b"padding-top") {
                style.pad_before = css_pad_lines(&style_attr[start..start + len]);
            }
            if let Some((start, len)) = find_style_property(style_attr, b"padding-bottom") {
                style.pad_after = css_pad_lines(&style_attr[start..start + len]);
            }
            if let Some((start, len)) = find_style_property(style_attr, b"height") {
                let pad = (css_pad_lines(&style_attr[start..start + len]) / 2).min(MAX_PAD_LINES);
                style.pad_before = pad;
                style.pad_after = pad;
            }
            if let Some((start, len)) = find_style_property(style_attr, b"min-height") {
                let pad = (css_pad_lines(&style_attr[start..start + len]) / 2).min(MAX_PAD_LINES);
                style.pad_before = pad;
                style.pad_after = pad;
            }
        }

        style
    }

    fn append_to_line(
        &mut self,
        line: &mut [u8; MAX_COLS],
        line_len: &mut usize,
        b: u8,
        prev_space: &mut bool,
        style: RenderStyle,
    ) {
        if style.hidden {
            return;
        }
        if b == b' ' {
            if *prev_space {
                return;
            }
            *prev_space = true;
        } else {
            *prev_space = false;
        }

        if *line_len >= MAX_COLS {
            self.flush_line(line, line_len, prev_space, style);
        }

        if *line_len < MAX_COLS {
            line[*line_len] = b;
            *line_len += 1;
        }
    }

    fn flush_line(
        &mut self,
        line: &mut [u8; MAX_COLS],
        line_len: &mut usize,
        prev_space: &mut bool,
        style: RenderStyle,
    ) {
        if style.hidden {
            for i in 0..*line_len {
                line[i] = 0;
            }
            *line_len = 0;
            *prev_space = true;
            return;
        }

        // Якщо рядок порожній, але є фон — зберігаємо для банерів/блоків
        if *line_len == 0 {
            if style.bg != STYLE_NONE {
                self.push_styled_full(b"", style.fg, style.bg, style.kind, style.align, style.link_idx);
            }
            *prev_space = true;
            return;
        }

        // Визначаємо, чи це перший заголовок — для банера
        let is_first_heading = style.kind == KIND_HEADING_1
            || style.kind == KIND_HEADING_2
            || style.kind == KIND_HEADING_3;
        if is_first_heading && self.count == 0 && !self.banner_active {
            self.banner_active = true;
            let len = (*line_len).min(MAX_COLS);
            self.banner_text[..len].copy_from_slice(&line[..len]);
            self.banner_len = len;
            self.banner_bg = if style.bg != STYLE_NONE {
                style.bg
            } else {
                0x002D4A7A
            };
            self.banner_fg = if style.fg != STYLE_NONE {
                style.fg
            } else {
                0x00FFFFFF
            };
        }

        self.push_styled_full(
            &line[..*line_len],
            style.fg,
            style.bg,
            style.kind,
            style.align,
            style.link_idx,
        );

        for i in 0..*line_len {
            line[i] = 0;
        }
        *line_len = 0;
        *prev_space = true;
    }

    fn push_line(&mut self, bytes: &[u8]) {
        self.push_styled(bytes, STYLE_NONE, STYLE_NONE, KIND_NORMAL);
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        self.push_styled(bytes, STYLE_NONE, STYLE_NONE, KIND_NORMAL);
    }

    fn push_styled(&mut self, bytes: &[u8], fg: u32, bg: u32, kind: u8) {
        self.push_styled_aligned(bytes, fg, bg, kind, ALIGN_LEFT);
    }

    fn push_styled_link(&mut self, bytes: &[u8], fg: u32, bg: u32, kind: u8, link_idx: u8) {
        self.push_styled_full(bytes, fg, bg, kind, ALIGN_LEFT, link_idx);
    }

    fn push_styled_aligned(&mut self, bytes: &[u8], fg: u32, bg: u32, kind: u8, align: u8) {
        self.push_styled_full(bytes, fg, bg, kind, align, NO_LINK);
    }

    fn push_styled_full(
        &mut self,
        bytes: &[u8],
        fg: u32,
        bg: u32,
        kind: u8,
        align: u8,
        link_idx: u8,
    ) -> usize {
        if self.count < MAX_LINES {
            let idx = self.count;
            let len = bytes.len().min(MAX_COLS);
            self.lines[self.count][..len].copy_from_slice(&bytes[..len]);
            self.lens[self.count] = len;
            self.fgs[self.count] = fg;
            self.bgs[self.count] = bg;
            self.kinds[self.count] = kind;
            self.aligns[self.count] = align;
            self.line_links[self.count] = link_idx;
            self.line_image_lens[self.count] = 0;
            self.line_image_urls[self.count] = [0; MAX_URL];
            self.count += 1;
            idx
        } else {
            // Зсув вгору
            for i in 1..MAX_LINES {
                self.lines[i - 1] = self.lines[i];
                self.lens[i - 1] = self.lens[i];
                self.fgs[i - 1] = self.fgs[i];
                self.bgs[i - 1] = self.bgs[i];
                self.kinds[i - 1] = self.kinds[i];
                self.aligns[i - 1] = self.aligns[i];
                self.line_links[i - 1] = self.line_links[i];
                self.line_image_lens[i - 1] = self.line_image_lens[i];
                self.line_image_urls[i - 1] = self.line_image_urls[i];
            }
            let len = bytes.len().min(MAX_COLS);
            self.lines[MAX_LINES - 1][..len].copy_from_slice(&bytes[..len]);
            self.lens[MAX_LINES - 1] = len;
            self.fgs[MAX_LINES - 1] = fg;
            self.bgs[MAX_LINES - 1] = bg;
            self.kinds[MAX_LINES - 1] = kind;
            self.aligns[MAX_LINES - 1] = align;
            self.line_links[MAX_LINES - 1] = link_idx;
            self.line_image_lens[MAX_LINES - 1] = 0;
            self.line_image_urls[MAX_LINES - 1] = [0; MAX_URL];
            MAX_LINES - 1
        }
    }

    fn clear_lines(&mut self) {
        self.count = 0;
        self.scroll = 0;
        self.link_count = 0;
        self.banner_active = false;
        self.banner_len = 0;
        for i in 0..MAX_LINES {
            self.line_image_lens[i] = 0;
            self.line_image_urls[i] = [0; MAX_URL];
        }
    }
}

// ============================================================================
// Helper Functions (без змін)
// ============================================================================

#[derive(Copy, Clone)]
struct ParsedTag {
    name: [u8; 12],
    name_len: usize,
    closing: bool,
    self_closing: bool,
}

fn parse_tag(tag: &[u8]) -> ParsedTag {
    let mut parsed = ParsedTag {
        name: [0; 12],
        name_len: 0,
        closing: false,
        self_closing: false,
    };
    let mut i = 0usize;
    while i < tag.len() && tag[i] == b' ' {
        i += 1;
    }
    if i < tag.len() && tag[i] == b'/' {
        parsed.closing = true;
        i += 1;
    }
    while i < tag.len() && parsed.name_len < parsed.name.len() {
        let b = tag[i];
        if !is_name_byte(b) {
            break;
        }
        parsed.name[parsed.name_len] = ascii_lower(b);
        parsed.name_len += 1;
        i += 1;
    }
    let mut j = tag.len();
    while j > 0 && tag[j - 1] == b' ' {
        j -= 1;
    }
    parsed.self_closing = j > 0 && tag[j - 1] == b'/';
    parsed
}

fn tag_eq(tag: &ParsedTag, name: &[u8]) -> bool {
    if tag.name_len != name.len() {
        return false;
    }
    for i in 0..name.len() {
        if tag.name[i] != ascii_lower(name[i]) {
            return false;
        }
    }
    true
}

fn tag_is_void(tag: &ParsedTag) -> bool {
    tag_eq(tag, b"area")
        || tag_eq(tag, b"base")
        || tag_eq(tag, b"br")
        || tag_eq(tag, b"col")
        || tag_eq(tag, b"embed")
        || tag_eq(tag, b"hr")
        || tag_eq(tag, b"img")
        || tag_eq(tag, b"input")
        || tag_eq(tag, b"link")
        || tag_eq(tag, b"meta")
        || tag_eq(tag, b"param")
        || tag_eq(tag, b"source")
        || tag_eq(tag, b"track")
        || tag_eq(tag, b"wbr")
}

fn tag_supports_padding(tag: &ParsedTag) -> bool {
    tag_eq(tag, b"body")
        || tag_eq(tag, b"div")
        || tag_eq(tag, b"section")
        || tag_eq(tag, b"header")
        || tag_eq(tag, b"footer")
        || tag_eq(tag, b"main")
        || tag_eq(tag, b"article")
        || tag_eq(tag, b"aside")
        || tag_eq(tag, b"nav")
        || tag_eq(tag, b"h1")
        || tag_eq(tag, b"h2")
        || tag_eq(tag, b"h3")
}

fn skip_element(body: &[u8], start: usize, name: [u8; 12], name_len: usize) -> usize {
    let mut i = start;
    while i + name_len + 3 < body.len() {
        if body[i] == b'<' && body[i + 1] == b'/' {
            let mut ok = true;
            for n in 0..name_len {
                if ascii_lower(body[i + 2 + n]) != name[n] {
                    ok = false;
                    break;
                }
            }
            if ok {
                if let Some(end) = find_byte(body, i + 2 + name_len, b'>') {
                    return end + 1;
                }
                return body.len();
            }
        }
        i += 1;
    }
    body.len()
}

fn find_next_js_visible_op(script: &[u8], start: usize) -> Option<usize> {
    let needles: [&[u8]; 4] = [b"document.write", b"innerHTML", b"textContent", b"innerText"];
    let mut best = usize::MAX;
    for needle in needles {
        if let Some(pos) = find_subslice_ci(script, needle, start) {
            if pos < best {
                best = pos;
            }
        }
    }
    if best == usize::MAX {
        None
    } else {
        Some(best)
    }
}

fn copy_js_quoted_text(src: &[u8], out: &mut [u8; 512], html_like: &mut bool) -> usize {
    let mut out_len = 0usize;
    let mut i = 0usize;
    while i < src.len() && out_len + 1 < out.len() {
        let quote = src[i];
        if quote != b'\'' && quote != b'"' && quote != b'`' {
            i += 1;
            continue;
        }
        i += 1;
        if out_len > 0 && out[out_len - 1] != b' ' {
            out[out_len] = b' ';
            out_len += 1;
        }
        while i < src.len() && out_len + 1 < out.len() {
            let b = src[i];
            if b == quote {
                i += 1;
                break;
            }
            if b == b'\\' && i + 1 < src.len() {
                let esc = src[i + 1];
                let mapped = match esc {
                    b'n' | b'r' | b't' => b' ',
                    b'\'' => b'\'',
                    b'"' => b'"',
                    b'`' => b'`',
                    b'\\' => b'\\',
                    b'/' => b'/',
                    _ => esc,
                };
                if mapped == b'<' {
                    *html_like = true;
                }
                if mapped >= 32 {
                    out[out_len] = mapped;
                    out_len += 1;
                }
                i += 2;
                continue;
            }
            if b == b'<' {
                *html_like = true;
            }
            if b >= 32 {
                out[out_len] = b;
                out_len += 1;
            }
            i += 1;
        }
    }
    while out_len > 0 && out[out_len - 1] == b' ' {
        out_len -= 1;
    }
    out_len
}

fn useful_js_text(text: &[u8]) -> bool {
    let mut letters = 0usize;
    let mut only_symbols = true;
    for &b in text {
        if (b'A' <= b && b <= b'Z') || (b'a' <= b && b <= b'z') || b >= 0x80 {
            letters += 1;
            only_symbols = false;
        }
        if b == b'<' || b == b'>' {
            only_symbols = false;
        }
    }
    letters >= 2 && !only_symbols
}

fn decode_entity(body: &[u8], start: usize) -> (u8, usize) {
    let rest = &body[start..];
    if starts_with_ci(rest, b"&amp;") {
        return (b'&', start + 5);
    }
    if starts_with_ci(rest, b"&lt;") {
        return (b'<', start + 4);
    }
    if starts_with_ci(rest, b"&gt;") {
        return (b'>', start + 4);
    }
    if starts_with_ci(rest, b"&quot;") {
        return (b'"', start + 6);
    }
    if starts_with_ci(rest, b"&nbsp;") {
        return (b' ', start + 6);
    }
    (b' ', start + 1)
}

fn decode_utf8_to_ascii(body: &[u8], start: usize) -> ([u8; 4], usize, usize) {
    let mut out = [b'?'; 4];
    if start >= body.len() {
        return (out, 0, start);
    }
    let first = body[start];
    let (cp, next) = if first & 0xE0 == 0xC0 && start + 1 < body.len() {
        let b1 = body[start + 1];
        ((((first & 0x1F) as u32) << 6) | ((b1 & 0x3F) as u32), start + 2)
    } else if first & 0xF0 == 0xE0 && start + 2 < body.len() {
        let b1 = body[start + 1];
        let b2 = body[start + 2];
        (
            (((first & 0x0F) as u32) << 12) | (((b1 & 0x3F) as u32) << 6) | ((b2 & 0x3F) as u32),
            start + 3,
        )
    } else if first & 0xF8 == 0xF0 && start + 3 < body.len() {
        let b1 = body[start + 1];
        let b2 = body[start + 2];
        let b3 = body[start + 3];
        (
            (((first & 0x07) as u32) << 18)
                | (((b1 & 0x3F) as u32) << 12)
                | (((b2 & 0x3F) as u32) << 6)
                | ((b3 & 0x3F) as u32),
            start + 4,
        )
    } else {
        (first as u32, start + 1)
    };

    if cp == 0x00A0 {
        out[0] = b' ';
        return (out, 1, next);
    }
    if cp == 0x2013 || cp == 0x2014 || cp == 0x2212 {
        out[0] = b'-';
        return (out, 1, next);
    }
    if cp == 0x2018 || cp == 0x2019 {
        out[0] = b'\'';
        return (out, 1, next);
    }
    if cp == 0x201C || cp == 0x201D {
        out[0] = b'"';
        return (out, 1, next);
    }
    if cp == 0x2026 {
        out = [b'.', b'.', b'.', b'?'];
        return (out, 3, next);
    }
    if cp >= 0x0400 && cp <= 0x04FF {
        return (out, cyrillic_ascii(cp, &mut out), next);
    }
    if cp < 128 {
        out[0] = cp as u8;
        return (out, 1, next);
    }
    (out, 1, next)
}

fn cyrillic_ascii(cp: u32, out: &mut [u8; 4]) -> usize {
    match cp {
        0x0410 => one(out, b'A'),
        0x0430 => one(out, b'a'),
        0x0411 => one(out, b'B'),
        0x0431 => one(out, b'b'),
        0x0412 => one(out, b'V'),
        0x0432 => one(out, b'v'),
        0x0413 | 0x0490 => one(out, b'G'),
        0x0433 | 0x0491 => one(out, b'g'),
        0x0414 => one(out, b'D'),
        0x0434 => one(out, b'd'),
        0x0415 => one(out, b'E'),
        0x0435 => one(out, b'e'),
        0x0404 => two(out, b'Y', b'e'),
        0x0454 => two(out, b'y', b'e'),
        0x0401 | 0x0416 => two(out, b'Z', b'h'),
        0x0451 | 0x0436 => two(out, b'z', b'h'),
        0x0417 => one(out, b'Z'),
        0x0437 => one(out, b'z'),
        0x0418 => one(out, b'I'),
        0x0438 => one(out, b'i'),
        0x0406 => one(out, b'I'),
        0x0456 => one(out, b'i'),
        0x0407 => two(out, b'Y', b'i'),
        0x0457 => two(out, b'y', b'i'),
        0x0419 => one(out, b'Y'),
        0x0439 => one(out, b'y'),
        0x041A => one(out, b'K'),
        0x043A => one(out, b'k'),
        0x041B => one(out, b'L'),
        0x043B => one(out, b'l'),
        0x041C => one(out, b'M'),
        0x043C => one(out, b'm'),
        0x041D => one(out, b'N'),
        0x043D => one(out, b'n'),
        0x041E => one(out, b'O'),
        0x043E => one(out, b'o'),
        0x041F => one(out, b'P'),
        0x043F => one(out, b'p'),
        0x0420 => one(out, b'R'),
        0x0440 => one(out, b'r'),
        0x0421 => one(out, b'S'),
        0x0441 => one(out, b's'),
        0x0422 => one(out, b'T'),
        0x0442 => one(out, b't'),
        0x0423 => one(out, b'U'),
        0x0443 => one(out, b'u'),
        0x0424 => one(out, b'F'),
        0x0444 => one(out, b'f'),
        0x0425 => two(out, b'K', b'h'),
        0x0445 => two(out, b'k', b'h'),
        0x0426 => two(out, b'T', b's'),
        0x0446 => two(out, b't', b's'),
        0x0427 => two(out, b'C', b'h'),
        0x0447 => two(out, b'c', b'h'),
        0x0428 => two(out, b'S', b'h'),
        0x0448 => two(out, b's', b'h'),
        0x0429 => four(out, b'S', b'h', b'c', b'h'),
        0x0449 => four(out, b's', b'h', b'c', b'h'),
        0x042B => one(out, b'Y'),
        0x044B => one(out, b'y'),
        0x042D => one(out, b'E'),
        0x044D => one(out, b'e'),
        0x042E => two(out, b'Y', b'u'),
        0x044E => two(out, b'y', b'u'),
        0x042F => two(out, b'Y', b'a'),
        0x044F => two(out, b'y', b'a'),
        _ => one(out, b'?'),
    }
}

fn one(out: &mut [u8; 4], a: u8) -> usize {
    out[0] = a;
    1
}

fn two(out: &mut [u8; 4], a: u8, b: u8) -> usize {
    out[0] = a;
    out[1] = b;
    2
}

fn four(out: &mut [u8; 4], a: u8, b: u8, c: u8, d: u8) -> usize {
    out[0] = a;
    out[1] = b;
    out[2] = c;
    out[3] = d;
    4
}

fn find_attr(tag: &[u8], attr: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0usize;
    while i < tag.len() {
        while i < tag.len() && !is_name_byte(tag[i]) {
            i += 1;
        }
        let name_start = i;
        while i < tag.len() && is_name_byte(tag[i]) {
            i += 1;
        }
        let name_end = i;
        while i < tag.len() && tag[i] == b' ' {
            i += 1;
        }
        if i >= tag.len() || tag[i] != b'=' {
            continue;
        }
        i += 1;
        while i < tag.len() && tag[i] == b' ' {
            i += 1;
        }
        if i >= tag.len() {
            return None;
        }
        let quote = if tag[i] == b'"' || tag[i] == b'\'' {
            let q = tag[i];
            i += 1;
            q
        } else {
            0
        };
        let value_start = i;
        if quote != 0 {
            while i < tag.len() && tag[i] != quote {
                i += 1;
            }
        } else {
            while i < tag.len() && tag[i] != b' ' && tag[i] != b'/' {
                i += 1;
            }
        }
        let value_len = i.saturating_sub(value_start);
        if slice_eq_ci(&tag[name_start..name_end], attr) {
            return Some((value_start, value_len));
        }
        if quote != 0 && i < tag.len() {
            i += 1;
        }
    }
    None
}

fn link_is_stylesheet(tag: &[u8]) -> bool {
    let (start, len) = match find_attr(tag, b"rel") {
        Some(v) => v,
        None => return false,
    };
    contains_word_ci(&tag[start..start + len], b"stylesheet")
}

fn apply_css_rules_to_tag(tag: &[u8], parsed: &ParsedTag, style: &mut RenderStyle) {
    unsafe {
        for i in 0..CSS_RULE_COUNT {
            let rule = CSS_RULES[i];
            if rule.name_len == 0 {
                continue;
            }
            let name = &rule.name[..rule.name_len];
            let matched = match rule.selector_kind {
                CSS_SEL_TAG => parsed.name_len == rule.name_len
                    && slice_eq_ci(&parsed.name[..parsed.name_len], name),
                CSS_SEL_CLASS => tag_has_class(tag, name),
                CSS_SEL_ID => tag_attr_equals(tag, b"id", name),
                _ => false,
            };
            if matched {
                if let Some(fg) = rule.fg {
                    style.fg = fg;
                }
                if let Some(bg) = rule.bg {
                    style.bg = bg;
                }
                if rule.has_kind {
                    style.kind = rule.kind;
                }
                if rule.has_align {
                    style.align = rule.align;
                }
                if rule.hidden {
                    style.hidden = true;
                }
                if rule.pad_before > 0 {
                    style.pad_before = rule.pad_before;
                }
                if rule.pad_after > 0 {
                    style.pad_after = rule.pad_after;
                }
                if rule.image_url_len > 0 {
                    style.image_url = [0; MAX_URL];
                    style.image_url_len = rule.image_url_len.min(MAX_URL);
                    style.image_url[..style.image_url_len]
                        .copy_from_slice(&rule.image_url[..style.image_url_len]);
                }
            }
        }
    }
}

fn apply_decl_to_render_style(decl: CssDecl, style: &mut RenderStyle) {
    if let Some(fg) = decl.fg {
        style.fg = fg;
    }
    if let Some(bg) = decl.bg {
        style.bg = bg;
    }
    if decl.has_kind {
        style.kind = decl.kind;
    }
    if decl.has_align {
        style.align = decl.align;
    }
    if decl.hidden {
        style.hidden = true;
    }
    if decl.pad_before > 0 {
        style.pad_before = decl.pad_before;
    }
    if decl.pad_after > 0 {
        style.pad_after = decl.pad_after;
    }
    if decl.image_url_len > 0 {
        style.image_url = [0; MAX_URL];
        style.image_url_len = decl.image_url_len.min(MAX_URL);
        style.image_url[..style.image_url_len].copy_from_slice(&decl.image_url[..style.image_url_len]);
    }
}

fn tag_has_class(tag: &[u8], class: &[u8]) -> bool {
    let (start, len) = match find_attr(tag, b"class") {
        Some(v) => v,
        None => return false,
    };
    let value = &tag[start..start + len];
    let mut i = 0usize;
    while i < value.len() {
        while i < value.len() && (value[i] == b' ' || value[i] == b'\t' || value[i] == b'\r' || value[i] == b'\n')
        {
            i += 1;
        }
        let word_start = i;
        while i < value.len() && value[i] != b' ' && value[i] != b'\t' && value[i] != b'\r' && value[i] != b'\n'
        {
            i += 1;
        }
        if slice_eq_ci(&value[word_start..i], class) {
            return true;
        }
    }
    false
}

fn tag_attr_equals(tag: &[u8], attr: &[u8], expected: &[u8]) -> bool {
    let (start, len) = match find_attr(tag, attr) {
        Some(v) => v,
        None => return false,
    };
    slice_eq_ci(trim_ascii(&tag[start..start + len]), expected)
}

fn parse_style_color(style: &[u8], prop: &[u8]) -> Option<u32> {
    let (start, len) = find_style_property(style, prop)?;
    parse_color_value(&style[start..start + len])
}

fn style_property_has_word(style: &[u8], prop: &[u8], word: &[u8]) -> bool {
    let (start, len) = match find_style_property(style, prop) {
        Some(v) => v,
        None => return false,
    };
    contains_word_ci(&style[start..start + len], word)
}

fn find_style_property(style: &[u8], prop: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0usize;
    while i < style.len() {
        while i < style.len() && (style[i] == b' ' || style[i] == b';') {
            i += 1;
        }
        let name_start = i;
        while i < style.len() && style[i] != b':' && style[i] != b';' {
            i += 1;
        }
        let mut name_end = i;
        while name_end > name_start && style[name_end - 1] == b' ' {
            name_end -= 1;
        }
        if i < style.len() && style[i] == b':' {
            i += 1;
            while i < style.len() && style[i] == b' ' {
                i += 1;
            }
            let value_start = i;
            while i < style.len() && style[i] != b';' {
                i += 1;
            }
            let mut value_end = i;
            while value_end > value_start && style[value_end - 1] == b' ' {
                value_end -= 1;
            }
            if slice_eq_ci(&style[name_start..name_end], prop) {
                return Some((value_start, value_end.saturating_sub(value_start)));
            }
        } else {
            while i < style.len() && style[i] != b';' {
                i += 1;
            }
        }
        if i < style.len() && style[i] == b';' {
            i += 1;
        }
    }
    None
}

fn parse_color_value(value: &[u8]) -> Option<u32> {
    let mut start = 0usize;
    while start < value.len() && (value[start] == b' ' || value[start] == b'"' || value[start] == b'\'') {
        start += 1;
    }
    if start >= value.len() {
        return None;
    }
    parse_css_value(&value[start..])
}

fn hex_pair(a: u8, b: u8) -> Option<u8> {
    Some((hex_nibble(a)? << 4) | hex_nibble(b)?)
}

fn hex_nibble(b: u8) -> Option<u8> {
    if b'0' <= b && b <= b'9' {
        Some(b - b'0')
    } else if b'a' <= b && b <= b'f' {
        Some(b - b'a' + 10)
    } else if b'A' <= b && b <= b'F' {
        Some(b - b'A' + 10)
    } else {
        None
    }
}

fn bg_if_none(bg: u32, fallback: u32) -> u32 {
    if bg == STYLE_NONE {
        fallback
    } else {
        bg
    }
}

fn readable_text_for(bg: u32) -> u32 {
    if brightness(bg) < 128 {
        0x00FFFFFF
    } else {
        0x00192533
    }
}

fn low_contrast(fg: u32, bg: u32) -> bool {
    let fb = brightness(fg);
    let bb = brightness(bg);
    if fb > bb {
        fb - bb < 46
    } else {
        bb - fb < 46
    }
}

fn brightness(rgb: u32) -> i32 {
    let r = ((rgb >> 16) & 0xFF) as i32;
    let g = ((rgb >> 8) & 0xFF) as i32;
    let b = (rgb & 0xFF) as i32;
    (r * 30 + g * 59 + b * 11) / 100
}

fn kind_name(kind: NetKind) -> &'static [u8] {
    match kind {
        NetKind::IntelE1000 => b"Intel E1000",
        NetKind::Realtek8139 => b"Realtek RTL8139",
        NetKind::VirtioNet => b"VirtIO Net",
        NetKind::Unknown => b"Unknown NIC",
    }
}

fn state_name(state: NetState) -> &'static [u8] {
    match state {
        NetState::Detected => b"Detected",
        NetState::Ready => b"Ready",
        NetState::Error => b"Error",
    }
}

fn fetch_code_name(code: FetchCode) -> &'static [u8] {
    match code {
        FetchCode::Ok => b"OK",
        FetchCode::BufferFull => b"OK (buffer full)",
        FetchCode::BadUrl => b"Bad URL",
        FetchCode::HttpsUnsupported => b"HTTPS not supported",
        FetchCode::TlsHandshakeOnly => b"TLS answered (decryptor missing)",
        FetchCode::NoNic => b"No NIC",
        FetchCode::ArpFailed => b"ARP failed",
        FetchCode::DnsFailed => b"DNS failed",
        FetchCode::TcpFailed => b"TCP failed",
        FetchCode::Timeout => b"Timeout",
    }
}

fn prepare_request_url(input: &[u8], out: &mut [u8; MAX_URL]) -> usize {
    let mut start = 0usize;
    while start < input.len() && input[start] == b' ' {
        start += 1;
    }
    let mut end = input.len();
    while end > start && input[end - 1] == b' ' {
        end -= 1;
    }
    if start >= end {
        return 0;
    }
    let src = &input[start..end];
    if starts_with_ci(src, b"http://") {
        return copy_url_normalized_scheme(out, src);
    }
    if starts_with_ci(src, b"https://") {
        let mut p = copy_limited(out, b"http://10.0.2.2:18080/");
        p += copy_url_normalized_scheme(&mut out[p..], src);
        return p;
    }
    if looks_like_host(src) {
        let mut p = copy_limited(out, b"http://10.0.2.2:18080/https://");
        p += copy_limited(&mut out[p..], src);
        return p;
    }
    let mut p = copy_limited(out, b"http://10.0.2.2:18080/https://duckduckgo.com/html/?q=");
    p += write_query_encoded(&mut out[p..], src);
    p
}

fn is_https_proxy_url(url: &[u8]) -> bool {
    starts_with_ci(url, b"http://10.0.2.2:18080/")
}

fn prepare_image_request_url(input: &[u8], out: &mut [u8; MAX_URL]) -> usize {
    let src = trim_ascii(input);
    if src.is_empty() {
        return 0;
    }
    let mut p = copy_limited(out, b"http://10.0.2.2:18080/img/");
    if starts_with_ci(src, b"http://") || starts_with_ci(src, b"https://") {
        p += copy_url_normalized_scheme(&mut out[p..], src);
    } else if looks_like_host(src) {
        p += copy_limited(&mut out[p..], b"http://");
        p += copy_limited(&mut out[p..], src);
    } else {
        return 0;
    }
    p
}

fn build_url_with_base(base: &[u8], href: &[u8], url: &mut [u8; MAX_URL]) -> usize {
    let href = trim_ascii(href);
    if href.is_empty() || starts_with_ci(href, b"javascript:") || starts_with_ci(href, b"data:") || href[0] == b'#' {
        return 0;
    }
    let mut p = 0usize;
    if starts_with_ci(href, b"http://") || starts_with_ci(href, b"https://") {
        p += copy_limited(&mut url[p..], href);
    } else if href.len() > 2 && href[0] == b'/' && href[1] == b'/' {
        p += copy_limited(&mut url[p..], b"https:");
        p += copy_limited(&mut url[p..], href);
    } else if href[0] == b'/' {
        p += copy_current_origin(base, &mut url[p..]);
        p += copy_limited(&mut url[p..], href);
    } else {
        p += copy_current_base(base, &mut url[p..]);
        p += copy_limited(&mut url[p..], href);
    }
    p
}

fn is_plain_search_query(input: &[u8]) -> bool {
    let src = trim_ascii(input);
    if src.is_empty() {
        return false;
    }
    if starts_with_ci(src, b"http://") || starts_with_ci(src, b"https://") {
        return false;
    }
    !looks_like_host(src)
}

fn looks_like_host(src: &[u8]) -> bool {
    let mut dot = false;
    for &b in src {
        if b == b' ' || b == b'\t' {
            return false;
        }
        if b == b'.' {
            dot = true;
        }
    }
    dot
}

fn copy_limited(out: &mut [u8], src: &[u8]) -> usize {
    let mut n = 0usize;
    while n < out.len() && n < src.len() {
        out[n] = src[n];
        n += 1;
    }
    n
}

fn copy_url_normalized_scheme(out: &mut [u8], src: &[u8]) -> usize {
    if starts_with_ci(src, b"https://") {
        let mut p = copy_limited(out, b"https://");
        p += copy_limited(&mut out[p..], &src[8..]);
        return p;
    }
    if starts_with_ci(src, b"http://") {
        let mut p = copy_limited(out, b"http://");
        p += copy_limited(&mut out[p..], &src[7..]);
        return p;
    }
    copy_limited(out, src)
}

fn copy_current_origin(current: &[u8], out: &mut [u8]) -> usize {
    let cur = trim_ascii(current);
    let mut p = 0usize;
    let mut start = 0usize;
    if starts_with_ci(cur, b"http://") {
        p += copy_limited(&mut out[p..], b"http://");
        start = 7;
    } else if starts_with_ci(cur, b"https://") {
        p += copy_limited(&mut out[p..], b"https://");
        start = 8;
    } else {
        p += copy_limited(&mut out[p..], b"https://");
    }
    let mut i = start;
    while i < cur.len() && cur[i] != b'/' && cur[i] != b' ' && cur[i] != 0 {
        if p >= out.len() {
            return p;
        }
        out[p] = cur[i];
        p += 1;
        i += 1;
    }
    p
}

fn copy_current_base(current: &[u8], out: &mut [u8]) -> usize {
    let mut p = copy_current_origin(current, out);
    let cur = trim_ascii(current);
    let mut path_start = 0usize;
    if starts_with_ci(cur, b"http://") {
        path_start = 7;
    } else if starts_with_ci(cur, b"https://") {
        path_start = 8;
    }
    while path_start < cur.len() && cur[path_start] != b'/' {
        path_start += 1;
    }
    if path_start >= cur.len() {
        if p < out.len() {
            out[p] = b'/';
            p += 1;
        }
        return p;
    }
    let mut last_slash = path_start;
    let mut i = path_start;
    while i < cur.len() && cur[i] != b' ' && cur[i] != 0 {
        if cur[i] == b'/' {
            last_slash = i;
        }
        i += 1;
    }
    let path = &cur[path_start..=last_slash];
    p += copy_limited(&mut out[p..], path);
    p
}

fn write_query_encoded(out: &mut [u8], src: &[u8]) -> usize {
    let mut p = 0usize;
    for &b in src {
        if p >= out.len() {
            break;
        }
        if b == b' ' || b == b'\t' {
            out[p] = b'+';
            p += 1;
        } else if (b'0' <= b && b <= b'9')
            || (b'a' <= b && b <= b'z')
            || (b'A' <= b && b <= b'Z')
            || b == b'-'
            || b == b'_'
            || b == b'.'
        {
            out[p] = b;
            p += 1;
        }
    }
    p
}

fn find_byte(buf: &[u8], mut start: usize, b: u8) -> Option<usize> {
    while start < buf.len() {
        if buf[start] == b {
            return Some(start);
        }
        start += 1;
    }
    None
}

fn is_name_byte(b: u8) -> bool {
    (b'a' <= b && b <= b'z')
        || (b'A' <= b && b <= b'Z')
        || (b'0' <= b && b <= b'9')
        || b == b'-'
        || b == b'_'
}

fn slice_eq_ci(a: &[u8], b: &[u8]) -> bool {
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

fn contains_byte(buf: &[u8], byte: u8) -> bool {
    for &b in buf {
        if b == byte {
            return true;
        }
    }
    false
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

fn write_str(buf: &mut [u8], s: &[u8]) -> usize {
    let mut n = 0;
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
    let mut len = 0;
    while val > 0 && len < tmp.len() {
        tmp[len] = b'0' + (val % 10) as u8;
        val /= 10;
        len += 1;
    }
    let mut out = 0;
    while len > 0 && out < buf.len() {
        len -= 1;
        buf[out] = tmp[len];
        out += 1;
    }
    out
}

fn calc_rect(fb: &Framebuffer) -> (usize, usize, usize, usize) {
    let w = (fb.width * 4 / 5).min(920).max(520);
    let h = (fb.height * 4 / 5).min(620).max(360);
    let x = (fb.width.saturating_sub(w)) / 2;
    let y = (fb.height.saturating_sub(h)) / 2;
    (x, y, w, h)
}

fn hit(px: usize, py: usize, x: usize, y: usize, w: usize, h: usize) -> bool {
    px >= x && py >= y && px < x + w && py < y + h
}

fn fill_vertical_gradient(
    fb: &Framebuffer,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    top: u32,
    bottom: u32,
) {
    if w == 0 || h == 0 {
        return;
    }
    if h == 1 {
        display::fill_rect(fb, x, y, w, 1, top);
        return;
    }
    let den = (h - 1) as u32;
    for row in 0..h {
        let c = lerp_rgb(top, bottom, row as u32, den);
        display::fill_rect(fb, x, y + row, w, 1, c);
    }
}

fn lerp_rgb(a: u32, b: u32, num: u32, den: u32) -> u32 {
    if den == 0 {
        return a;
    }
    let ar = ((a >> 16) & 0xFF) as u32;
    let ag = ((a >> 8) & 0xFF) as u32;
    let ab = (a & 0xFF) as u32;
    let br = ((b >> 16) & 0xFF) as u32;
    let bg = ((b >> 8) & 0xFF) as u32;
    let bb = (b & 0xFF) as u32;
    let r = (ar * (den - num) + br * num) / den;
    let g = (ag * (den - num) + bg * num) / den;
    let b = (ab * (den - num) + bb * num) / den;
    (r << 16) | (g << 8) | b
}
