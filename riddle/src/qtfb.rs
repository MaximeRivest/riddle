//! Native qtfb client: SOCK_SEQPACKET protocol + shared-memory framebuffer.
//!
//! Wire format (verified against rm-appload src/qtfb/common.h):
//!   ClientMessage  = 24 bytes, type:u8 @0, payload @4
//!   ServerMessage layout is ABI-dependent — aarch64 pads the union to @8,
//!   armv7 places init/userInput fields immediately after the type byte @4.

use std::io;
use std::os::fd::RawFd;

pub const MESSAGE_INITIALIZE: u8 = 0;
pub const MESSAGE_UPDATE: u8 = 1;
#[allow(dead_code)]
pub const MESSAGE_CUSTOM_INITIALIZE: u8 = 2;
pub const MESSAGE_TERMINATE: u8 = 3;
pub const MESSAGE_USERINPUT: u8 = 4;
pub const MESSAGE_SET_REFRESH_MODE: u8 = 5;
pub const MESSAGE_REQUEST_FULL_REFRESH: u8 = 6;

pub const UPDATE_ALL: i32 = 0;
pub const UPDATE_PARTIAL: i32 = 1;

/// FBFMT_RM2FB: native 1404x1872 on rM1/rM2, 2 bytes/pixel RGB565.
pub const FBFMT_RM2FB: u8 = 0;
/// FBFMT_RMPP_RGB565: native 1620x2160 on Paper Pro, 2 bytes/pixel.
pub const FBFMT_RMPP_RGB565: u8 = 3;

#[allow(dead_code)]
pub const REFRESH_MODE_UFAST: i32 = 0;
pub const REFRESH_MODE_FAST: i32 = 1;

// Input event types (server -> client).
pub const INPUT_TOUCH_PRESS: i32 = 0x10;
pub const INPUT_TOUCH_RELEASE: i32 = 0x11;
pub const INPUT_TOUCH_UPDATE: i32 = 0x12;
pub const INPUT_PEN_PRESS: i32 = 0x20;
pub const INPUT_PEN_RELEASE: i32 = 0x21;
#[allow(dead_code)]
pub const INPUT_PEN_UPDATE: i32 = 0x22;
pub const INPUT_VKB_PRESS: i32 = 0x40;
#[allow(dead_code)]
pub const INPUT_VKB_RELEASE: i32 = 0x41;

const SOCKET_PATH: &str = "/tmp/qtfb.sock";

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub input_type: i32,
    pub dev_id: i32,
    pub x: i32,
    pub y: i32,
    #[allow(dead_code)]
    pub d: i32,
}

pub struct QtfbClient {
    fd: RawFd,
    shm: *mut u8,
    shm_len: usize,
    pub width: usize,
    pub height: usize,
    /// bytes per pixel
    pub bpp: usize,
}

// The raw pointer is to a MAP_SHARED region; we are the only writer thread.
unsafe impl Send for QtfbClient {}

impl QtfbClient {
    /// Connect and initialize with the default resolution of `format`.
    pub fn connect(key: i32, format: u8, width: usize, height: usize, bpp: usize) -> io::Result<Self> {
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (i, b) in SOCKET_PATH.bytes().enumerate() {
            addr.sun_path[i] = b as libc::c_char;
        }
        let rc = unsafe {
            libc::connect(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }

        // MESSAGE_INITIALIZE: key i32 @4, format u8 @8.
        let mut msg = [0u8; 24];
        msg[0] = MESSAGE_INITIALIZE;
        msg[4..8].copy_from_slice(&key.to_le_bytes());
        msg[8] = format;
        send_all(fd, &msg)?;

        // Init reply. Server closing without replying (recv == 0) means rejected.
        let mut reply = [0u8; 32];
        let n = unsafe { libc::recv(fd, reply.as_mut_ptr() as *mut libc::c_void, 32, 0) };
        if n <= 0 {
            unsafe { libc::close(fd) };
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "qtfb server rejected init (no reply)",
            ));
        }
        #[cfg(target_pointer_width = "32")]
        let min_reply = 12;
        #[cfg(target_pointer_width = "64")]
        let min_reply = 24;
        if (n as usize) < min_reply {
            unsafe { libc::close(fd) };
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("qtfb init reply too short: {n} < {min_reply}"),
            ));
        }
        let (shm_key, shm_size) = parse_init_reply(&reply)?;

        let shm_path = format!("/dev/shm/qtfb_{}\0", shm_key);
        let shm_fd = unsafe { libc::open(shm_path.as_ptr() as *const libc::c_char, libc::O_RDWR) };
        if shm_fd < 0 {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                shm_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                shm_fd,
                0,
            )
        };
        unsafe { libc::close(shm_fd) };
        if ptr == libc::MAP_FAILED {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }

        if shm_size < width * height * bpp {
            unsafe { libc::close(fd) };
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("shm too small: {} < {}", shm_size, width * height * bpp),
            ));
        }

        // Non-blocking: the event loop drains input events opportunistically.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        Ok(Self {
            fd,
            shm: ptr as *mut u8,
            shm_len: shm_size,
            width,
            height,
            bpp,
        })
    }

    pub fn raw_fd(&self) -> RawFd {
        self.fd
    }

    pub fn framebuffer(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.shm, self.shm_len) }
    }

    fn send_msg(&self, msg: &[u8; 24]) -> io::Result<()> {
        send_all(self.fd, msg)
    }

    pub fn update_all(&self) -> io::Result<()> {
        let mut msg = [0u8; 24];
        msg[0] = MESSAGE_UPDATE;
        msg[4..8].copy_from_slice(&UPDATE_ALL.to_le_bytes());
        self.send_msg(&msg)
    }

    pub fn update_partial(&self, x: i32, y: i32, w: i32, h: i32) -> io::Result<()> {
        let mut msg = [0u8; 24];
        msg[0] = MESSAGE_UPDATE;
        msg[4..8].copy_from_slice(&UPDATE_PARTIAL.to_le_bytes());
        msg[8..12].copy_from_slice(&x.to_le_bytes());
        msg[12..16].copy_from_slice(&y.to_le_bytes());
        msg[16..20].copy_from_slice(&w.to_le_bytes());
        msg[20..24].copy_from_slice(&h.to_le_bytes());
        self.send_msg(&msg)
    }

    /// NOTE: the server sleeps its handler thread for 1s after this — call rarely.
    pub fn set_refresh_mode(&self, mode: i32) -> io::Result<()> {
        let mut msg = [0u8; 24];
        msg[0] = MESSAGE_SET_REFRESH_MODE;
        msg[4..8].copy_from_slice(&mode.to_le_bytes());
        self.send_msg(&msg)
    }

    /// NOTE: 1s server-side stall, use only on explicit user request.
    pub fn request_full_refresh(&self) -> io::Result<()> {
        let mut msg = [0u8; 24];
        msg[0] = MESSAGE_REQUEST_FULL_REFRESH;
        self.send_msg(&msg)
    }

    pub fn terminate(&self) {
        let mut msg = [0u8; 24];
        msg[0] = MESSAGE_TERMINATE;
        let _ = self.send_msg(&msg);
    }

    /// Drain pending server messages. Returns input events, or Err on
    /// disconnect (window closed -> we must exit).
    pub fn drain_events(&self) -> io::Result<Vec<InputEvent>> {
        let mut out = Vec::new();
        loop {
            let mut buf = [0u8; 32];
            let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, 32, 0) };
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "qtfb socket closed",
                ));
            }
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock {
                    return Ok(out);
                }
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if let Some(evt) = parse_userinput(&buf[..n as usize]) {
                out.push(evt);
            }
        }
    }
}

