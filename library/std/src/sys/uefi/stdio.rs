use crate::sys_common::ucs2;
use crate::{io, os::uefi, ptr::NonNull};
use r_efi::protocols::{simple_text_input, simple_text_output};
use r_efi::system::BootWaitForEvent;

pub struct Stdin(());
pub struct Stdout(());
pub struct Stderr(());

const MAX_BUFFER_SIZE: usize = 8192;

pub const STDIN_BUF_SIZE: usize = MAX_BUFFER_SIZE / 2 * 3;

impl Stdin {
    pub const fn new() -> Stdin {
        Stdin(())
    }

    // FIXME: Improve Errors
    fn fire_wait_event(
        con_in: NonNull<simple_text_input::Protocol>,
        wait_for_event: BootWaitForEvent,
    ) -> io::Result<()> {
        let r = unsafe {
            let mut x: usize = 0;
            (wait_for_event)(1, &mut (*con_in.as_ptr()).wait_for_key, &mut x)
        };

        if r.is_error() {
            Err(io::Error::new(io::ErrorKind::Other, "Could not wait for event"))
        } else {
            Ok(())
        }
    }

    // FIXME Improve Errors
    fn read_key_stroke(con_in: NonNull<simple_text_input::Protocol>) -> io::Result<u16> {
        let mut input_key = simple_text_input::InputKey::default();
        let r = unsafe { ((*con_in.as_ptr()).read_key_stroke)(con_in.as_ptr(), &mut input_key) };

        if r.is_error() || input_key.scan_code != 0 {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "Error in Reading Keystroke"))
        } else {
            Ok(input_key.unicode_char)
        }
    }

    // FIXME Improve Errors
    fn reset_weak(con_in: NonNull<simple_text_input::Protocol>) -> io::Result<()> {
        let r = unsafe { ((*con_in.as_ptr()).reset)(con_in.as_ptr(), r_efi::efi::Boolean::TRUE) };

        if r.is_error() {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "Device Error"))
        } else {
            Ok(())
        }
    }

    // FIXME Improve Errors
    fn write_character(
        con_out: NonNull<simple_text_output::Protocol>,
        character: ucs2::Ucs2Char,
    ) -> io::Result<()> {
        let mut buf: [u16; 2] = [character.into(), 0];
        let r = unsafe { ((*con_out.as_ptr()).output_string)(con_out.as_ptr(), buf.as_mut_ptr()) };

        if r.is_error() {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "Device Error"))
        } else if character == ucs2::Ucs2Char::CR {
            // Handle enter key
            Self::write_character(con_out, ucs2::Ucs2Char::LF)
        } else {
            Ok(())
        }
    }
}

impl io::Read for Stdin {
    // Reads 1 UCS-2 character at a time and returns.
    // FIXME: Implement buffered reading. Currently backspace and other characters are read as
    // normal characters. Thus it might look like line-editing but it actually isn't
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Ok(current_exe) = crate::env::current_exe() {
            if let Ok(v) = crate::env::var(format!("{}_stdin", current_exe.to_string_lossy())) {
                if v.as_str() == "null" {
                    return Ok(buf.len());
                }
            }
        }

        let global_system_table = uefi::env::get_system_table()
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "Global System Table"))?;
        let con_in = get_con_in(global_system_table)?;
        let con_out = get_con_out(global_system_table)?;
        let wait_for_event = get_wait_for_event(global_system_table)?;

        if buf.len() < 3 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Buffer too small"));
        }

        let ch = {
            Stdin::reset_weak(con_in)?;
            Stdin::fire_wait_event(con_in, wait_for_event)?;
            Stdin::read_key_stroke(con_in)?
        };

        let ch = ucs2::Ucs2Char::from_u16(ch);
        Stdin::write_character(con_out, ch)?;

        let ch = char::from(ch);
        let bytes_read = ch.len_utf8();

        // Replace CR with LF
        if ch == '\r' {
            '\n'.encode_utf8(buf);
        } else {
            ch.encode_utf8(buf);
        }

        Ok(bytes_read)
    }
}

impl Stdout {
    pub const fn new() -> Stdout {
        Stdout(())
    }
}

