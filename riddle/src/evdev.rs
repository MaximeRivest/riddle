//! Kernel `struct input_event` wire layout, which follows the userspace ABI:
//! on 64-bit the leading timeval is 16 bytes (record = 24 bytes), on 32-bit
//! ARM (reMarkable 1/2) it is 8 bytes (record = 16 bytes). Parsing with the
//! wrong size silently misreads every event, so all readers go through here.

/// Size of one input_event record as read(2) from an evdev fd.
#[cfg(target_pointer_width = "64")]
pub const EV_SIZE: usize = 24;
#[cfg(target_pointer_width = "32")]
pub const EV_SIZE: usize = 16;

#[cfg(target_pointer_width = "64")]
const TYPE_OFF: usize = 16;
#[cfg(target_pointer_width = "32")]
const TYPE_OFF: usize = 8;

/// Decode (type, code, value) from one EV_SIZE-byte record.
pub fn decode(chunk: &[u8]) -> (u16, u16, i32) {
    let etype = u16::from_le_bytes(chunk[TYPE_OFF..TYPE_OFF + 2].try_into().unwrap());
    let code = u16::from_le_bytes(chunk[TYPE_OFF + 2..TYPE_OFF + 4].try_into().unwrap());
    let value = i32::from_le_bytes(chunk[TYPE_OFF + 4..TYPE_OFF + 8].try_into().unwrap());
    (etype, code, value)
}
