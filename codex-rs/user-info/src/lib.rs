const DEFAULT_USER_FIRST_NAME: &str = "there";

use std::path::PathBuf;

pub fn current_user_first_name() -> String {
    current_user_first_name_with_default(DEFAULT_USER_FIRST_NAME)
}

pub fn current_user_first_name_with_default(default: &str) -> String {
    [whoami::realname(), whoami::username()]
        .into_iter()
        .filter_map(|name| name.split_whitespace().next().map(str::to_string))
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub fn current_timezone() -> Option<String> {
    iana_time_zone::get_timezone().ok()
}

/// Returns the login shell path recorded for the current OS user.
#[cfg(unix)]
pub fn current_user_shell_path() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::mem::MaybeUninit;
    use std::ptr;

    let uid = unsafe { libc::getuid() };
    let mut passwd = MaybeUninit::<libc::passwd>::uninit();

    // getpwuid returns pointers into libc-managed storage. getpwuid_r keeps the
    // passwd data in caller-owned memory, avoiding races in parallel callers.
    let suggested_buffer_len = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let buffer_len = usize::try_from(suggested_buffer_len)
        .ok()
        .filter(|len| *len > 0)
        .unwrap_or(1024);
    let mut buffer = vec![0; buffer_len];

    loop {
        let mut result = ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                passwd.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };

        if status == 0 {
            if result.is_null() {
                return None;
            }

            let passwd = unsafe { passwd.assume_init_ref() };
            if passwd.pw_shell.is_null() {
                return None;
            }

            let shell_path = unsafe { CStr::from_ptr(passwd.pw_shell) }
                .to_string_lossy()
                .into_owned();
            return Some(PathBuf::from(shell_path));
        }

        if status != libc::ERANGE {
            return None;
        }

        let new_len = buffer.len().checked_mul(2)?;
        if new_len > 1024 * 1024 {
            return None;
        }
        buffer.resize(new_len, 0);
    }
}

/// Returns the login shell path recorded for the current OS user.
#[cfg(not(unix))]
pub fn current_user_shell_path() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::current_timezone;
    use super::current_user_first_name;
    use super::current_user_first_name_with_default;
    use super::current_user_shell_path;

    #[test]
    fn current_user_first_name_returns_non_empty_value() {
        assert!(!current_user_first_name().is_empty());
    }

    #[test]
    fn current_user_first_name_with_default_returns_non_empty_value() {
        assert!(!current_user_first_name_with_default("there").is_empty());
    }

    #[test]
    fn current_timezone_returns_non_empty_value_when_available() {
        if let Some(timezone) = current_timezone() {
            assert!(!timezone.is_empty());
        }
    }

    #[test]
    fn current_user_shell_path_is_queryable() {
        let _ = current_user_shell_path();
    }
}
