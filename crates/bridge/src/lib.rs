//! Temporary C adapters. Permanent crates never depend on this crate.
mod clock;
mod power;
mod theme;
mod wayland;
mod wm;
use std::{
    ffi::{CStr, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
};
use way_shell_core::{
    audio::{self, Scale},
    gamma,
};

fn scale(value: i32) -> Scale {
    if value == 1 {
        Scale::Cubic
    } else {
        Scale::Linear
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn way_shell_gamma_supported(size: usize, temperature: i32) -> i32 {
    i32::from(gamma::is_supported(size, temperature as u32))
}

#[unsafe(no_mangle)]
pub extern "C" fn volume_from_linear(volume: f32, curve: i32) -> f64 {
    audio::from_linear(volume, scale(curve))
}

#[unsafe(no_mangle)]
pub extern "C" fn volume_to_linear(volume: f64, curve: i32) -> f32 {
    audio::to_linear(volume, scale(curve))
}

/// # Safety
/// A non-null channel points to a readable, NUL-terminated string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_channel_index(channel: *const c_char) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if channel.is_null() {
            return -1;
        }
        // The adapter borrows the C-owned string only for this call.
        let channel = unsafe { CStr::from_ptr(channel) };
        channel
            .to_str()
            .ok()
            .and_then(audio::channel_index)
            .map_or(-1, |index| index as i32)
    }))
    .unwrap_or(-1)
}

/// # Safety
/// Each pointer addresses `size` writable u16 values; the three buffers must
/// not overlap and remain valid for this synchronous call. Ownership stays in C.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn way_shell_gamma_fill(
    red: *mut u16,
    green: *mut u16,
    blue: *mut u16,
    size: usize,
    temperature: i32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if red.is_null()
            || green.is_null()
            || blue.is_null()
            || size == 0
            || size > gamma::MAX_RAMP_SIZE
        {
            return -1;
        }
        // C's single gamma allocation is divided into three disjoint channels.
        let (red, green, blue) = unsafe {
            (
                std::slice::from_raw_parts_mut(red, size),
                std::slice::from_raw_parts_mut(green, size),
                std::slice::from_raw_parts_mut(blue, size),
            )
        };
        if gamma::apply(red, green, blue, temperature as u32) {
            0
        } else {
            -1
        }
    }))
    .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_arguments_and_unknown_channels_are_rejected() {
        // Null inputs are explicitly accepted by these adapters as errors.
        unsafe {
            assert_eq!(way_shell_channel_index(std::ptr::null()), -1);
            assert_eq!(way_shell_channel_index(c"FL".as_ptr()), 2);
            assert_eq!(way_shell_channel_index(c"MONO".as_ptr()), -1);
            assert_eq!(
                way_shell_gamma_fill(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    256,
                    6500
                ),
                -1
            );
        }
    }
}
