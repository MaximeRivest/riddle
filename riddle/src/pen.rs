//! Raw evdev pen input: the full digitizer, bypassing Qt's filtered view.
//! Gives us native pressure, hover, and the eraser tip (BTN_TOOL_RUBBER),
//! at the hardware event rate.
//!
//! The device is grabbed (EVIOCGRAB) while the diary is open so xochitl
//! doesn't also react to the pen; released automatically on close/exit.

use std::io;
use std::os::fd::RawFd;

use crate::fb::{SCREEN_H, SCREEN_W};

pub const MAX_PRESSURE: i32 = 4096;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_PRESSURE: u16 = 24;
const BTN_TOOL_PEN: u16 = 320;
const BTN_TOOL_RUBBER: u16 = 321;
const BTN_TOUCH: u16 = 330;

const EVIOCGRAB: libc::c_ulong = 0x40044590;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Eraser,
}

#[derive(Debug, Clone, Copy)]
pub struct PenSample {
    /// Screen coordinates.
    pub x: i32,
    pub y: i32,
    /// 0..4096
    pub pressure: i32,
    pub tool: Tool,
    pub touching: bool,
}

pub struct PenDevice {
    fd: RawFd,
    // Accumulated state between SYN_REPORTs.
    raw_x: i32,
    raw_y: i32,
    pressure: i32,
    tool: Tool,
    touching: bool,
    dirty: bool,
    max_x: i32,
    max_y: i32,
    event_size: usize,
    grabbed: bool,
}

