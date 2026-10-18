//! Caller-owned input for desktop and embedded commissioning consoles.
#![no_std]

pub mod workflow;

/// Explicit operator approval; PIV touch presence is distinct from WebAuthn UV.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ApprovalMode {
    #[default]
    WebAuthn,
    PivTouch,
    PivTouchless,
}

impl ApprovalMode {
    pub fn from_flag(flag: &str) -> Option<Self> {
        match flag {
            "--piv-touch" => Some(Self::PivTouch),
            "--development-no-touch" => Some(Self::PivTouchless),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::WebAuthn => "WebAuthn user verification",
            Self::PivTouch => "PIV hardware touch required",
            Self::PivTouchless => "development PIV touchless",
        }
    }
}

/// A complete line remains available until `clear`; oversized input is drained.
pub struct Line<const N: usize> {
    bytes: [u8; N],
    len: usize,
    overflow: bool,
    complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputError {
    TooLong,
    InvalidUtf8,
    Incomplete,
}

impl<const N: usize> Default for Line<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Line<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
            overflow: false,
            complete: false,
        }
    }

    /// Returns true on newline. Does not consume another line until cleared.
    pub fn push(&mut self, byte: u8) -> bool {
        if self.complete {
            return true;
        }
        if byte == b'\n' {
            self.complete = true;
            return true;
        }
        if self.len < N {
            self.bytes[self.len] = byte;
            self.len += 1;
        } else {
            self.overflow = true;
        }
        false
    }

    /// Finish a partial line at transport EOF. False means no input was received.
    pub fn finish(&mut self) -> bool {
        self.complete = true;
        self.len != 0 || self.overflow
    }

    pub fn text(&self) -> Result<&str, InputError> {
        if !self.complete {
            return Err(InputError::Incomplete);
        }
        if self.overflow {
            return Err(InputError::TooLong);
        }
        core::str::from_utf8(&self.bytes[..self.len])
            .map(str::trim)
            .map_err(|_| InputError::InvalidUtf8)
    }

    pub fn clear(&mut self) {
        self.bytes.fill(0);
        self.len = 0;
        self.overflow = false;
        self.complete = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_overflow_until_newline_and_recovers() {
        let mut line = Line::<3>::new();
        for byte in b"abcd" {
            assert!(!line.push(*byte));
        }
        assert!(line.push(b'\n'));
        assert_eq!(line.text(), Err(InputError::TooLong));
        line.clear();
        for byte in b"ok\n" {
            line.push(*byte);
        }
        assert_eq!(line.text(), Ok("ok"));
    }
    #[test]
    fn rejects_invalid_utf8_and_handles_eof() {
        let mut line = Line::<2>::new();
        assert!(!line.finish());
        line.clear();
        line.push(255);
        assert!(line.finish());
        assert_eq!(line.text(), Err(InputError::InvalidUtf8));
    }
}
