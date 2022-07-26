use super::uefi_service_binding::ServiceBinding;
use crate::io::{self, IoSlice, IoSliceMut};
use crate::mem::MaybeUninit;
use crate::net::{Ipv4Addr, SocketAddrV4};
use crate::os::uefi;
use crate::os::uefi::raw::VariableSizeType;
use crate::ptr::NonNull;
use crate::sys::uefi::common::status_to_io_error;
use r_efi::efi::Status;
use r_efi::protocols::{ip4, managed_network, simple_network, tcp4};

// FIXME: Discuss what the values these constants should have
const TYPE_OF_SERVICE: u8 = 8;
const TIME_TO_LIVE: u8 = 255;

pub struct Tcp4Protocol {
    protocol: NonNull<tcp4::Protocol>,
    service_binding: ServiceBinding,
    child_handle: NonNull<crate::ffi::c_void>,
}

impl Tcp4Protocol {
    pub fn create(service_binding: ServiceBinding) -> io::Result<Tcp4Protocol> {
        let child_handle = service_binding.create_child()?;
        Self::with_child_handle(service_binding, child_handle)
    }

    pub fn config(
        &self,
        use_default_address: bool,
        active_flag: bool,
        station_addr: &crate::net::SocketAddrV4,
        subnet_mask: &crate::net::Ipv4Addr,
        remote_addr: &crate::net::SocketAddrV4,
    ) -> io::Result<()> {
        let mut config_data = tcp4::ConfigData {
            // FIXME: Check in mailing list what traffic_class should be used
            type_of_service: TYPE_OF_SERVICE,
            // FIXME: Check in mailing list what hop_limit should be used
            time_to_live: TIME_TO_LIVE,
            access_point: tcp4::AccessPoint {
                use_default_address: r_efi::efi::Boolean::from(use_default_address),
                station_address: r_efi::efi::Ipv4Address::from(station_addr.ip()),
                station_port: station_addr.port(),
                subnet_mask: r_efi::efi::Ipv4Address::from(subnet_mask),
                remote_address: r_efi::efi::Ipv4Address::from(remote_addr.ip()),
                remote_port: remote_addr.port(),
                active_flag: r_efi::efi::Boolean::from(active_flag),
            },
            // FIXME: Maybe provide a rust default one at some point
            control_option: crate::ptr::null_mut(),
        };
        unsafe { Self::config_raw(self.protocol.as_ptr(), &mut config_data) }
    }

    pub fn accept(&self) -> io::Result<Tcp4Protocol> {
        let accept_event = uefi::thread::Event::create(
            r_efi::efi::EVT_NOTIFY_WAIT,
            r_efi::efi::TPL_CALLBACK,
            Some(nop_notify4),
            None,
        )?;
        let completion_token =
            tcp4::CompletionToken { event: accept_event.as_raw_event(), status: Status::ABORTED };

        let mut listen_token = tcp4::ListenToken {
            completion_token,
            new_child_handle: unsafe { MaybeUninit::<r_efi::efi::Handle>::uninit().assume_init() },
        };

        unsafe { Self::accept_raw(self.protocol.as_ptr(), &mut listen_token) }?;

        accept_event.wait()?;

        let r = listen_token.completion_token.status;
        if r.is_error() {
            Err(status_to_io_error(r))
        } else {
            let child_handle = NonNull::new(listen_token.new_child_handle)
                .ok_or(io::Error::new(io::ErrorKind::Other, "Null Child Handle"))?;
            Self::with_child_handle(self.service_binding, child_handle)
        }
    }

    pub fn connect(&self) -> io::Result<()> {
        todo!()
    }

