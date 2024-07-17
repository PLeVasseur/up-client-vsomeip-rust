#[cfg(feature = "timing")]
mod timing_imports {
    pub use std::sync::Arc;
    pub use std::sync::Mutex;
    pub use std::time::Duration;
    pub use std::time::Instant;
}

use crate::{AuthorityName, ClientId, RequestId, SessionId, UeId};
use std::sync::RwLock as StdRwLock;
#[cfg(feature = "timing")]
use timing_imports::*;
use up_rust::UUri;

/// A wrapper around [tokio::sync::RwLock] which can, if configured, keep track of lock wait times
///
/// To configure the ability to time the lock waits, use the `timing` feature flag
pub struct TimedStdRwLock<T> {
    inner: StdRwLock<T>,
    #[cfg(feature = "timing")]
    read_durations: Arc<Mutex<Vec<Duration>>>,
    #[cfg(feature = "timing")]
    write_durations: Arc<Mutex<Vec<Duration>>>,
}

impl<T> TimedStdRwLock<T> {
    /// Wraps a `T` in a TimedAsyncRwLock
    pub fn new(value: T) -> Self {
        Self {
            inner: StdRwLock::new(value),
            #[cfg(feature = "timing")]
            read_durations: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "timing")]
            write_durations: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Functions like and is a wrapper around [tokio::sync::RwLock::read]
    ///
    /// If the `timing` feature is enabled will store the lock wait time of this call
    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, T> {
        #[cfg(feature = "timing")]
        let start = Instant::now();

        let guard = self.inner.read().unwrap();

        #[cfg(feature = "timing")]
        {
            let duration = start.elapsed();
            let mut read_durations = self.read_durations.lock().unwrap();
            read_durations.push(duration);
        }

        guard
    }

    /// Functions like and is a wrapper around [tokio::sync::RwLock::write]
    ///
    /// If the `timing` feature is enabled will store the lock wait time of this call
    pub fn write(&self) -> std::sync::RwLockWriteGuard<'_, T> {
        #[cfg(feature = "timing")]
        let start = Instant::now();

        let guard = self.inner.write().unwrap();

        #[cfg(feature = "timing")]
        {
            let duration = start.elapsed();
            let mut write_durations = self.write_durations.lock().unwrap();
            write_durations.push(duration);
        }

        guard
    }

    #[cfg(feature = "timing")]
    /// Reads the current durations stored of all calls to `read()`
    pub fn read_durations(&self) -> Vec<Duration> {
        let read_durations = self.read_durations.lock().unwrap();
        read_durations.clone()
    }

    #[cfg(feature = "timing")]
    /// Reads the current durations stored of all calls to `write()`
    pub fn write_durations(&self) -> Vec<Duration> {
        let write_durations = self.write_durations.lock().unwrap();
        write_durations.clone()
    }
}

// TODO: use function from up-rust when merged
/// Creates a completely wildcarded [UUri]
pub fn any_uuri() -> UUri {
    UUri {
        authority_name: "*".to_string(),
        ue_id: 0x0000_FFFF,     // any instance, any service
        ue_version_major: 0xFF, // any
        resource_id: 0xFFFF,    // any
        ..Default::default()
    }
}

/// Creates a [UUri] with specified [UUri::authority_name] and [UUri::ue_id]
pub fn any_uuri_fixed_authority_id(authority_name: &AuthorityName, ue_id: UeId) -> UUri {
    UUri {
        authority_name: authority_name.to_string(),
        ue_id: ue_id as u32,
        ue_version_major: 0xFF, // any
        resource_id: 0xFFFF,    // any
        ..Default::default()
    }
}

/// Useful for splitting u32 into u16s when manipulating [UUri] elements
pub fn split_u32_to_u16(value: u32) -> (u16, u16) {
    let most_significant_bits = (value >> 16) as u16;
    let least_significant_bits = (value & 0xFFFF) as u16;
    (most_significant_bits, least_significant_bits)
}

/// Useful for splitting u32 into u8s when manipulating [UUri] elements
pub fn split_u32_to_u8(value: u32) -> (u8, u8, u8, u8) {
    let byte1 = (value >> 24) as u8;
    let byte2 = (value >> 16 & 0xFF) as u8;
    let byte3 = (value >> 8 & 0xFF) as u8;
    let byte4 = (value & 0xFF) as u8;
    (byte1, byte2, byte3, byte4)
}

/// Create a vsomeip request_id from client_id and session_id as per SOME/IP spec
pub fn create_request_id(client_id: ClientId, session_id: SessionId) -> RequestId {
    ((client_id as u32) << 16) | (session_id as u32)
}
