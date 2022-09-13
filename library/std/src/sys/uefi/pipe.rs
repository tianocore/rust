//! An implementation of Pipes for UEFI

use super::common;
use crate::io::{self, IoSlice, IoSliceMut};
use crate::ptr::NonNull;

pub struct AnonPipe {
    _pipe_data: Option<Box<uefi_pipe_protocol::Pipedata>>,
    _protocol: Option<common::ProtocolWrapper<uefi_pipe_protocol::Protocol>>,
    handle: NonNull<crate::ffi::c_void>,
}

unsafe impl Send for AnonPipe {}

// Safety: There are no threads in UEFI
unsafe impl Sync for AnonPipe {}

impl AnonPipe {
    pub(crate) fn new(
        pipe_data: Option<Box<uefi_pipe_protocol::Pipedata>>,
        protocol: Option<common::ProtocolWrapper<uefi_pipe_protocol::Protocol>>,
        handle: NonNull<crate::ffi::c_void>,
    ) -> Self {
        Self { _pipe_data: pipe_data, _protocol: protocol, handle }
    }

    pub(crate) fn null() -> Self {
        let pipe = common::ProtocolWrapper::install_protocol(uefi_pipe_protocol::Protocol::null())
            .unwrap();
        let handle = pipe.handle();
        Self::new(None, Some(pipe), handle)
    }

    pub(crate) fn make_pipe() -> Self {
        const MIN_BUFFER: usize = 1024;
        let mut pipe_data = Box::new(uefi_pipe_protocol::Pipedata::with_capacity(MIN_BUFFER));
        let pipe = common::ProtocolWrapper::install_protocol(
            uefi_pipe_protocol::Protocol::with_data(&mut pipe_data),
        )
        .unwrap();
        let handle = pipe.handle();
        Self::new(Some(pipe_data), Some(pipe), handle)
    }

    pub(crate) fn handle(&self) -> NonNull<crate::ffi::c_void> {
        self.handle
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let protocol = common::open_protocol::<uefi_pipe_protocol::Protocol>(
            self.handle,
            uefi_pipe_protocol::PROTOCOL_GUID,
        )?;
        let mut buf_size = buf.len();
        let r = unsafe {
            ((*protocol.as_ptr()).read)(protocol.as_ptr(), &mut buf_size, buf.as_mut_ptr())
        };
        if r.is_error() { Err(common::status_to_io_error(r)) } else { Ok(buf_size) }
    }

    pub(crate) fn read_to_end(&self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let protocol = common::open_protocol::<uefi_pipe_protocol::Protocol>(
            self.handle,
            uefi_pipe_protocol::PROTOCOL_GUID,
        )?;
        let buf_size = unsafe { ((*protocol.as_ptr()).size)(protocol.as_ptr()) };
        buf.reserve_exact(buf_size);
        let mut buf_size = buf.capacity();
        let r = unsafe {
            ((*protocol.as_ptr()).read)(protocol.as_ptr(), &mut buf_size, buf.as_mut_ptr())
        };
        if r.is_error() {
            Err(common::status_to_io_error(r))
        } else {
            unsafe {
                buf.set_len(buf.len() + buf_size);
            }
            Ok(buf_size)
        }
    }

    pub fn read_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        crate::io::default_read_vectored(|buf| self.read(buf), bufs)
    }

    #[inline]
    pub fn is_read_vectored(&self) -> bool {
        false
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let protocol = common::open_protocol::<uefi_pipe_protocol::Protocol>(
            self.handle,
            uefi_pipe_protocol::PROTOCOL_GUID,
        )?;
        let mut buf_size = buf.len();
        let r =
            unsafe { ((*protocol.as_ptr()).write)(protocol.as_ptr(), &mut buf_size, buf.as_ptr()) };
        if r.is_error() { Err(common::status_to_io_error(r)) } else { Ok(buf_size) }
    }

    pub fn write_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        crate::io::default_write_vectored(|buf| self.write(buf), bufs)
    }

    #[inline]
    pub fn is_write_vectored(&self) -> bool {
        false
    }

    pub fn diverge(&self) -> ! {
        unimplemented!()
    }
}

pub fn read2(p1: AnonPipe, v1: &mut Vec<u8>, p2: AnonPipe, v2: &mut Vec<u8>) -> io::Result<()> {
    p1.read_to_end(v1)?;
    p2.read_to_end(v2)?;
    Ok(())
}

pub(crate) mod uefi_pipe_protocol {
    use crate::collections::VecDeque;
    use crate::io;
    use crate::sys::uefi::common;
    use io::{Read, Write};
    use r_efi::efi::Guid;
    use r_efi::{eficall, eficall_abi};

    pub(crate) const PROTOCOL_GUID: Guid = Guid::from_fields(
        0x3c4acb49,
        0xfb4c,
        0x45fb,
        0x93,
        0xe4,
        &[0x63, 0x5d, 0x71, 0x48, 0x4c, 0x0f],
    );