impl PenDevice {
    /// Find and open the marker input device. `shared` (companion mode, where
    /// xochitl must keep seeing the pen) skips the exclusive grab; it is the
    /// caller's display mode, not an env var, so a stray RIDDLE_XOCHITL in
    /// oracle.env cannot silently un-grab the pen in qtfb mode.
    pub fn open(shared: bool) -> io::Result<Self> {
        let path = find_marker_device()?;
        let cpath = std::ffi::CString::new(path.clone()).unwrap();
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let grab = if shared { 0 } else { unsafe { libc::ioctl(fd, EVIOCGRAB, 1i32) } };
        if grab != 0 {
            eprintln!("riddle: warning: EVIOCGRAB failed ({}) — xochitl will also see the pen", io::Error::last_os_error());
        }
        eprintln!("riddle: pen device {path} opened (shared with xochitl: {shared})");
        let max_x = abs_max(fd, ABS_X).unwrap_or(20967);
        let max_y = abs_max(fd, ABS_Y).unwrap_or(15725);
        Ok(Self {
            fd,
            raw_x: 0,
            raw_y: 0,
            pressure: 0,
            tool: Tool::Pen,
            touching: false,
            dirty: false,
            max_x,
            max_y,
            event_size: std::mem::size_of::<libc::timeval>() + 8,
            grabbed: !shared && grab == 0,
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.fd
    }

    /// Drain all pending events; returns one sample per SYN_REPORT frame
    /// that changed state.
    pub fn drain(&mut self) -> Vec<PenSample> {
        let mut out = Vec::new();
        let mut buf = [0u8; 24 * 64];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            let off = self.event_size - 8;
            for chunk in buf[..n as usize].chunks_exact(self.event_size) {
                let etype = u16::from_ne_bytes(chunk[off..off + 2].try_into().unwrap());
                let code = u16::from_ne_bytes(chunk[off + 2..off + 4].try_into().unwrap());
                let value = i32::from_ne_bytes(chunk[off + 4..off + 8].try_into().unwrap());
                match (etype, code) {
                    (EV_ABS, ABS_X) => {
                        self.raw_x = value;
                        self.dirty = true;
                    }
                    (EV_ABS, ABS_Y) => {
                        self.raw_y = value;
                        self.dirty = true;
                    }
                    (EV_ABS, ABS_PRESSURE) => {
                        self.pressure = value;
                        self.dirty = true;
                    }
                    (EV_KEY, BTN_TOOL_PEN) if value == 1 => {
                        self.tool = Tool::Pen;
                    }
                    (EV_KEY, BTN_TOOL_RUBBER) => {
                        self.tool = if value == 1 { Tool::Eraser } else { Tool::Pen };
                    }
                    (EV_KEY, BTN_TOUCH) => {
                        self.touching = value == 1;
                        self.dirty = true;
                    }
                    (EV_SYN, SYN_REPORT) => {
                        if self.dirty {
                            self.dirty = false;
                            out.push(PenSample {
                                // RM2's Wacom axes are landscape relative to the
                                // portrait panel: rotate and invert into screen space.
                                x: self.raw_y * (SCREEN_W as i32 - 1) / self.max_y,
                                y: (self.max_x - self.raw_x) * (SCREEN_H as i32 - 1) / self.max_x,
                                pressure: self.pressure,
                                tool: self.tool,
                                touching: self.touching,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }
}

impl Drop for PenDevice {
    fn drop(&mut self) {
        unsafe {
            if self.grabbed { libc::ioctl(self.fd, EVIOCGRAB, 0i32); }
            libc::close(self.fd);
        }
    }
}

/// Inject generated strokes into xochitl through the RM2 Wacom device.  The
/// stock app receives them exactly like real Marker events and records them in
/// the currently open notebook.
pub struct PenWriter {
    fd: RawFd,
    max_x: i32,
    max_y: i32,
    /// Last injected screen position: samples read back from the shared
    /// device near this point are our own loopback, anything else is the
    /// user's real pen.
    last: std::cell::Cell<(i32, i32)>,
}

impl PenWriter {
    pub fn open() -> io::Result<Self> {
        let path = find_marker_device()?;
        let cpath = std::ffi::CString::new(path).unwrap();
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        Ok(Self { fd, max_x: 20966, max_y: 15725, last: std::cell::Cell::new((i32::MIN / 2, i32::MIN / 2)) })
    }

    /// True if (x, y) is within `r` px of the last injected point.
    pub fn near(&self, x: i32, y: i32, r: i32) -> bool {
        let (lx, ly) = self.last.get();
        (x - lx).abs() <= r && (y - ly).abs() <= r
    }

    fn send(&self, events: &[(u16, u16, i32)]) -> io::Result<()> {
        #[repr(C)]
        struct Event { time: libc::timeval, kind: u16, code: u16, value: i32 }
        for &(kind, code, value) in events {
            let ev = Event { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind, code, value };
            let n = unsafe { libc::write(self.fd, &ev as *const _ as *const libc::c_void, std::mem::size_of::<Event>()) };
            if n < 0 { return Err(io::Error::last_os_error()); }
        }
        Ok(())
    }

    fn raw(&self, x: i32, y: i32) -> (i32, i32) {
        let rx = (crate::fb::SCREEN_H as i32 - 1 - y).clamp(0, crate::fb::SCREEN_H as i32 - 1)
            * self.max_x / (crate::fb::SCREEN_H as i32 - 1);
        let ry = x.clamp(0, crate::fb::SCREEN_W as i32 - 1)
            * self.max_y / (crate::fb::SCREEN_W as i32 - 1);
        (rx, ry)
    }

    pub fn down_at(&self, x: i32, y: i32) -> io::Result<()> {
        self.last.set((x, y));
        let (x, y) = self.raw(x, y);
        self.send(&[(EV_ABS, ABS_X, x), (EV_ABS, ABS_Y, y), (EV_KEY, BTN_TOOL_PEN, 1),
            (EV_KEY, BTN_TOUCH, 0), (EV_ABS, ABS_PRESSURE, 0), (EV_SYN, SYN_REPORT, 0),
            (EV_KEY, BTN_TOUCH, 1), (EV_ABS, ABS_PRESSURE, 1800), (EV_SYN, SYN_REPORT, 0)])
    }

    pub fn goto(&self, x: i32, y: i32) -> io::Result<()> {
        self.last.set((x, y));
        let (x, y) = self.raw(x, y);
        self.send(&[(EV_ABS, ABS_X, x), (EV_ABS, ABS_Y, y), (EV_SYN, SYN_REPORT, 0)])
    }

    pub fn up(&self) -> io::Result<()> {
        self.send(&[(EV_ABS, ABS_PRESSURE, 0), (EV_KEY, BTN_TOUCH, 0),
            (EV_KEY, BTN_TOOL_PEN, 0), (EV_SYN, SYN_REPORT, 0)])
    }
}

impl Drop for PenWriter { fn drop(&mut self) { let _ = self.up(); unsafe { libc::close(self.fd); } } }

fn find_marker_device() -> io::Result<String> {
    for i in 0..32 {
        let name_path = format!("/sys/class/input/event{i}/device/name");
        if let Ok(name) = std::fs::read_to_string(&name_path) {
            let name = name.to_lowercase();
            if name.contains("marker") || name.contains("wacom") {
                return Ok(format!("/dev/input/event{i}"));
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "no marker input device found"))
}

#[repr(C)]
#[derive(Default)]
struct InputAbsInfo { value: i32, minimum: i32, maximum: i32, fuzz: i32, flat: i32, resolution: i32 }

fn abs_max(fd: RawFd, axis: u16) -> Option<i32> {
    // _IOR('E', 0x40 + axis, struct input_absinfo), whose size is 24 bytes.
    let request = 0x8018_4500u64 | (0x40 + axis as u64);
    let mut info = InputAbsInfo::default();
    let rc = unsafe { libc::ioctl(fd, request as libc::c_ulong, &mut info) };
    (rc == 0 && info.maximum > 0).then_some(info.maximum)
}