    pub fn transmit(&self, buf: &[u8]) -> io::Result<usize> {
        let buf_size = buf.len() as u32;
        let transmit_event = uefi::thread::Event::create(
            r_efi::efi::EVT_NOTIFY_WAIT,
            r_efi::efi::TPL_CALLBACK,
            Some(nop_notify4),
            None,
        )?;
        let completion_token =
            tcp4::CompletionToken { event: transmit_event.as_raw_event(), status: Status::ABORTED };
        let fragment_table = tcp4::FragmentData {
            fragment_length: buf_size,
            // FIXME: Probably dangerous
            fragment_buffer: buf.as_ptr() as *mut crate::ffi::c_void,
        };

        let transmit_data: VariableSizeType<tcp4::TransmitData> = VariableSizeType::from_size(
            crate::mem::size_of::<tcp4::TransmitData>()
                + crate::mem::size_of::<tcp4::FragmentData>(),
        )?;

        // Initialize VariableSizeType
        unsafe {
            (*transmit_data.as_ptr()).push = r_efi::efi::Boolean::from(true);
            (*transmit_data.as_ptr()).urgent = r_efi::efi::Boolean::from(false);
            (*transmit_data.as_ptr()).data_length = buf_size;
            (*transmit_data.as_ptr()).fragment_count = 1;
            crate::ptr::copy(
                [fragment_table].as_ptr(),
                (*transmit_data.as_ptr()).fragment_table.as_mut_ptr(),
                1,
            )
        };

        let packet = tcp4::IoTokenPacket { tx_data: transmit_data.as_ptr() };
        let mut transmit_token = tcp4::IoToken { completion_token, packet };
        unsafe { Self::transmit_raw(self.protocol.as_ptr(), &mut transmit_token) }?;

        transmit_event.wait()?;

        let r = transmit_token.completion_token.status;
        if r.is_error() {
            Err(status_to_io_error(r))
        } else {
            Ok(unsafe { (*transmit_token.packet.tx_data).data_length } as usize)
        }
    }