    // Maybe the internal data needs to be wrapped in a Mutex?
    #[repr(C)]
    #[derive(Debug)]
    pub(crate) struct Pipedata {
        data: VecDeque<u8>,
    }

    impl Pipedata {
        #[inline]
        pub(crate) fn with_capacity(capacity: usize) -> Pipedata {
            Pipedata { data: VecDeque::with_capacity(capacity) }
        }

        #[inline]
        unsafe fn read(data: *mut Pipedata, buf: &mut [u8]) -> io::Result<usize> {
            unsafe { (*data).data.read(buf) }
        }

        #[inline]
        unsafe fn write(data: *mut Pipedata, buf: &[u8]) -> io::Result<usize> {
            unsafe { (*data).data.write(buf) }
        }

        #[inline]
        unsafe fn size(data: *mut Pipedata) -> usize {
            unsafe { (*data).data.len() }
        }
    }

    type WriteSignature = eficall! {fn(*mut Protocol, *mut usize, *const u8) -> r_efi::efi::Status};
    type ReadSignature = eficall! {fn(*mut Protocol, *mut usize, *mut u8) -> r_efi::efi::Status};
    type SizeSignature = eficall! {fn(*mut Protocol) -> usize};

    #[repr(C)]
    pub(crate) struct Protocol {
        pub read: ReadSignature,
        pub write: WriteSignature,
        pub size: SizeSignature,
        data: *mut Pipedata,
    }

    impl common::Protocol for Protocol {
        const PROTOCOL_GUID: Guid = PROTOCOL_GUID;
    }

    impl Protocol {
        #[inline]
        pub(crate) fn with_data(data: &mut Pipedata) -> Self {
            Self {
                data,
                read: pipe_protocol_read,
                write: pipe_protocol_write,
                size: pipe_protocol_size,
            }
        }

        #[inline]
        pub(crate) fn null() -> Self {
            Self {
                data: crate::ptr::null_mut(),
                read: pipe_protocol_null_read,
                write: pipe_protocol_null_write,
                size: pipe_protocol_null_size,
            }
        }

        unsafe fn read(protocol: *mut Protocol, buf: &mut [u8]) -> io::Result<usize> {
            unsafe {
                assert!(!(*protocol).data.is_null());
                Pipedata::read((*protocol).data, buf)
            }
        }

        unsafe fn write(protocol: *mut Protocol, buf: &[u8]) -> io::Result<usize> {
            unsafe {
                assert!(!(*protocol).data.is_null());
                Pipedata::write((*protocol).data, buf)
            }
        }

        unsafe fn size(protocol: *mut Protocol) -> usize {
            unsafe {
                assert!(!(*protocol).data.is_null());
                Pipedata::size((*protocol).data)
            }
        }
    }

    extern "efiapi" fn pipe_protocol_read(
        protocol: *mut Protocol,
        buf_size: *mut usize,
        buf: *mut u8,
    ) -> r_efi::efi::Status {
        let buffer = unsafe { crate::slice::from_raw_parts_mut(buf, buf_size.read()) };
        match unsafe { Protocol::read(protocol, buffer) } {
            Ok(x) => {
                unsafe { buf_size.write(x) };
                r_efi::efi::Status::SUCCESS
            }
            Err(_) => r_efi::efi::Status::ABORTED,
        }
    }

    extern "efiapi" fn pipe_protocol_write(
        protocol: *mut Protocol,
        buf_size: *mut usize,
        buf: *const u8,
    ) -> r_efi::efi::Status {
        let buffer = unsafe { crate::slice::from_raw_parts(buf, buf_size.read()) };
        match unsafe { Protocol::write(protocol, buffer) } {
            Ok(x) => {
                unsafe { buf_size.write(x) };
                r_efi::efi::Status::SUCCESS
            }
            Err(_) => r_efi::efi::Status::ABORTED,
        }
    }

    extern "efiapi" fn pipe_protocol_size(protocol: *mut Protocol) -> usize {
        unsafe { Protocol::size(protocol) }
    }

    extern "efiapi" fn pipe_protocol_null_write(
        _protocol: *mut Protocol,
        _buf_size: *mut usize,
        _buf: *const u8,
    ) -> r_efi::efi::Status {
        r_efi::efi::Status::SUCCESS
    }

    extern "efiapi" fn pipe_protocol_null_read(
        _protocol: *mut Protocol,
        buf_size: *mut usize,
        _buf: *mut u8,
    ) -> r_efi::efi::Status {
        unsafe { buf_size.write(0) };
        r_efi::efi::Status::SUCCESS
    }

    extern "efiapi" fn pipe_protocol_null_size(_protocol: *mut Protocol) -> usize {
        0
    }
}
