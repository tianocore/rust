use crate::ffi::OsStr;
use crate::fmt;
use crate::io;
use crate::marker::PhantomData;
use crate::num::NonZeroI32;
use crate::os::uefi;
use crate::path::Path;
use crate::sys::fs::File;
use crate::sys::pipe::AnonPipe;
use crate::sys::unsupported;
use crate::sys_common::process::{CommandEnv, CommandEnvs};

pub use crate::ffi::OsString as EnvKey;

////////////////////////////////////////////////////////////////////////////////
// Command
////////////////////////////////////////////////////////////////////////////////

pub struct Command {
    env: CommandEnv,
    program: crate::ffi::OsString,
    args: crate::ffi::OsString,
    stdout_key: Option<crate::ffi::OsString>,
    stderr_key: Option<crate::ffi::OsString>,
    stdin_key: Option<crate::ffi::OsString>,
}
// passed back to std::process with the pipes connected to the child, if any were requested
#[derive(Default)]
pub struct StdioPipes {
    pub stdin: Option<AnonPipe>,
    pub stdout: Option<AnonPipe>,
    pub stderr: Option<AnonPipe>,
}

pub enum Stdio {
    Inherit,
    Null,
    MakePipe,
}

impl Command {
    pub fn new(program: &OsStr) -> Command {
        Command {
            env: Default::default(),
            program: program.to_os_string(),
            args: program.to_os_string(),
            stdout_key: None,
            stderr_key: None,
            stdin_key: None,
        }
    }

    pub fn arg(&mut self, arg: &OsStr) {
        self.args.push(" ");
        self.args.push(arg);
    }

    pub fn env_mut(&mut self) -> &mut CommandEnv {
        &mut self.env
    }

    pub fn cwd(&mut self, _dir: &OsStr) {}

    pub fn stdin(&mut self, stdin: Stdio) {
        match stdin {
            Stdio::Inherit => {}
            Stdio::Null => {
                let mut key = self.program.clone();
                key.push("_stdin");
                self.env.set(&key, OsStr::new("null"));
            }
            Stdio::MakePipe => {
                todo!()
            }
        }
    }

    pub fn stdout(&mut self, stdout: Stdio) {
        match stdout {
            Stdio::Inherit => {}
            Stdio::Null => {
                let mut key = self.program.clone();
                key.push("_stdout");
                self.env.set(&key, OsStr::new("null"));
            }
            Stdio::MakePipe => {
                let mut key = self.program.clone();
                key.push("_stdout");
                let mut val = self.program.clone();
                val.push("_stdout_pipe");
                self.env.set(&key, &val);
                self.stdout_key = Some(val);
            }
        }
    }

    pub fn stderr(&mut self, stderr: Stdio) {
        match stderr {
            Stdio::Inherit => {}
            Stdio::Null => {
                let mut key = self.program.clone();
                key.push("_stderr");
                self.env.set(&key, OsStr::new("null"));
            }
            Stdio::MakePipe => {
                let mut key = self.program.clone();
                key.push("_stderr");
                let mut val = self.program.clone();
                val.push("_stderr_pipe");
                self.env.set(&key, &val);
                self.stderr_key = Some(val);
            }
        }
    }

    pub fn get_program(&self) -> &OsStr {
        self.program.as_os_str()
    }

    pub fn get_args(&self) -> CommandArgs<'_> {
        CommandArgs { _p: PhantomData }
    }

    pub fn get_envs(&self) -> CommandEnvs<'_> {
        self.env.iter()
    }

    pub fn get_current_dir(&self) -> Option<&Path> {
        None
    }

    pub fn spawn(
        &mut self,
        default: Stdio,
        _needs_stdin: bool,
    ) -> io::Result<(Process, StdioPipes)> {
        let cmd = uefi_command::Command::load_image(self.program.as_os_str())?;
        cmd.set_args(self.args.as_os_str())?;

        // Set env varibles
        for (key, val) in self.env.iter() {
            match val {
                None => crate::env::remove_var(key),
                Some(x) => crate::env::set_var(key, x),
            }
        }

        let mut stdio_pipe = StdioPipes::default();

        if let Some(x) = &self.stdout_key {
            stdio_pipe.stdout = Some(AnonPipe::new(x));
        }
        if let Some(x) = &self.stderr_key {
            stdio_pipe.stderr = Some(AnonPipe::new(x));
        }
        // Initially thought to implement start at wait. However, it seems like everything expectes
        // stdio output to be ready for reading before calling wait, which is not possible at least
        // in current implementation.
        let r = cmd.start_image()?;
        let proc = Process { status: r, env: self.env.clone() };

        Ok((proc, stdio_pipe))
    }
}

