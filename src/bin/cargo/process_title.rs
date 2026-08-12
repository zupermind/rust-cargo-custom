/// Sets the Linux process title from `Z_PROG_NAME` when the variable is present.
///
/// Linux exposes this title through `/proc/<pid>/comm` and limits it to 15 bytes.
/// Longer UTF-8 values are truncated at a character boundary.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn set_from_env() {
    use std::ffi::CString;

    if let Ok(title) = std::env::var("Z_PROG_NAME") {
        let mut end = title.len().min(15);
        while !title.is_char_boundary(end) {
            end -= 1;
        }

        if let Ok(title) = CString::new(&title[..end]) {
            // SAFETY: `title` is a live NUL-terminated string for the duration of the call.
            unsafe {
                libc::prctl(libc::PR_SET_NAME, title.as_ptr());
            }
        }
    }

    set_process_args_from_env();
}

/// Leaves the process title unchanged on unsupported platforms.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn set_from_env() {}

/// Replaces the Linux kernel-visible command line with `Z_PROG_ARGS`.
///
/// Rust's argument iterator reads the original `argv` pointers lazily. Before
/// overwriting their backing memory, redirect those pointers to process-lifetime
/// copies so Cargo argument parsing continues to see the real invocation.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn set_process_args_from_env() {
    use std::ffi::{CStr, CString, OsString};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let Some(title) = std::env::var_os("Z_PROG_ARGS") else {
        return;
    };
    let title = title.as_bytes();

    let Ok(original_args): Result<Vec<CString>, _> = std::env::args_os()
        .map(OsString::into_vec)
        .map(CString::new)
        .collect()
    else {
        return;
    };
    if original_args.is_empty() {
        return;
    }

    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return;
    };
    let Some(command_end) = stat.rfind(')') else {
        return;
    };
    let mut fields = stat[command_end + 1..].split_ascii_whitespace();
    let Some(arg_start) = fields.nth(45).and_then(|field| field.parse::<usize>().ok()) else {
        return;
    };
    let Some(arg_end) = fields.next().and_then(|field| field.parse::<usize>().ok()) else {
        return;
    };
    let Some(capacity) = arg_end.checked_sub(arg_start) else {
        return;
    };
    if capacity == 0 {
        return;
    }

    unsafe extern "C" {
        static mut environ: *mut *mut libc::c_char;
    }

    // SAFETY: On glibc Linux at process entry, `environ` follows the NULL that
    // terminates the argv pointer array. Validate every recovered pointer and
    // byte string before modifying either the pointer array or argument span.
    unsafe {
        let environment = environ;
        if environment.is_null() {
            return;
        }
        let argv = environment.sub(original_args.len() + 1);
        if !(*argv.add(original_args.len())).is_null() {
            return;
        }
        for (index, original) in original_args.iter().enumerate() {
            let argument = *argv.add(index);
            if argument.is_null() || CStr::from_ptr(argument).to_bytes() != original.as_bytes() {
                return;
            }
        }

        for (index, original) in original_args.into_iter().enumerate() {
            *argv.add(index) = original.into_raw();
        }

        let title_len = title.len().min(capacity - 1);
        std::ptr::write_bytes(arg_start as *mut u8, 0, capacity);
        std::ptr::copy_nonoverlapping(title.as_ptr(), arg_start as *mut u8, title_len);
    }
}
