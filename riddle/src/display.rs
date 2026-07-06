//! Display backends: qtfb (windowed, inside xochitl) and quill (takeover,
//! vendor engine, xochitl stopped). Selected at runtime: if QTFB_KEY is set
//! we're an AppLoad app; otherwise we assume takeover.

use crate::fb::{SCREEN_H, SCREEN_W};
use crate::surface::{PixFmt, Surface};
use std::io;

// Both devices' native qtfb formats are RGB565; only geometry differs.
#[cfg(not(feature = "rm2"))]
const QTFB_FORMAT: u8 = crate::qtfb::FBFMT_RMPP_RGB565;
#[cfg(feature = "rm2")]
const QTFB_FORMAT: u8 = crate::qtfb::FBFMT_RM2FB;

pub enum Display {
    Qtfb(crate::qtfb::QtfbClient),
    #[allow(dead_code)]
    Quill,
}

// C ABI from libquill.so (linked when built with --features takeover).
#[cfg(feature = "takeover")]
mod quill_ffi {
    extern "C" {
        pub fn quill_init() -> i32;
        pub fn quill_width() -> i32;
        pub fn quill_height() -> i32;
        pub fn quill_stride() -> i32;
        pub fn quill_buffer() -> *mut u8;
        pub fn quill_swap(x: i32, y: i32, w: i32, h: i32, mode: i32, full: i32) -> u64;
        pub fn quill_process_events();
    }
}

impl Display {
    pub fn open() -> io::Result<(Self, Surface)> {
        if let Ok(key) = std::env::var("QTFB_KEY") {
            let key: i32 = key.parse().map_err(io::Error::other)?;
            let mut client = crate::qtfb::QtfbClient::connect(
                key,
                QTFB_FORMAT,
                SCREEN_W,
                SCREEN_H,
                2,
            )?;
            let _ = client.set_refresh_mode(crate::qtfb::REFRESH_MODE_UFAST);
            let buf = client.framebuffer();
            let (ptr, len) = (buf.as_mut_ptr(), buf.len());
            let surface = Surface::new(ptr, len, SCREEN_W, SCREEN_H, SCREEN_W * 2, PixFmt::Rgb565);
            return Ok((Display::Qtfb(client), surface));
        }

        #[cfg(feature = "takeover")]
        {
            unsafe {
                if quill_ffi::quill_init() != 0 {
                    return Err(io::Error::other("quill_init failed"));
                }
                let w = quill_ffi::quill_width() as usize;
                let h = quill_ffi::quill_height() as usize;
                let stride = quill_ffi::quill_stride() as usize;
                let ptr = quill_ffi::quill_buffer();
                if ptr.is_null() {
                    return Err(io::Error::other("quill buffer null"));
                }
                let surface = Surface::new(ptr, stride * h, w, h, stride, PixFmt::Rgb32);
                Ok((Display::Quill, surface))
            }
        }
        #[cfg(not(feature = "takeover"))]
        Err(io::Error::other(
            "QTFB_KEY not set and this build has no takeover backend",
        ))
    }

    /// Push a region to the panel. `fast` selects the low-latency waveform.
    pub fn update(&self, x: i32, y: i32, w: i32, h: i32, _fast: bool) {
        match self {
            Display::Qtfb(c) => {
                let _ = c.update_partial(x, y, w, h);
            }
            #[allow(unused_variables)]
            Display::Quill => {
                #[cfg(feature = "takeover")]
                unsafe {
                    // mode 0 = fastest (ink), 3 = balanced (text/anim)
                    quill_ffi::quill_swap(x, y, w, h, if _fast { 0 } else { 3 }, 0);
                    quill_ffi::quill_process_events();
                }
            }
        }
    }

    pub fn update_all(&self, w: usize, h: usize) {
        match self {
            Display::Qtfb(c) => {
                let _ = c.update_all();
            }
            #[allow(unused_variables)]
            Display::Quill => {
                #[cfg(feature = "takeover")]
                unsafe {
                    quill_ffi::quill_swap(0, 0, w as i32, h as i32, 3, 0);
                    quill_ffi::quill_process_events();
                }
            }
        }
        let _ = (w, h);
    }

    /// Flashing clear of the whole panel (ghost removal).
    pub fn full_refresh(&self, w: usize, h: usize) {
        match self {
            Display::Qtfb(c) => {
                let _ = c.request_full_refresh();
            }
            #[allow(unused_variables)]
            Display::Quill => {
                #[cfg(feature = "takeover")]
                unsafe {
                    quill_ffi::quill_swap(0, 0, w as i32, h as i32, 4, 1);
                    quill_ffi::quill_process_events();
                }
            }
        }
        let _ = (w, h);
    }

    /// Drain window-system events. For qtfb this also detects window close
    /// (returns Err); the takeover backend has no window to lose.
    pub fn pump(&self) -> io::Result<Vec<crate::qtfb::InputEvent>> {
        match self {
            Display::Qtfb(c) => c.drain_events(),
            Display::Quill => {
                #[cfg(feature = "takeover")]
                unsafe {
                    quill_ffi::quill_process_events();
                }
                Ok(Vec::new())
            }
        }
    }

    pub fn terminate(&self) {
        if let Display::Qtfb(c) = self {
            c.terminate();
        }
    }
}
