//! Low level `ESC { ... }` frame encoding used by the TEPRA SR3700P protocol.
//!
//! Every "command frame" observed in the captured print jobs has the form:
//!
//! ```text
//! 0x1B 0x7B <len> <cmd> <params...> <checksum> 0x7D
//! ```
//!
//! - `len` is a single byte whose meaning differs between frame kinds. For
//!   the small configuration frames (`C`, `D`, `G`, `L`, `T`, `@`) it equals
//!   the number of bytes that follow it up to and including the trailing
//!   `0x7D` (i.e. `cmd.len() + params.len() + 1(checksum) + 1(closing)`).
//! - `checksum` is the sum of `cmd` and `params` bytes, truncated to `u8`.
//!
//! The large job-header block uses a different, fixed `len` marker (`0x80`)
//! that does not correspond to the payload length; it is handled separately
//! in [`crate::tepra::header`].

/// Build a small command frame: `ESC { <len> <cmd> <params...> <checksum> }`.
///
/// `cmd` is a single command byte/character (e.g. `b'L'`), `params` are the
/// command's parameter bytes. The checksum is `sum(cmd, params) mod 256`.
pub fn build_command_frame(cmd: u8, params: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(1 + params.len());
    body.push(cmd);
    body.extend_from_slice(params);
    let checksum = checksum_of(&body);

    // len = cmd(1) + params.len() + checksum(1) + closing(1)
    let len = (body.len() + 2) as u8;

    let mut frame = Vec::with_capacity(4 + body.len() + 1);
    frame.push(0x1B);
    frame.push(b'{');
    frame.push(len);
    frame.extend_from_slice(&body);
    frame.push(checksum);
    frame.push(b'}');
    frame
}

/// Sum of all bytes modulo 256, used as the checksum for every `ESC { }`
/// frame observed in the protocol.
pub fn checksum_of(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, &b| acc.wrapping_add(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_l_command_from_a_prn() {
        // ESC { 07 4C AA 00 00 00 F6 7D  (L command, 170 rows, little endian u32)
        let frame = build_command_frame(b'L', &170u32.to_le_bytes());
        assert_eq!(frame, vec![0x1B, b'{', 0x07, b'L', 0xAA, 0x00, 0x00, 0x00, 0xF6, b'}']);
    }

    #[test]
    fn matches_t_command_from_a_prn() {
        // ESC { 05 54 24 00 78 7D (T command, margin=36, little endian u16)
        let frame = build_command_frame(b'T', &36u16.to_le_bytes());
        assert_eq!(frame, vec![0x1B, b'{', 0x05, b'T', 0x24, 0x00, 0x78, b'}']);
    }

    #[test]
    fn matches_end_command() {
        // ESC { 03 40 40 7D (end command, cmd='@' 0x40, no params)
        let frame = build_command_frame(b'@', &[]);
        assert_eq!(frame, vec![0x1B, b'{', 0x03, 0x40, 0x40, b'}']);
    }
}
