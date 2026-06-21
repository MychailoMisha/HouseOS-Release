use crate::display::Framebuffer;

const MAX_DIRTY_RECTS: usize = 48;
const MIN_DIRTY_AREA_THRESHOLD: usize = 4 * 1024;
const DIRTY_RATIO_DIVISOR: usize = 3;
const LOW_MEM_DIRTY_RATIO_DIVISOR: usize = 5;
const NEARBY_MERGE_GAP: usize = 8;
const DIRTY_EDGE_PAD: usize = 10;
const LOW_MEMORY_SKIP_DIV: usize = 2;
const MAX_FULL_REDRAW_STREAK: usize = 300;
const LOW_MEM_RECOVERY_FRAMES: usize = 480;

#[derive(Clone, Copy, Debug)]
pub struct DirtyRect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl DirtyRect {
    pub const EMPTY: DirtyRect = DirtyRect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };

    pub const fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Self { x, y, w, h }
    }

    #[inline]
    fn right(&self) -> usize {
        self.x.saturating_add(self.w)
    }

    #[inline]
    fn bottom(&self) -> usize {
        self.y.saturating_add(self.h)
    }

    #[inline]
    fn area(&self) -> usize {
        self.w.saturating_mul(self.h)
    }

    #[inline]
    fn contains(&self, other: &DirtyRect) -> bool {
        self.x <= other.x
            && self.y <= other.y
            && self.right() >= other.right()
            && self.bottom() >= other.bottom()
    }

    #[inline]
    fn intersects_or_nearby(&self, other: &DirtyRect) -> bool {
        let self_right = self.right().saturating_add(NEARBY_MERGE_GAP);
        let other_right = other.right().saturating_add(NEARBY_MERGE_GAP);
        let self_bottom = self.bottom().saturating_add(NEARBY_MERGE_GAP);
        let other_bottom = other.bottom().saturating_add(NEARBY_MERGE_GAP);
        !(self_right <= other.x
            || other_right <= self.x
            || self_bottom <= other.y
            || other_bottom <= self.y)
    }

    #[inline]
    fn merge(&mut self, other: &DirtyRect) {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = self.right().max(other.right());
        let y2 = self.bottom().max(other.bottom());
        self.x = x1;
        self.y = y1;
        self.w = x2.saturating_sub(x1);
        self.h = y2.saturating_sub(y1);
    }

    pub fn merged(&self, other: &DirtyRect) -> Self {
        let mut result = *self;
        result.merge(other);
        result
    }
}

pub struct Optimizer {
    dirty_rects: [DirtyRect; MAX_DIRTY_RECTS],
    dirty_count: usize,
    dirty_area: usize,
    full_redraw_needed: bool,
    frame_counter: usize,
    optimization_enabled: bool,
    low_memory_mode: bool,
    low_mem_skip_counter: usize,
    full_redraw_streak: usize,
    frames_since_last_full_redraw: usize,
    screen_w: usize,
    screen_h: usize,
    screen_area: usize,
}

impl Optimizer {
    pub const fn new() -> Self {
        Self {
            dirty_rects: [DirtyRect::EMPTY; MAX_DIRTY_RECTS],
            dirty_count: 0,
            dirty_area: 0,
            full_redraw_needed: true,
            frame_counter: 0,
            optimization_enabled: true,
            low_memory_mode: false,
            low_mem_skip_counter: 0,
            full_redraw_streak: 0,
            frames_since_last_full_redraw: 0,
            screen_w: 0,
            screen_h: 0,
            screen_area: 1,
        }
    }

    pub fn init(&mut self, fb: &Framebuffer) {
        self.screen_w = fb.width;
        self.screen_h = fb.height;
        self.screen_area = fb.width.saturating_mul(fb.height).max(1);
        self.full_redraw_needed = true;
        self.frame_counter = 0;
        self.low_mem_skip_counter = 0;
        self.full_redraw_streak = 0;
        self.frames_since_last_full_redraw = 0;
        self.clear_dirty_rects();
    }

