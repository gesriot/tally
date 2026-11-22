//! CreateProcess может передать исходные дескрипторы даже с переназначенными
//! stdio. Окно не должно удерживать каналы, из которых читают вывод tally.
#![allow(unsafe_code)]

use std::io;

use windows_sys::Win32::{
    Foundation::{
        GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        SetHandleInformation,
    },
    System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
};

pub(super) struct StandardStreams(Vec<HANDLE>);

impl StandardStreams {
    pub(super) fn prevent_inheritance() -> io::Result<Self> {
        let mut streams = Self(Vec::new());
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: GetStdHandle returns a borrowed standard handle. We do not close
            // it. In the CLI it stays open throughout this synchronous spawn.
            let handle = unsafe { GetStdHandle(which) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            let mut flags = 0;
            // Some console handles don't report flags. Only modify handles known
            // to be inheritable. The out-pointer refers to an initialized DWORD.
            if unsafe { GetHandleInformation(handle, &mut flags) } != 0
                && flags & HANDLE_FLAG_INHERIT != 0
            {
                // SAFETY: only the inheritance bit of the borrowed handle is changed.
                if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                streams.0.push(handle);
            }
        }
        Ok(streams)
    }
}

impl Drop for StandardStreams {
    fn drop(&mut self) {
        for &handle in &self.0 {
            // SAFETY: the standard handle is still open; this restores its original bit.
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        }
    }
}
