use crate::ffi::OsString;
use crate::fmt;
use crate::num::NonZeroU16;
use crate::os::uefi::{self, ffi::OsStringExt};
use crate::sys_common::ucs2::Ucs2Units;
use crate::vec;
use core::iter;

pub struct Args {
    parsed_args_list: vec::IntoIter<OsString>,
}

pub fn args() -> Args {
    use r_efi::efi::protocols::loaded_image;

    let mut protocol_guid = loaded_image::PROTOCOL_GUID;
    match uefi::env::get_current_handle_protocol::<loaded_image::Protocol>(&mut protocol_guid) {
        Some(x) => {
            let lp_cmd_line = unsafe { (*x.as_ptr()).load_options as *const u16 };
            let parsed_args_list =
                parse_lp_cmd_line(unsafe { Ucs2Units::new(lp_cmd_line) }, || OsString::new());

            Args { parsed_args_list: parsed_args_list.into_iter() }
        }
        None => Args { parsed_args_list: Vec::new().into_iter() },
    }
}

/// Implements the Windows command-line argument parsing algorithm. Since UEFI is so similar, this
/// can be used pretty much as is in UEFI
///
/// Microsoft's documentation for the Windows CLI argument format can be found at
/// <https://docs.microsoft.com/en-us/cpp/cpp/main-function-command-line-args?view=msvc-160#parsing-c-command-line-arguments>
///
/// A more in-depth explanation is here:
/// <https://daviddeley.com/autohotkey/parameters/parameters.htm#WIN>
///
/// This function was tested for equivalence to the C/C++ parsing rules using an
/// extensive test suite available at
/// <https://github.com/ChrisDenton/winarg/tree/std>.
fn parse_lp_cmd_line<'a, F: Fn() -> OsString>(
    lp_cmd_line: Option<Ucs2Units<'a>>,
    exe_name: F,
) -> Vec<OsString> {
    const BACKSLASH: NonZeroU16 = NonZeroU16::new(b'\\' as u16).unwrap();
    const QUOTE: NonZeroU16 = NonZeroU16::new(b'"' as u16).unwrap();
    const TAB: NonZeroU16 = NonZeroU16::new(b'\t' as u16).unwrap();
    const SPACE: NonZeroU16 = NonZeroU16::new(b' ' as u16).unwrap();

    let mut ret_val = Vec::new();
    // If the cmd line pointer is null or it points to an empty string then
    // return the name of the executable as argv[0].
    if lp_cmd_line.as_ref().and_then(|cmd| cmd.peek()).is_none() {
        ret_val.push(exe_name());
        return ret_val;
    }
    let mut code_units = lp_cmd_line.unwrap();

    // The executable name at the beginning is special.
    let mut in_quotes = false;
    let mut cur = Vec::new();
    for w in &mut code_units {
        match w {
            // A quote mark always toggles `in_quotes` no matter what because
            // there are no escape characters when parsing the executable name.
            QUOTE => in_quotes = !in_quotes,
            // If not `in_quotes` then whitespace ends argv[0].
            SPACE | TAB if !in_quotes => break,
            // In all other cases the code unit is taken literally.
            _ => cur.push(w.get()),
        }
    }
    // Skip whitespace.
    code_units.advance_while(|w| w == SPACE || w == TAB);
    ret_val.push(OsString::from_ucs2(&cur));

    // Parse the arguments according to these rules:
    // * All code units are taken literally except space, tab, quote and backslash.
    // * When not `in_quotes`, space and tab separate arguments. Consecutive spaces and tabs are
    // treated as a single separator.
    // * A space or tab `in_quotes` is taken literally.
    // * A quote toggles `in_quotes` mode unless it's escaped. An escaped quote is taken literally.
    // * A quote can be escaped if preceded by an odd number of backslashes.
    // * If any number of backslashes is immediately followed by a quote then the number of
    // backslashes is halved (rounding down).
    // * Backslashes not followed by a quote are all taken literally.
    // * If `in_quotes` then a quote can also be escaped using another quote
    // (i.e. two consecutive quotes become one literal quote).
    let mut cur = Vec::new();
    let mut in_quotes = false;
    while let Some(w) = code_units.next() {
        match w {
            // If not `in_quotes`, a space or tab ends the argument.
            SPACE | TAB if !in_quotes => {
                ret_val.push(OsString::from_ucs2(&cur[..]));
                cur.truncate(0);

                // Skip whitespace.
                code_units.advance_while(|w| w == SPACE || w == TAB);
            }
            // Backslashes can escape quotes or backslashes but only if consecutive backslashes are followed by a quote.
            BACKSLASH => {
                let backslash_count = code_units.advance_while(|w| w == BACKSLASH) + 1;
                if code_units.peek() == Some(QUOTE) {
                    cur.extend(iter::repeat(BACKSLASH.get()).take(backslash_count / 2));
                    // The quote is escaped if there are an odd number of backslashes.
                    if backslash_count % 2 == 1 {
                        code_units.next();
                        cur.push(QUOTE.get());
                    }
                } else {
                    // If there is no quote on the end then there is no escaping.
                    cur.extend(iter::repeat(BACKSLASH.get()).take(backslash_count));
                }
            }
            // If `in_quotes` and not backslash escaped (see above) then a quote either
            // unsets `in_quote` or is escaped by another quote.
            QUOTE if in_quotes => match code_units.peek() {
                // Two consecutive quotes when `in_quotes` produces one literal quote.
                Some(QUOTE) => {
                    cur.push(QUOTE.get());
                    code_units.next();
                }
                // Otherwise set `in_quotes`.
                Some(_) => in_quotes = false,
                // The end of the command line.
                // Push `cur` even if empty, which we do by breaking while `in_quotes` is still set.
                None => break,
            },
            // If not `in_quotes` and not BACKSLASH escaped (see above) then a quote sets `in_quote`.
            QUOTE => in_quotes = true,
            // Everything else is always taken literally.
            _ => cur.push(w.get()),
        }
    }
    // Push the final argument, if any.
    if !cur.is_empty() || in_quotes {
        ret_val.push(OsString::from_ucs2(&cur[..]));
    }
    ret_val
}

impl fmt::Debug for Args {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.parsed_args_list.as_slice().fmt(f)
    }
}

impl Iterator for Args {
    type Item = OsString;
    fn next(&mut self) -> Option<OsString> {
        self.parsed_args_list.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.parsed_args_list.size_hint()
    }
}

impl DoubleEndedIterator for Args {
    fn next_back(&mut self) -> Option<OsString> {
        self.parsed_args_list.next_back()
    }
}

impl ExactSizeIterator for Args {
    fn len(&self) -> usize {
        self.parsed_args_list.len()
    }
}