    pub fn transmit_vectored(&self, buf: &[IoSlice<'_>]) -> io::Result<usize> {
        let buf_size = crate::mem::size_of_val(buf);
        let transmit_event = uefi::thread::Event::create(
            r_efi::efi::EVT_NOTIFY_WAIT,
            r_efi::efi::TPL_CALLBACK,
            Some(nop_notify4),
            None,
        )?;
        let completion_token =
            tcp4::CompletionToken { event: transmit_event.as_raw_event(), status: Status::ABORTED };
        let fragment_tables: Vec<tcp4::FragmentData> = buf
            .iter()
            .map(|b| tcp4::FragmentData {
                fragment_length: crate::mem::size_of_val(b) as u32,
                fragment_buffer: (*b).as_ptr() as *mut crate::ffi::c_void,
            })
            .collect();

        let transmit_data: VariableSizeType<tcp4::TransmitData> = VariableSizeType::from_size(
            crate::mem::size_of::<tcp4::TransmitData>() + crate::mem::size_of_val(&fragment_tables),
        )?;
        let fragment_tables_len = fragment_tables.len();

        // Initialize VariableSizeType
        unsafe {
            (*transmit_data.as_ptr()).push = r_efi::efi::Boolean::from(true);
            (*transmit_data.as_ptr()).urgent = r_efi::efi::Boolean::from(false);
            (*transmit_data.as_ptr()).data_length = buf_size as u32;
            (*transmit_data.as_ptr()).fragment_count = fragment_tables_len as u32;
            crate::ptr::copy(
                fragment_tables.as_ptr(),
                (*transmit_data.as_ptr()).fragment_table.as_mut_ptr(),
                fragment_tables_len,
            )
        };

        let packet = tcp4::IoTokenPacket { tx_data: transmit_data.as_ptr() };
        let mut transmit_token = tcp4::IoToken { completion_token, packet };
        unsafe { Self::transmit_raw(self.protocol.as_ptr(), &mut transmit_token) }?;

        transmit_event.wait()?;

        let r = transmit_token.completion_token.status;
        if r.is_error() {
            Err(status_to_io_error(r))
        } else {
            Ok(unsafe { (*transmit_token.packet.tx_data).data_length } as usize)
        }
    }

    pub fn receive(&self, buf: &mut [u8]) -> io::Result<usize> {
        let buf_size = buf.len() as u32;
        let receive_event = uefi::thread::Event::create(
            r_efi::efi::EVT_NOTIFY_WAIT,
            r_efi::efi::TPL_CALLBACK,
            Some(nop_notify4),
            None,
        )?;
        let fragment_table = tcp4::FragmentData {
            fragment_length: buf_size,
            fragment_buffer: buf.as_mut_ptr().cast(),
        };

        let receive_data: VariableSizeType<tcp4::ReceiveData> = VariableSizeType::from_size(
            crate::mem::size_of::<tcp4::ReceiveData>()
                + crate::mem::size_of::<tcp4::FragmentData>(),
        )?;

        unsafe {
            (*receive_data.as_ptr()).urgent_flag = r_efi::efi::Boolean::from(false);
            (*receive_data.as_ptr()).data_length = buf_size;
            (*receive_data.as_ptr()).fragment_count = 1;
            crate::ptr::copy(
                [fragment_table].as_ptr(),
                (*receive_data.as_ptr()).fragment_table.as_mut_ptr(),
                1,
            )
        }

        let packet = tcp4::IoTokenPacket { rx_data: receive_data.as_ptr() };
        let completion_token =
            tcp4::CompletionToken { event: receive_event.as_raw_event(), status: Status::ABORTED };
        let mut receive_token = tcp4::IoToken { completion_token, packet };
        unsafe { Self::receive_raw(self.protocol.as_ptr(), &mut receive_token) }?;

        receive_event.wait()?;

        let r = receive_token.completion_token.status;
        if r.is_error() {
            Err(status_to_io_error(r))
        } else {
            Ok(unsafe { (*receive_token.packet.rx_data).data_length } as usize)
        }
    }

    pub fn receive_vectored(&self, buf: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        let receive_event = uefi::thread::Event::create(
            r_efi::efi::EVT_NOTIFY_WAIT,
            r_efi::efi::TPL_CALLBACK,
            Some(nop_notify4),
            None,
        )?;

        let buf_size = crate::mem::size_of_val(&buf) as u32;
        let fragment_tables: Vec<tcp4::FragmentData> = buf
            .iter_mut()
            .map(|b| tcp4::FragmentData {
                fragment_length: crate::mem::size_of_val(b) as u32,
                fragment_buffer: b.as_mut_ptr().cast(),
            })
            .collect();
        let fragment_tables_len = fragment_tables.len();

        let receive_data: VariableSizeType<tcp4::ReceiveData> = VariableSizeType::from_size(
            crate::mem::size_of::<tcp4::ReceiveData>() + crate::mem::size_of_val(&fragment_tables),
        )?;

        unsafe {
            (*receive_data.as_ptr()).urgent_flag = r_efi::efi::Boolean::from(false);
            (*receive_data.as_ptr()).data_length = buf_size;
            (*receive_data.as_ptr()).fragment_count = fragment_tables_len as u32;
            crate::ptr::copy(
                fragment_tables.as_ptr(),
                (*receive_data.as_ptr()).fragment_table.as_mut_ptr(),
                fragment_tables_len,
            )
        }

        let packet = tcp4::IoTokenPacket { rx_data: receive_data.as_ptr() };
        let completion_token =
            tcp4::CompletionToken { event: receive_event.as_raw_event(), status: Status::ABORTED };
        let mut receive_token = tcp4::IoToken { completion_token, packet };
        unsafe { Self::receive_raw(self.protocol.as_ptr(), &mut receive_token) }?;

        receive_event.wait()?;

        let r = receive_token.completion_token.status;
        if r.is_error() {
            Err(status_to_io_error(r))
        } else {
            Ok(unsafe { (*receive_token.packet.rx_data).data_length } as usize)
        }
    }

    pub fn close(&self, abort_on_close: bool) -> io::Result<()> {
        let protocol = self.protocol.as_ptr();

        let close_event = uefi::thread::Event::create(
            r_efi::efi::EVT_NOTIFY_WAIT,
            r_efi::efi::TPL_CALLBACK,
            Some(nop_notify4),
            None,
        )?;
        let completion_token =
            tcp4::CompletionToken { event: close_event.as_raw_event(), status: Status::ABORTED };
        let mut close_token = tcp4::CloseToken {
            abort_on_close: r_efi::efi::Boolean::from(abort_on_close),
            completion_token,
        };
        let r = unsafe { ((*protocol).close)(protocol, &mut close_token) };

        if r.is_error() {
            return Err(status_to_io_error(r));
        }

        close_event.wait()?;

        let r = close_token.completion_token.status;
        if r.is_error() { Err(status_to_io_error(r)) } else { Ok(()) }
    }

    pub fn remote_socket(&self) -> io::Result<SocketAddrV4> {
        let config_data = self.get_config_data()?;
        Ok(SocketAddrV4::new(
            Ipv4Addr::from(config_data.access_point.remote_address),
            config_data.access_point.remote_port,
        ))
    }

    pub fn station_socket(&self) -> io::Result<SocketAddrV4> {
        let config_data = self.get_config_data()?;
        Ok(SocketAddrV4::new(
            Ipv4Addr::from(config_data.access_point.station_address),
            config_data.access_point.station_port,
        ))
    }

    fn new(
        protocol: NonNull<tcp4::Protocol>,
        service_binding: ServiceBinding,
        child_handle: NonNull<crate::ffi::c_void>,
    ) -> Self {
        Self { protocol, service_binding, child_handle }
    }

    fn with_child_handle(
        service_binding: ServiceBinding,
        child_handle: NonNull<crate::ffi::c_void>,
    ) -> io::Result<Self> {
        let tcp4_protocol = uefi::env::open_protocol(child_handle, tcp4::PROTOCOL_GUID)?;
        Ok(Self::new(tcp4_protocol, service_binding, child_handle))
    }

    // FIXME: This function causes the program to freeze.
    fn get_config_data(&self) -> io::Result<tcp4::ConfigData> {
        let protocol = self.protocol.as_ptr();

        let mut state: MaybeUninit<tcp4::ConnectionState> = MaybeUninit::uninit();
        let mut config_data: MaybeUninit<tcp4::ConfigData> = MaybeUninit::uninit();
        let mut ip4_mode_data: MaybeUninit<ip4::ModeData> = MaybeUninit::uninit();
        let mut mnp_mode_data: MaybeUninit<managed_network::ConfigData> = MaybeUninit::uninit();
        let mut snp_mode_data: MaybeUninit<simple_network::Mode> = MaybeUninit::uninit();

        let r = unsafe {
            ((*protocol).get_mode_data)(
                protocol,
                state.as_mut_ptr(),
                config_data.as_mut_ptr(),
                ip4_mode_data.as_mut_ptr(),
                mnp_mode_data.as_mut_ptr(),
                snp_mode_data.as_mut_ptr(),
            )
        };

        if r.is_error() {
            Err(status_to_io_error(r))
        } else {
            unsafe {
                state.assume_init_drop();
                ip4_mode_data.assume_init_drop();
                mnp_mode_data.assume_init_drop();
                snp_mode_data.assume_init_drop();
            }
            Ok(unsafe { config_data.assume_init() })
        }
    }

    unsafe fn receive_raw(
        protocol: *mut tcp4::Protocol,
        token: *mut tcp4::IoToken,
    ) -> io::Result<()> {
        let r = unsafe { ((*protocol).receive)(protocol, token) };

        if r.is_error() { Err(status_to_io_error(r)) } else { Ok(()) }
    }

    unsafe fn transmit_raw(
        protocol: *mut tcp4::Protocol,
        token: *mut tcp4::IoToken,
    ) -> io::Result<()> {
        let r = unsafe { ((*protocol).transmit)(protocol, token) };

        if r.is_error() { Err(status_to_io_error(r)) } else { Ok(()) }
    }

    unsafe fn config_raw(
        protocol: *mut tcp4::Protocol,
        config_data: *mut tcp4::ConfigData,
    ) -> io::Result<()> {
        let r = unsafe { ((*protocol).configure)(protocol, config_data) };

        if r.is_error() { Err(status_to_io_error(r)) } else { Ok(()) }
    }

    unsafe fn accept_raw(
        protocol: *mut tcp4::Protocol,
        token: *mut tcp4::ListenToken,
    ) -> io::Result<()> {
        let r = unsafe { ((*protocol).accept)(protocol, token) };

        if r.is_error() { Err(status_to_io_error(r)) } else { Ok(()) }
    }
}

impl Drop for Tcp4Protocol {
    fn drop(&mut self) {
        let _ = self.close(true);
        let _ = self.service_binding.destroy_child(self.child_handle);
    }
}

#[no_mangle]
pub extern "efiapi" fn nop_notify4(_: r_efi::efi::Event, _: *mut crate::ffi::c_void) {}