impl Drop for QtfbClient {
    fn drop(&mut self) {
        self.terminate();
        unsafe {
            libc::munmap(self.shm as *mut libc::c_void, self.shm_len);
            libc::close(self.fd);
        }
    }
}

fn parse_init_reply(reply: &[u8; 32]) -> io::Result<(i32, usize)> {
    // AppLoad's ServerMessage union is padded differently on 32- vs 64-bit ABIs.
    #[cfg(target_pointer_width = "32")]
    {
        let shm_key = i32::from_le_bytes(reply[4..8].try_into().unwrap());
        let shm_size = u32::from_le_bytes(reply[8..12].try_into().unwrap()) as usize;
        Ok((shm_key, shm_size))
    }
    #[cfg(target_pointer_width = "64")]
    {
        let shm_key = i32::from_le_bytes(reply[8..12].try_into().unwrap());
        let shm_size = u64::from_le_bytes(reply[16..24].try_into().unwrap()) as usize;
        Ok((shm_key, shm_size))
    }
}

fn parse_userinput(buf: &[u8]) -> Option<InputEvent> {
    if buf.first().copied()? != MESSAGE_USERINPUT {
        return None;
    }
    #[cfg(target_pointer_width = "32")]
    {
        if buf.len() < 24 {
            return None;
        }
        Some(InputEvent {
            input_type: i32::from_le_bytes(buf[4..8].try_into().ok()?),
            dev_id: i32::from_le_bytes(buf[8..12].try_into().ok()?),
            x: i32::from_le_bytes(buf[12..16].try_into().ok()?),
            y: i32::from_le_bytes(buf[16..20].try_into().ok()?),
            d: i32::from_le_bytes(buf[20..24].try_into().ok()?),
        })
    }
    #[cfg(target_pointer_width = "64")]
    {
        if buf.len() < 28 {
            return None;
        }
        Some(InputEvent {
            input_type: i32::from_le_bytes(buf[8..12].try_into().ok()?),
            dev_id: i32::from_le_bytes(buf[12..16].try_into().ok()?),
            x: i32::from_le_bytes(buf[16..20].try_into().ok()?),
            y: i32::from_le_bytes(buf[20..24].try_into().ok()?),
            d: i32::from_le_bytes(buf[24..28].try_into().ok()?),
        })
    }
}

fn send_all(fd: RawFd, buf: &[u8]) -> io::Result<()> {
    loop {
        let n = unsafe { libc::send(fd, buf.as_ptr() as *const libc::c_void, buf.len(), 0) };
        if n == buf.len() as isize {
            return Ok(());
        }
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            // Non-blocking socket: retry sends briefly rather than dropping a
            // protocol message (updates are small and the server drains fast).
            if e.kind() == io::ErrorKind::WouldBlock {
                std::thread::sleep(std::time::Duration::from_millis(2));
                continue;
            }
            return Err(e);
        }
        return Err(io::Error::new(io::ErrorKind::WriteZero, "short send"));
    }
}
