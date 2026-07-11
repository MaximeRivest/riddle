//! Raw evdev pen input: the full digitizer, bypassing Qt's filtered view.
//! Gives us 0-4096 pressure, tilt, hover, and the eraser tip (BTN_TOOL_RUBBER),
//! at the hardware event rate.
//!
//! The device is grabbed (EVIOCGRAB) while the diary is open so xochitl
//! doesn't also react to the pen; released automatically on close/exit.

use std::io;
use std::os::fd::RawFd;

pub const MAX_PRESSURE: i32 = 4096;

// EVIOCGABS ioctl: read absolute axis info (struct input_absinfo).
const EVIOCGABS_X: libc::c_ulong = 0x80184540; // _IOR('E', 0x40 + ABS_X, struct input_absinfo)
const EVIOCGABS_Y: libc::c_ulong = 0x80184541; // _IOR('E', 0x40 + ABS_Y, struct input_absinfo)

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
    sw: i32,
    sh: i32,
    digi_max_x: i32,
    digi_max_y: i32,
    // Accumulated state between SYN_REPORTs.
    raw_x: i32,
    raw_y: i32,
    pressure: i32,
    tool: Tool,
    touching: bool,
    dirty: bool,
}

impl PenDevice {
    /// Find and grab the marker input device.
    pub fn open(sw: usize, sh: usize) -> io::Result<Self> {
        let path = find_marker_device()?;
        let cpath = std::ffi::CString::new(path.clone()).unwrap();
        let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // Read actual digitizer axis ranges — they differ between devices
        // (e.g. Paper Pro: 11180×15340, Paper Pro Move: 6760×11960).
        let digi_max_x = read_abs_max(fd, EVIOCGABS_X).unwrap_or(11180);
        let digi_max_y = read_abs_max(fd, EVIOCGABS_Y).unwrap_or(15340);

        let grab = unsafe { libc::ioctl(fd, EVIOCGRAB, 1i32) };
        if grab != 0 {
            eprintln!("riddle: warning: EVIOCGRAB failed ({}) — xochitl will also see the pen", io::Error::last_os_error());
        }
        eprintln!("riddle: pen device {path} opened (grabbed: {}, digi {}x{})",
            grab == 0, digi_max_x, digi_max_y);
        Ok(Self {
            fd,
            sw: sw as i32,
            sh: sh as i32,
            digi_max_x,
            digi_max_y,
            raw_x: 0,
            raw_y: 0,
            pressure: 0,
            tool: Tool::Pen,
            touching: false,
            dirty: false,
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.fd
    }

    /// Drain all pending events; returns one sample per SYN_REPORT frame
    /// that changed state.
    pub fn drain(&mut self) -> Vec<PenSample> {
        let mut out = Vec::new();
        // input_event on 64-bit: struct timeval (16) + type u16 + code u16 + value i32.
        let mut buf = [0u8; 24 * 64];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for chunk in buf[..n as usize].chunks_exact(24) {
                let etype = u16::from_le_bytes(chunk[16..18].try_into().unwrap());
                let code = u16::from_le_bytes(chunk[18..20].try_into().unwrap());
                let value = i32::from_le_bytes(chunk[20..24].try_into().unwrap());
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
                                x: self.raw_x * (self.sw - 1) / self.digi_max_x,
                                y: self.raw_y * (self.sh - 1) / self.digi_max_y,
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
            libc::ioctl(self.fd, EVIOCGRAB, 0i32);
            libc::close(self.fd);
        }
    }
}

/// Read the maximum value of an ABS axis via EVIOCGABS ioctl.
/// struct input_absinfo: value(i32), minimum(i32), maximum(i32), fuzz(i32), flat(i32), resolution(i32)
fn read_abs_max(fd: RawFd, ioctl_code: libc::c_ulong) -> Option<i32> {
    let mut buf = [0i32; 6];
    let ret = unsafe { libc::ioctl(fd, ioctl_code, buf.as_mut_ptr()) };
    if ret == 0 && buf[2] > 0 { Some(buf[2]) } else { None }
}

fn find_marker_device() -> io::Result<String> {
    for i in 0..8 {
        let name_path = format!("/sys/class/input/event{i}/device/name");
        if let Ok(name) = std::fs::read_to_string(&name_path) {
            if name.to_lowercase().contains("marker") {
                return Ok(format!("/dev/input/event{i}"));
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "no marker input device found"))
}