    pub fn begin_frame(&mut self) -> bool {
        if !self.optimization_enabled {
            return true;
        }
        if self.full_redraw_needed {
            return true;
        }

        self.frame_counter = self.frame_counter.wrapping_add(1);
        self.frames_since_last_full_redraw = self.frames_since_last_full_redraw.saturating_add(1);

        if self.low_memory_mode && self.frames_since_last_full_redraw > LOW_MEM_RECOVERY_FRAMES {
            self.low_memory_mode = false;
            self.low_mem_skip_counter = 0;
        }

        if self.low_memory_mode && self.dirty_count == 0 {
            self.low_mem_skip_counter = self.low_mem_skip_counter.wrapping_add(1);
            if self.low_mem_skip_counter % LOW_MEMORY_SKIP_DIV != 0 {
                return false;
            }
        } else {
            self.low_mem_skip_counter = 0;
        }

        true
    }

    pub fn end_frame(&mut self) {
        if !self.optimization_enabled {
            return;
        }
        if !self.full_redraw_needed {
            self.clear_dirty_rects();
        }
    }

    pub fn add_dirty_rect(&mut self, x: usize, y: usize, w: usize, h: usize) {
        if !self.optimization_enabled || w == 0 || h == 0 || self.full_redraw_needed {
            return;
        }
        let mut rect = match self.clamp_to_screen(x, y, w, h) {
            Some(r) => r,
            None => return,
        };
        rect = self.pad_rect(rect);

        for i in 0..self.dirty_count {
            if self.dirty_rects[i].contains(&rect) {
                return;
            }
        }

        let mut i = 0usize;
        while i < self.dirty_count {
            if rect.contains(&self.dirty_rects[i]) {
                self.remove_dirty_rect(i);
            } else {
                i += 1;
            }
        }

        if self.dirty_count >= MAX_DIRTY_RECTS {
            self.request_full_redraw();
            return;
        }

        self.dirty_rects[self.dirty_count] = rect;
        self.dirty_count += 1;
        self.coalesce_dirty_rects();
        self.recalc_dirty_area();
        self.check_dirty_budget();
    }

    pub fn should_redraw_full(&self) -> bool {
        self.full_redraw_needed || !self.optimization_enabled
    }

    pub fn mark_clean(&mut self) {
        self.full_redraw_needed = false;
        self.full_redraw_streak = 0;
        self.frames_since_last_full_redraw = 0;
        self.clear_dirty_rects();
    }

    pub fn dirty_rects(&self) -> &[DirtyRect] {
        &self.dirty_rects[..self.dirty_count]
    }

    pub fn dirty_bounding_box(&self) -> Option<DirtyRect> {
        if self.dirty_count == 0 {
            return None;
        }
        let mut bbox = self.dirty_rects[0];
        for i in 1..self.dirty_count {
            bbox.merge(&self.dirty_rects[i]);
        }
        Some(bbox)
    }

    pub fn prevent_hang(&mut self) -> bool {
        if !self.optimization_enabled {
            return false;
        }
        if self.full_redraw_needed {
            self.full_redraw_streak = self.full_redraw_streak.saturating_add(1);
        } else if self.full_redraw_streak > 0 {
            self.full_redraw_streak -= 1;
        }
        if self.full_redraw_streak > MAX_FULL_REDRAW_STREAK {
            self.low_memory_mode = true;
            self.request_full_redraw();
            self.full_redraw_streak = 0;
            return true;
        }
        false
    }

    pub fn reset_hang_protection(&mut self) {
        self.low_memory_mode = false;
        self.low_mem_skip_counter = 0;
        self.full_redraw_streak = 0;
    }

    pub fn toggle_optimization(&mut self) {
        self.optimization_enabled = !self.optimization_enabled;
        self.full_redraw_needed = true;
        self.clear_dirty_rects();
    }

    fn request_full_redraw(&mut self) {
        self.full_redraw_needed = true;
        self.clear_dirty_rects();
    }

