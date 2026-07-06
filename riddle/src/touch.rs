//! Raw touch input for takeover mode: 5-finger tap = quit gesture.

use std::io;
use std::os::fd::RawFd;

use crate::evdev;

const EV_ABS: u16 = 3;
const ABS_MT_SLOT: u16 = 47;
const ABS_MT_TRACKING_ID: u16 = 57;
const EVIOCGRAB: libc::c_ulong = 0x40044590;
const MAX_SLOTS: usize = 16;

pub struct TouchDevice {
    fd: RawFd,
    slots: [bool; MAX_SLOTS],
    cur: usize,
}

impl TouchDevice {
    pub fn open() -> io::Result<Self> {
        for i in 0..8 {
            let name_path = format!("/sys/class/input/event{i}/device/name");
            if let Ok(name) = std::fs::read_to_string(&name_path) {
                let name = name.to_lowercase();
                // "touch" on the Paper Pro; "pt_mt"/"cyttsp5_mt" on rM2/rM1.
                if name.contains("touch") || name.contains("pt_mt") || name.contains("cyttsp5") {
                    let path = std::ffi::CString::new(format!("/dev/input/event{i}")).unwrap();
                    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
                    if fd < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    unsafe { libc::ioctl(fd, EVIOCGRAB, 1i32) };
                    return Ok(Self { fd, slots: [false; MAX_SLOTS], cur: 0 });
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, "no touch device"))
    }

    /// Returns true if a 5-finger touch was seen.
    pub fn drain_check_quit(&mut self) -> bool {
        let mut quit = false;
        let mut buf = [0u8; evdev::EV_SIZE * 64];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for chunk in buf[..n as usize].chunks_exact(evdev::EV_SIZE) {
                let (etype, code, value) = evdev::decode(chunk);
                if etype == EV_ABS && code == ABS_MT_SLOT {
                    self.cur = (value.max(0) as usize).min(MAX_SLOTS - 1);
                } else if etype == EV_ABS && code == ABS_MT_TRACKING_ID {
                    self.slots[self.cur] = value != -1;
                    if self.slots.iter().filter(|&&s| s).count() >= 5 {
                        quit = true;
                    }
                }
            }
        }
        quit
    }
}

impl Drop for TouchDevice {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, EVIOCGRAB, 0i32);
            libc::close(self.fd);
        }
    }
}
