//! Platform-specific extensions to `std` for UEFI platforms.
//!
//! Provides access to platform-level information on Unix platforms, and
//! exposes Unix-specific functions that would otherwise be inappropriate as
//! part of the core `std` library.
//!
//! It exposes more ways to deal with platform-specific strings ([`OsStr`],
//! [`OsString`]), allows to set permissions more granularly, extract low-level
//! file descriptors from files and sockets, and has platform-specific helpers
//! for spawning processes.
//!
//! [`OsStr`]: crate::ffi::OsStr
//! [`OsString`]: crate::ffi::OsString

#![deny(unsafe_op_in_unsafe_fn)]
pub mod alloc;
pub mod args;
#[path = "../unix/cmath.rs"]
pub mod cmath;
pub mod env;
pub mod fs;
#[path = "../unsupported/io.rs"]
pub mod io;
#[path = "../unsupported/locks/mod.rs"]
pub mod locks;
pub mod net;
pub mod os;
pub mod os_str;
pub mod path;
pub mod pipe;
pub mod process;
pub mod stdio;
pub mod thread;
#[path = "../unsupported/thread_local_key.rs"]
pub mod thread_local_key;
pub mod time;

pub(crate) mod common;

#[cfg(test)]
mod tests;

use crate::io as std_io;
use crate::os::uefi;
use crate::ptr::NonNull;

pub mod memchr {
    pub use core::slice::memchr::{memchr, memrchr};
}

pub fn init(_argc: isize, _argv: *const *const u8) {}

// SAFETY: must be called only once during runtime cleanup.
// NOTE: this is not guaranteed to run, for example when the program aborts.
pub unsafe fn cleanup() {}

pub fn unsupported<T>() -> std_io::Result<T> {
    Err(unsupported_err())
}

pub fn unsupported_err() -> std_io::Error {
    std_io::const_io_error!(
        std_io::ErrorKind::Unsupported,
        "operation not supported on this platform",
    )
}

pub fn decode_error_kind(code: i32) -> crate::io::ErrorKind {
    use crate::io::ErrorKind;
    use r_efi::efi::Status;

    if let Ok(code) = usize::try_from(code) {
        common::status_to_io_error(Status::from_usize(code)).kind()
    } else {
        ErrorKind::Uncategorized
    }
}

pub fn abort_internal() -> ! {
    // First try to use EFI_BOOT_SERVICES.Exit()
    if let (Some(boot_services), Some(handle)) =
        (uefi::env::get_boot_services(), uefi::env::get_system_handle())
    {
        let _ = unsafe {
            ((*boot_services.as_ptr()).exit)(
                handle.as_ptr(),
                r_efi::efi::Status::ABORTED,
                0,
                [0].as_mut_ptr(),
            )
        };
    }

    // In case SystemTable and SystemHandle cannot be reached, do things the Windows way
    #[allow(unused)]
    const FAST_FAIL_FATAL_APP_EXIT: usize = 7;
    unsafe {
        cfg_if::cfg_if! {
            if #[cfg(any(target_arch = "x86", target_arch = "x86_64"))] {
                core::arch::asm!("int $$0x29", in("ecx") FAST_FAIL_FATAL_APP_EXIT);
            } else if #[cfg(all(target_arch = "arm", target_feature = "thumb-mode"))] {
                core::arch::asm!(".inst 0xDEFB", in("r0") FAST_FAIL_FATAL_APP_EXIT);
            } else if #[cfg(target_arch = "aarch64")] {
                core::arch::asm!("brk 0xF003", in("x0") FAST_FAIL_FATAL_APP_EXIT);
            } else {
                core::intrinsics::abort();
            }
        }
        core::intrinsics::unreachable();
    }
}

// This function is needed by the panic runtime. The symbol is named in
// pre-link args for the target specification, so keep that in sync.
#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn __rust_abort() {
    abort_internal();
}

pub fn hashmap_random_keys() -> (u64, u64) {
    unsafe { (get_random().unwrap_or(1), get_random().unwrap_or(2)) }
}

unsafe fn get_random() -> Option<u64> {
    use r_efi::protocols::rng;

    let mut buf = [0u8; 8];
    let handles = common::locate_handles(rng::PROTOCOL_GUID).ok()?;
    for handle in handles {
        if let Ok(protocol) = common::open_protocol::<rng::Protocol>(handle, rng::PROTOCOL_GUID) {
            let r = unsafe {
                ((*protocol.as_ptr()).get_rng)(
                    protocol.as_ptr(),
                    crate::ptr::null_mut(),
                    buf.len(),
                    buf.as_mut_ptr(),
                )
            };
            if r.is_error() {
                continue;
            } else {
                return Some(u64::from_le_bytes(buf));
            }
        }
    }
    None
}

extern "C" {
    fn main(argc: isize, argv: *const *const u8) -> isize;
}

// FIXME: Do not generate this in case of `no_main`
#[no_mangle]
pub unsafe extern "efiapi" fn efi_main(
    handle: r_efi::efi::Handle,
    st: *mut r_efi::efi::SystemTable,
) -> r_efi::efi::Status {
    if let (Some(system_table), Some(system_handle)) = (NonNull::new(st), NonNull::new(handle)) {
        uefi::env::init_globals(system_handle, system_table);

        match unsafe { main(0, crate::ptr::null()) } {
            0 => r_efi::efi::Status::SUCCESS,
            _ => r_efi::efi::Status::ABORTED, // Or some other status code
        }
    } else {
        r_efi::efi::Status::ABORTED
    }
}