    fn clamp_to_screen(&self, x: usize, y: usize, w: usize, h: usize) -> Option<DirtyRect> {
        if self.screen_w == 0 || self.screen_h == 0 {
            return Some(DirtyRect::new(x, y, w, h));
        }
        if x >= self.screen_w || y >= self.screen_h {
            return None;
        }
        let end_x = x.saturating_add(w).min(self.screen_w);
        let end_y = y.saturating_add(h).min(self.screen_h);
        if end_x <= x || end_y <= y {
            return None;
        }
        Some(DirtyRect::new(x, y, end_x - x, end_y - y))
    }

    fn pad_rect(&self, rect: DirtyRect) -> DirtyRect {
        if self.screen_w == 0 || self.screen_h == 0 {
            return rect;
        }
        let x = rect.x.saturating_sub(DIRTY_EDGE_PAD);
        let y = rect.y.saturating_sub(DIRTY_EDGE_PAD);
        let end_x = rect.right().saturating_add(DIRTY_EDGE_PAD).min(self.screen_w);
        let end_y = rect.bottom().saturating_add(DIRTY_EDGE_PAD).min(self.screen_h);
        DirtyRect::new(x, y, end_x.saturating_sub(x), end_y.saturating_sub(y))
    }

    fn dirty_area_threshold(&self) -> usize {
        let divisor = if self.low_memory_mode {
            LOW_MEM_DIRTY_RATIO_DIVISOR
        } else {
            DIRTY_RATIO_DIVISOR
        };
        let by_ratio = self.screen_area / divisor.max(1);
        by_ratio.min(self.screen_area).max(MIN_DIRTY_AREA_THRESHOLD)
    }

    fn check_dirty_budget(&mut self) {
        if self.dirty_count >= MAX_DIRTY_RECTS || self.dirty_area > self.dirty_area_threshold() {
            self.request_full_redraw();
        }
    }

    fn coalesce_dirty_rects(&mut self) {
        if self.dirty_count <= 1 {
            return;
        }

        let mut changed = true;
        while changed {
            changed = false;
            let mut i = 0usize;
            while i < self.dirty_count {
                let mut j = i + 1;
                while j < self.dirty_count {
                    let a = self.dirty_rects[i];
                    let b = self.dirty_rects[j];
                    if a.intersects_or_nearby(&b) {
                        let mut merged = a;
                        merged.merge(&b);
                        self.dirty_rects[i] = merged;
                        self.remove_dirty_rect(j);
                        changed = true;
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
        }
    }

    fn remove_dirty_rect(&mut self, index: usize) {
        if index >= self.dirty_count {
            return;
        }
        let mut i = index;
        while i + 1 < self.dirty_count {
            self.dirty_rects[i] = self.dirty_rects[i + 1];
            i += 1;
        }
        self.dirty_count -= 1;
        self.dirty_rects[self.dirty_count] = DirtyRect::EMPTY;
    }

    fn recalc_dirty_area(&mut self) {
        let mut area = 0usize;
        for i in 0..self.dirty_count {
            area = area.saturating_add(self.dirty_rects[i].area());
        }
        self.dirty_area = area;
    }

    fn clear_dirty_rects(&mut self) {
        for i in 0..self.dirty_count {
            self.dirty_rects[i] = DirtyRect::EMPTY;
        }
        self.dirty_count = 0;
        self.dirty_area = 0;
    }
}

static mut OPTIMIZER: Option<Optimizer> = None;

pub fn init_optimizer(fb: &Framebuffer) {
    unsafe {
        let mut opt = Optimizer::new();
        opt.init(fb);
        OPTIMIZER = Some(opt);
    }
}

pub fn get_optimizer() -> Option<&'static mut Optimizer> {
    unsafe { OPTIMIZER.as_mut() }
}

#[macro_export]
macro_rules! dirty_rect {
    ($x:expr, $y:expr, $w:expr, $h:expr) => {
        if let Some(opt) = $crate::optimizer::get_optimizer() {
            opt.add_dirty_rect($x as usize, $y as usize, $w as usize, $h as usize);
        }
    };
}