impl From<AnonPipe> for Stdio {
    fn from(pipe: AnonPipe) -> Stdio {
        pipe.diverge()
    }
}

impl From<File> for Stdio {
    fn from(_file: File) -> Stdio {
        panic!("unsupported")
    }
}

impl fmt::Debug for Command {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

#[derive(Copy, PartialEq, Eq, Clone)]
pub struct ExitStatus(uefi::raw::Status);

impl ExitStatus {
    pub fn exit_ok(&self) -> Result<(), ExitStatusError> {
        if self.0.is_error() { Err(ExitStatusError(*self)) } else { Ok(()) }
    }

    pub fn code(&self) -> Option<i32> {
        Some(self.0.as_usize() as i32)
    }
}

impl fmt::Debug for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&super::common::status_to_io_error(&self.0), f)
    }
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&super::common::status_to_io_error(&self.0), f)
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitStatusError(ExitStatus);

impl Into<ExitStatus> for ExitStatusError {
    fn into(self) -> ExitStatus {
        self.0
    }
}

impl ExitStatusError {
    pub fn code(self) -> Option<NonZeroI32> {
        NonZeroI32::new(self.0.0.as_usize() as i32)
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct ExitCode(bool);

impl ExitCode {
    pub const SUCCESS: ExitCode = ExitCode(false);
    pub const FAILURE: ExitCode = ExitCode(true);

    pub fn as_i32(&self) -> i32 {
        self.0 as i32
    }
}

impl From<u8> for ExitCode {
    fn from(code: u8) -> Self {
        match code {
            0 => Self::SUCCESS,
            1..=255 => Self::FAILURE,
        }
    }
}

pub struct Process {
    status: uefi::raw::Status,
    env: CommandEnv,
}

impl Process {
    pub fn id(&self) -> u32 {
        unimplemented!()
    }

    pub fn kill(&mut self) -> io::Result<()> {
        unsupported()
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        Ok(ExitStatus(self.status))
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        unsupported()
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Clenup env
        for (k, _) in self.env.iter() {
            let _ = super::os::unsetenv(k);
        }
    }
}

pub struct CommandArgs<'a> {
    _p: PhantomData<&'a ()>,
}

impl<'a> Iterator for CommandArgs<'a> {
    type Item = &'a OsStr;
    fn next(&mut self) -> Option<&'a OsStr> {
        None
    }
}

impl<'a> ExactSizeIterator for CommandArgs<'a> {}

impl<'a> fmt::Debug for CommandArgs<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().finish()
    }
}

mod uefi_command {
    use crate::ffi::OsStr;
    use crate::io;
    use crate::mem::{ManuallyDrop, MaybeUninit};
    use crate::os::uefi;
    use crate::os::uefi::ffi::OsStrExt;
    use crate::os::uefi::raw::protocols::loaded_image;
    use crate::ptr::NonNull;

    pub struct Command {
        inner: NonNull<crate::ffi::c_void>,
    }

    impl Command {
        pub fn load_image(p: &OsStr) -> io::Result<Self> {
            let boot_services = uefi::env::get_boot_services().ok_or(io::Error::new(
                io::ErrorKind::Uncategorized,
                "Failed to acquire boot_services",
            ))?;
            let system_handle = uefi::env::get_system_handle().ok_or(io::Error::new(
                io::ErrorKind::Uncategorized,
                "Failed to acquire System Handle",
            ))?;
            let path = uefi::path::DevicePath::try_from(p)?;
            let mut child_handle: MaybeUninit<uefi::raw::Handle> = MaybeUninit::uninit();
            let r = unsafe {
                ((*boot_services.as_ptr()).load_image)(
                    uefi::raw::Boolean::FALSE,
                    system_handle.as_ptr(),
                    path.as_ptr(),
                    crate::ptr::null_mut(),
                    0,
                    child_handle.as_mut_ptr(),
                )
            };
            if r.is_error() {
                Err(super::super::common::status_to_io_error(&r))
            } else {
                let child_handle = unsafe { child_handle.assume_init() };
                match NonNull::new(child_handle) {
                    None => Err(io::Error::new(io::ErrorKind::InvalidData, "Null Handle Received")),
                    Some(x) => Ok(Self { inner: x }),
                }
            }
        }

