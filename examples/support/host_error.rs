//! Fixed-size errors for the small host demonstrations.

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Build(coaptic::BuildError),
    Coap(coaptic::app::Error<std::io::Error>),
    Buffer(coaptic::app::ResponseBufferError),
    Call(coaptic::app::CallFailure),
    #[cfg(feature = "oscore")]
    Oscore(coaptic::oscore::Error),
    Entropy,
    Clock,
    UnexpectedReply,
    Timeout,
    Utf8(core::str::Utf8Error),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "UDP: {error}"),
            Self::Build(error) => write!(f, "configuration: {error}"),
            Self::Coap(error) => write!(f, "CoAP: {error}"),
            Self::Buffer(error) => write!(f, "response buffer: {error:?}"),
            Self::Call(error) => write!(f, "exchange: {error:?}"),
            #[cfg(feature = "oscore")]
            Self::Oscore(error) => write!(f, "OSCORE: {error:?}"),
            Self::Entropy => f.write_str("OS entropy unavailable"),
            Self::Clock => f.write_str("millisecond clock overflow"),
            Self::UnexpectedReply => f.write_str("unexpected response"),
            Self::Timeout => f.write_str("exchange timed out"),
            Self::Utf8(error) => write!(f, "response text: {error}"),
        }
    }
}
impl std::error::Error for Error {}

macro_rules! convert {
    ($source:ty, $variant:ident) => {
        impl From<$source> for Error {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}
convert!(std::io::Error, Io);
convert!(coaptic::BuildError, Build);
convert!(coaptic::app::Error<std::io::Error>, Coap);
convert!(coaptic::app::ResponseBufferError, Buffer);
convert!(coaptic::app::CallFailure, Call);
#[cfg(feature = "oscore")]
convert!(coaptic::oscore::Error, Oscore);
convert!(core::str::Utf8Error, Utf8);