impl io::Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(current_exe) = crate::env::current_exe() {
            if let Ok(v) = crate::env::var(format!("{}_stdout", current_exe.to_string_lossy())) {
                if v.as_str() == "null" {
                    return Ok(buf.len());
                } else {
                    return super::pipe::AnonPipe::new(v).write(buf);
                }
            }
        }
        let global_system_table = uefi::env::get_system_table()
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "Global System Table"))?;
        let con_out = get_con_out(global_system_table)?;
        simple_text_output_write(con_out, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Stderr {
    pub const fn new() -> Stderr {
        Stderr(())
    }
}

impl io::Write for Stderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(current_exe) = crate::env::current_exe() {
            if let Ok(v) = crate::env::var(format!("{}_stderr", current_exe.to_string_lossy())) {
                if v.as_str() == "null" {
                    return Ok(buf.len());
                } else {
                    return super::pipe::AnonPipe::new(v).write(buf);
                }
            }
        }

        let global_system_table = uefi::env::get_system_table()
            .ok_or(io::Error::new(io::ErrorKind::NotFound, "Global System Table"))?;
        let std_err = get_std_err(global_system_table)?;
        simple_text_output_write(std_err, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn is_ebadf(err: &io::Error) -> bool {
    err.raw_os_error() == Some(r_efi::efi::Status::DEVICE_ERROR.as_usize() as i32)
}

pub fn panic_output() -> Option<impl io::Write> {
    Some(Stderr::new())
}

fn utf8_to_ucs2(buf: &[u8], output: &mut [u16]) -> io::Result<usize> {
    let iter = ucs2::EncodeUcs2::from_bytes(buf)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid Output buffer"))?;
    let mut count = 0;
    let mut bytes_read = 0;

    for ch in iter {
        // Convert LF to CRLF
        if ch == ucs2::Ucs2Char::LF {
            output[count] = u16::from(ucs2::Ucs2Char::CR);
            count += 1;

            if count + 1 >= output.len() {
                break;
            }
        }

        bytes_read += ch.len_utf8();
        output[count] = u16::from(ch);
        count += 1;

        if count + 1 >= output.len() {
            break;
        }
    }

    output[count] = 0;
    Ok(bytes_read)
}

fn simple_text_output_write(
    protocol: NonNull<simple_text_output::Protocol>,
    buf: &[u8],
) -> io::Result<usize> {
    let output_string_ptr = unsafe { (*protocol.as_ptr()).output_string };

    let mut output = [0u16; MAX_BUFFER_SIZE / 2];
    let count = utf8_to_ucs2(buf, &mut output)?;

    let r = (output_string_ptr)(protocol.as_ptr(), output.as_mut_ptr());

    if r.is_error() {
        Err(io::Error::new(io::ErrorKind::Other, r.as_usize().to_string()))
    } else {
        Ok(count)
    }
}

// Returns error if `SystemTable->ConIn` is null.
fn get_con_in(
    st: NonNull<uefi::raw::SystemTable>,
) -> io::Result<NonNull<simple_text_input::Protocol>> {
    let con_in = unsafe { (*st.as_ptr()).con_in };
    NonNull::new(con_in).ok_or(io::Error::new(io::ErrorKind::NotFound, "ConIn"))
}

fn get_wait_for_event(st: NonNull<uefi::raw::SystemTable>) -> io::Result<BootWaitForEvent> {
    let boot_services = unsafe { (*st.as_ptr()).boot_services };

    if boot_services.is_null() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Boot Services"));
    }

    Ok(unsafe { (*boot_services).wait_for_event })
}

// Returns error if `SystemTable->ConOut` is null.
fn get_con_out(
    st: NonNull<uefi::raw::SystemTable>,
) -> io::Result<NonNull<simple_text_output::Protocol>> {
    let con_out = unsafe { (*st.as_ptr()).con_out };
    NonNull::new(con_out).ok_or(io::Error::new(io::ErrorKind::NotFound, "ConOut"))
}

// Returns error if `SystemTable->StdErr` is null.
fn get_std_err(
    st: NonNull<uefi::raw::SystemTable>,
) -> io::Result<NonNull<simple_text_output::Protocol>> {
    let std_err = unsafe { (*st.as_ptr()).std_err };
    NonNull::new(std_err).ok_or(io::Error::new(io::ErrorKind::NotFound, "StdErr"))
}
