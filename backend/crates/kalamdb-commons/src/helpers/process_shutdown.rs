//! Cooperative process stop for managed KalamDB processes.
//!
//! Unix `kalam down` sends SIGTERM. Windows has no equivalent for a detached
//! console-less process, so the server (and `kalam dev`) wait on a named
//! kernel event. The waiter parks in the kernel; this is not on the request
//! path and does not use Tokio's blocking pool.

/// Session-local event name used as the Windows equivalent of SIGTERM.
pub fn shutdown_event_name(pid: u32) -> String {
    format!(r"Local\kalamdb-shutdown-{pid}")
}

#[cfg(windows)]
mod windows_event {
    use std::{
        ffi::OsStr,
        io,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
        },
        ptr,
    };

    use super::shutdown_event_name;

    type BOOL = i32;
    type DWORD = u32;
    type HANDLE = *mut core::ffi::c_void;
    type LPCWSTR = *const u16;

    const FALSE: BOOL = 0;
    const TRUE: BOOL = 1;
    const INFINITE: DWORD = 0xFFFF_FFFF;
    const WAIT_OBJECT_0: DWORD = 0;
    const EVENT_MODIFY_STATE: DWORD = 0x0002;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateEventW(
            lp_event_attributes: *const core::ffi::c_void,
            b_manual_reset: BOOL,
            b_initial_state: BOOL,
            lp_name: LPCWSTR,
        ) -> HANDLE;
        fn OpenEventW(dw_desired_access: DWORD, b_inherit_handle: BOOL, lp_name: LPCWSTR)
            -> HANDLE;
        fn SetEvent(h_event: HANDLE) -> BOOL;
        fn WaitForSingleObject(h_handle: HANDLE, dw_milliseconds: DWORD) -> DWORD;
    }

    /// Manual-reset named event used to request a graceful process stop.
    pub struct ShutdownEvent {
        handle: OwnedHandle,
    }

    impl ShutdownEvent {
        /// Create (or open) this process's shutdown event.
        ///
        /// # Errors
        ///
        /// Returns an error if the Windows event cannot be created.
        pub fn create_for_current_process() -> io::Result<Self> {
            Self::create_named(&shutdown_event_name(std::process::id()))
        }

        fn create_named(name: &str) -> io::Result<Self> {
            let wide = wide_null(name);
            // SAFETY: `wide` is a NUL-terminated UTF-16 name; a null security
            // descriptor requests the default DACL. CreateEventW returns null
            // or an owned handle that this process must close.
            let handle = unsafe { CreateEventW(ptr::null(), TRUE, FALSE, wide.as_ptr()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `handle` is a valid owned event handle from CreateEventW.
            Ok(Self {
                handle: unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) },
            })
        }

        fn open_named(name: &str) -> io::Result<Self> {
            let wide = wide_null(name);
            // SAFETY: `wide` is a NUL-terminated UTF-16 name. OpenEventW returns
            // null or an owned handle that this process must close.
            let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, FALSE, wide.as_ptr()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `handle` is a valid owned event handle from OpenEventW.
            Ok(Self {
                handle: unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) },
            })
        }

        /// Signal a running process to shut down cooperatively.
        ///
        /// # Errors
        ///
        /// Returns an error if the event does not exist or cannot be signaled.
        pub fn signal_pid(pid: u32) -> io::Result<()> {
            Self::open_named(&shutdown_event_name(pid))?.set()
        }

        fn set(&self) -> io::Result<()> {
            // SAFETY: `handle` is a live event opened or created by this
            // process; SetEvent requires EVENT_MODIFY_STATE, which we hold.
            let ok = unsafe { SetEvent(self.handle.as_raw_handle() as HANDLE) };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        /// Park until the event is signaled. Does not spin.
        ///
        /// # Errors
        ///
        /// Returns an error if the wait fails.
        pub fn wait(&self) -> io::Result<()> {
            // SAFETY: `handle` remains open for the duration of this call.
            let status =
                unsafe { WaitForSingleObject(self.handle.as_raw_handle() as HANDLE, INFINITE) };
            if status == WAIT_OBJECT_0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
    }

    fn wide_null(name: &str) -> Vec<u16> {
        OsStr::new(name).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Returns true when the target process has created its shutdown event and
    /// the event was signaled.
    pub fn signal_shutdown_event(pid: u32) -> bool {
        ShutdownEvent::signal_pid(pid).is_ok()
    }
}

#[cfg(windows)]
pub use windows_event::{signal_shutdown_event, ShutdownEvent};

#[cfg(test)]
mod tests {
    use super::shutdown_event_name;

    #[test]
    fn shutdown_event_name_is_session_local_and_pid_specific() {
        assert_eq!(shutdown_event_name(1), r"Local\kalamdb-shutdown-1");
        assert_eq!(shutdown_event_name(4242), r"Local\kalamdb-shutdown-4242");
        assert_ne!(shutdown_event_name(1), shutdown_event_name(2));
    }

    #[cfg(windows)]
    #[test]
    fn shutdown_event_wakes_a_parked_waiter() {
        use std::{
            thread,
            time::{Duration, Instant},
        };

        use super::ShutdownEvent;

        let event = ShutdownEvent::create_for_current_process()
            .expect("current process shutdown event should be created");
        let waiter = thread::spawn(move || event.wait());

        let pid = std::process::id();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if super::signal_shutdown_event(pid) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        waiter
            .join()
            .expect("waiter thread should finish")
            .expect("wait should succeed");
    }
}
