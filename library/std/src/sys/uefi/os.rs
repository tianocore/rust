use super::unsupported;
use crate::error::Error as StdError;
use crate::ffi::{OsStr, OsString};
use crate::fmt;
use crate::io;
use crate::marker::PhantomData;
use crate::os::uefi;
use crate::path::{self, PathBuf};

pub fn errno() -> i32 {
    uefi::raw::Status::ABORTED.as_usize() as i32
}

pub fn error_string(_errno: i32) -> String {
    "ABORTED".to_string()
}

pub fn getcwd() -> io::Result<PathBuf> {
    unsupported()
}

pub fn chdir(_: &path::Path) -> io::Result<()> {
    unsupported()
}

pub struct SplitPaths<'a>(!, PhantomData<&'a ()>);

pub fn split_paths(_unparsed: &OsStr) -> SplitPaths<'_> {
    panic!("unsupported")
}

impl<'a> Iterator for SplitPaths<'a> {
    type Item = PathBuf;
    fn next(&mut self) -> Option<PathBuf> {
        self.0
    }
}

#[derive(Debug)]
pub struct JoinPathsError;

pub fn join_paths<I, T>(_paths: I) -> Result<OsString, JoinPathsError>
where
    I: Iterator<Item = T>,
    T: AsRef<OsStr>,
{
    Err(JoinPathsError)
}

impl fmt::Display for JoinPathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "not supported on this platform yet".fmt(f)
    }
}

impl StdError for JoinPathsError {
    #[allow(deprecated)]
    fn description(&self) -> &str {
        "not supported on this platform yet"
    }
}

pub fn current_exe() -> io::Result<PathBuf> {
    unsupported()
}

// FIXME: Implement using Variable Services
pub struct Env(!);

impl Iterator for Env {
    type Item = (OsString, OsString);
    fn next(&mut self) -> Option<(OsString, OsString)> {
        self.0
    }
}

pub fn env() -> Env {
    panic!("not supported on this platform")
}

// FIXME: Use GetVariable() method
pub fn getenv(_: &OsStr) -> Option<OsString> {
    None
}

// FIXME: Use SetVariable() method
pub fn setenv(_: &OsStr, _: &OsStr) -> io::Result<()> {
    Err(io::const_io_error!(io::ErrorKind::Unsupported, "cannot set env vars on this platform"))
}

pub fn unsetenv(_: &OsStr) -> io::Result<()> {
    Err(io::const_io_error!(io::ErrorKind::Unsupported, "cannot unset env vars on this platform"))
}

pub fn temp_dir() -> PathBuf {
    panic!("no filesystem on this platform")
}

pub fn home_dir() -> Option<PathBuf> {
    None
}

pub fn exit(code: i32) -> ! {
    let code = match usize::try_from(code) {
        Ok(x) => uefi::raw::Status::from_usize(x),
        Err(_) => uefi::raw::Status::ABORTED,
    };

    if let (Some(boot_services), Some(handle)) =
        (uefi::env::get_boot_services(), uefi::env::get_system_handle())
    {
        let _ =
            unsafe { ((*boot_services.as_ptr()).exit)(handle.as_ptr(), code, 0, [0].as_mut_ptr()) };
    }

    crate::intrinsics::abort()
}

pub fn getpid() -> u32 {
    panic!("no pids on this platform")
}