        pub fn start_image(&self) -> io::Result<uefi::raw::Status> {
            let boot_services = uefi::env::get_boot_services().ok_or(io::Error::new(
                io::ErrorKind::Uncategorized,
                "Failed to acquire boot_services",
            ))?;
            let mut exit_data_size: MaybeUninit<usize> = MaybeUninit::uninit();
            let mut exit_data: MaybeUninit<*mut u16> = MaybeUninit::uninit();
            let r = unsafe {
                ((*boot_services.as_ptr()).start_image)(
                    self.inner.as_ptr(),
                    exit_data_size.as_mut_ptr(),
                    exit_data.as_mut_ptr(),
                )
            };

            // Drop exitdata
            unsafe {
                exit_data_size.assume_init_drop();
                exit_data.assume_init_drop();
            }

            Ok(r)
        }

        pub fn set_args(&self, args: &OsStr) -> io::Result<()> {
            let protocol: NonNull<loaded_image::Protocol> =
                uefi::env::get_handle_protocol(self.inner, &mut loaded_image::PROTOCOL_GUID)
                    .ok_or(io::Error::new(
                        io::ErrorKind::Uncategorized,
                        "Failed to acquire loaded image protocol for child handle",
                    ))?;
            let mut args = ManuallyDrop::new(args.to_ffi_string());
            let args_size = (crate::mem::size_of::<u16>() * args.len()) as u32;
            unsafe {
                (*protocol.as_ptr()).load_options_size = args_size;
                crate::mem::replace(
                    &mut (*protocol.as_ptr()).load_options,
                    args.as_mut_ptr() as *mut crate::ffi::c_void,
                );
            };
            Ok(())
        }

        pub fn change_stdout(
            &self,
            stdout_protocol: &mut super::uefi_stdio_pip::StdOutProtocol,
        ) -> io::Result<()> {
            let protocol: NonNull<loaded_image::Protocol> =
                uefi::env::get_handle_protocol(self.inner, &mut loaded_image::PROTOCOL_GUID)
                    .ok_or(io::Error::new(
                        io::ErrorKind::Uncategorized,
                        "Failed to acquire loaded image protocol for child handle",
                    ))?;
            unsafe {
                crate::mem::swap(
                    &mut (*(*protocol.as_ptr()).system_table).con_out,
                    &mut (stdout_protocol.get_protocol()
                        as *mut uefi::raw::protocols::simple_text_output::Protocol),
                );
                crate::mem::swap(
                    &mut (*(*protocol.as_ptr()).system_table).console_out_handle,
                    &mut stdout_protocol.get_handle_raw(),
                );
            }
            Ok(())
        }
    }

    impl Drop for Command {
        // Unload Image
        fn drop(&mut self) {
            if let Some(boot_services) = uefi::env::get_boot_services() {
                let _ = unsafe { ((*boot_services.as_ptr()).unload_image)(self.inner.as_ptr()) };
            }
        }
    }
}

mod uefi_stdio_pip {
    use crate::io;
    use crate::os::uefi;
    use crate::os::uefi::raw::protocols::simple_text_output;
    use crate::ptr::NonNull;

    pub struct ProtocolHandler<T> {
        handle: Option<NonNull<crate::ffi::c_void>>,
        guid: uefi::raw::Guid,
        protocol: T,
    }

    impl<T> ProtocolHandler<T> {
        pub fn new(
            handle: Option<NonNull<crate::ffi::c_void>>,
            guid: uefi::raw::Guid,
            protocol: T,
        ) -> Self {
            Self { handle, guid, protocol }
        }

        // Panics if protocol not installed yet
        pub unsafe fn get_handle_raw(&self) -> *mut crate::ffi::c_void {
            self.handle.unwrap().as_ptr()
        }

        pub fn get_protocol(&mut self) -> &mut T {
            &mut self.protocol
        }

        pub fn install_protocol(&mut self) -> io::Result<()> {
            let boot_services = uefi::env::get_boot_services().ok_or(io::Error::new(
                io::ErrorKind::Uncategorized,
                "Failed to acquire boot services",
            ))?;

            let mut new_handle: uefi::raw::Handle = match self.handle {
                Some(x) => x.as_ptr(),
                None => crate::ptr::null_mut(),
            };
            let r = unsafe {
                ((*boot_services.as_ptr()).install_multiple_protocol_interfaces)(
                    &mut new_handle,
                    (&mut self.guid) as *mut _ as *mut crate::ffi::c_void,
                    (&mut self.protocol) as *mut _ as *mut crate::ffi::c_void,
                )
            };

            if r.is_error() {
                Err(super::super::common::status_to_io_error(&r))
            } else {
                self.handle = match NonNull::new(new_handle) {
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::Uncategorized,
                            "Null Handle returned",
                        ));
                    }
                    Some(x) => Some(x),
                };
                Ok(())
            }
        }
    }

    impl<T> Drop for ProtocolHandler<T> {
        fn drop(&mut self) {
            if let (Some(handle), Some(boot_services)) =
                (self.handle, uefi::env::get_boot_services())
            {
                let _ = unsafe {
                    ((*boot_services.as_ptr()).uninstall_multiple_protocol_interfaces)(
                        &mut handle.as_ptr(),
                        (&mut self.guid) as *mut _ as *mut crate::ffi::c_void,
                        (&mut self.protocol) as *mut _ as *mut crate::ffi::c_void,
                    )
                };
            }
        }
    }

    extern "efiapi" fn null_stdio_1(
        _: *mut simple_text_output::Protocol,
        _: uefi::raw::Boolean,
    ) -> uefi::raw::Status {
        uefi::raw::Status::SUCCESS
    }

    extern "efiapi" fn null_stdio_2(
        _: *mut simple_text_output::Protocol,
        _: *mut uefi::raw::Char16,
    ) -> uefi::raw::Status {
        uefi::raw::Status::SUCCESS
    }

    extern "efiapi" fn null_stdio_3(
        _: *mut simple_text_output::Protocol,
        _: *mut uefi::raw::Char16,
    ) -> uefi::raw::Status {
        uefi::raw::Status::SUCCESS
    }

    extern "efiapi" fn null_stdio_4(
        _: *mut simple_text_output::Protocol,
        _: usize,
        _: *mut usize,
        _: *mut usize,
    ) -> uefi::raw::Status {
        uefi::raw::Status::SUCCESS
    }

    extern "efiapi" fn null_stdio_5(
        _: *mut simple_text_output::Protocol,
        _: usize,
    ) -> uefi::raw::Status {
        uefi::raw::Status::SUCCESS
    }

    extern "efiapi" fn null_stdio_6(_: *mut simple_text_output::Protocol) -> uefi::raw::Status {
        uefi::raw::Status::SUCCESS
    }

    extern "efiapi" fn null_stdio_7(
        _: *mut simple_text_output::Protocol,
        _: usize,
        _: usize,
    ) -> uefi::raw::Status {
        uefi::raw::Status::SUCCESS
    }

    pub fn get_null_stdio() -> ProtocolHandler<simple_text_output::Protocol> {
        let protocol = simple_text_output::Protocol {
            reset: null_stdio_1,
            output_string: null_stdio_2,
            test_string: null_stdio_3,
            query_mode: null_stdio_4,
            set_mode: null_stdio_5,
            set_attribute: null_stdio_5,
            clear_screen: null_stdio_6,
            set_cursor_position: null_stdio_7,
            enable_cursor: null_stdio_1,
            mode: crate::ptr::null_mut(),
        };
        ProtocolHandler::new(None, simple_text_output::PROTOCOL_GUID, protocol)
    }

    pub type StdOutProtocol = ProtocolHandler<simple_text_output::Protocol>;
}
