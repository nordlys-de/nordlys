//! Checking a password with PAM, the way every Linux locker does.
//!
//! A few lines of FFI against libpam instead of a bindings crate, so the
//! build needs no clang. The service is `nordlys` when
//! `/etc/pam.d/nordlys` exists (see config/pam.d/nordlys), otherwise
//! `login`, which is what hyprlock and swaylock include too.

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConvFn = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: ConvFn,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        pamh: *mut *mut c_void,
    ) -> c_int;
    fn pam_authenticate(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut c_void, status: c_int) -> c_int;
}

// From libc, which std already links.
unsafe extern "C" {
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn strdup(s: *const c_char) -> *mut c_char;
}

/// Answer PAM's prompts: the password for hidden ones, nothing for the rest.
/// PAM frees the answers, so they are allocated with the C allocator.
unsafe extern "C" fn converse(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || resp.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    let n = num_msg as usize;
    // SAFETY: PAM passes `num_msg` messages and a slot for the answers;
    // appdata is the CString we gave pam_start, alive for the whole call.
    unsafe {
        let answers = calloc(n, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if answers.is_null() {
            return PAM_BUF_ERR;
        }
        let password = appdata as *const c_char;
        for i in 0..n {
            let m = *msg.add(i);
            if m.is_null() {
                continue;
            }
            let answer = match (*m).msg_style {
                PAM_PROMPT_ECHO_OFF => strdup(password),
                PAM_PROMPT_ECHO_ON => strdup(c"".as_ptr()),
                // Info and error text: nothing to answer.
                _ => ptr::null_mut(),
            };
            (*answers.add(i)).resp = answer;
            (*answers.add(i)).resp_retcode = 0;
        }
        *resp = answers;
    }
    PAM_SUCCESS
}

/// The PAM service to use.
pub fn service() -> &'static str {
    if Path::new("/etc/pam.d/nordlys").exists() {
        "nordlys"
    } else {
        "login"
    }
}

/// A password held only as long as needed, wiped when dropped.
pub struct Secret(String);

impl Secret {
    pub fn new(s: String) -> Self {
        Secret(s)
    }

    /// Compare without keeping a copy (for the preview).
    pub fn is(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8, so the String stays valid.
        unsafe { self.0.as_mut_vec().fill(0) };
    }
}

/// Check `user`'s password. Blocks (PAM may wait a few seconds after a
/// wrong one), so run it off the UI thread.
pub fn authenticate(user: &str, password: Secret) -> Result<(), String> {
    let service = CString::new(service()).map_err(|e| e.to_string())?;
    let user = CString::new(user).map_err(|_| "bad user name".to_owned())?;
    // A NUL inside can't be typed; treat it as a wrong password.
    let pass = CString::new(password.0.as_bytes()).map_err(|_| "wrong".to_owned())?;
    drop(password);
    let conv = PamConv {
        conv: converse,
        appdata_ptr: pass.as_ptr() as *mut c_void,
    };
    let mut handle: *mut c_void = ptr::null_mut();
    // SAFETY: every pointer outlives the PAM transaction, which ends with
    // pam_end before this function returns.
    let result = unsafe {
        let started = pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle);
        if started != PAM_SUCCESS || handle.is_null() {
            Err(format!("PAM could not start ({started})"))
        } else {
            // Only the password is checked, as swaylock and hyprlock do: you
            // are already logged in. pam_acct_mgmt would also read the
            // service's "account" lines, and config/pam.d/nordlys has none,
            // so PAM falls back to "other", which is pam_deny on Arch.
            let status = pam_authenticate(handle, 0);
            pam_end(handle, status);
            if status == PAM_SUCCESS {
                Ok(())
            } else {
                Err("wrong".into())
            }
        }
    };
    // Wipe our copy too.
    pass.into_bytes().fill(0);
    result
}

/// The user running the lock and a friendly name for them.
pub fn current_user() -> (String, String) {
    let login = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "user".into());
    let full = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|passwd| full_name(&passwd, &login))
        .unwrap_or_else(|| login.clone());
    (login, full)
}

/// The first part of the GECOS field in /etc/passwd, if set.
fn full_name(passwd: &str, login: &str) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next() != Some(login) {
            return None;
        }
        let gecos = fields.nth(3)?.split(',').next()?.trim();
        (!gecos.is_empty()).then(|| gecos.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_passwd() {
        let passwd = "root:x:0:0::/root:/bin/bash\n\
                      ada:x:1000:1000:Ada Lovelace,,,:/home/ada:/bin/zsh\n\
                      bob:x:1001:1001::/home/bob:/bin/sh\n";
        assert_eq!(full_name(passwd, "ada").as_deref(), Some("Ada Lovelace"));
        assert_eq!(full_name(passwd, "bob"), None);
        assert_eq!(full_name(passwd, "carol"), None);
    }

    /// Runs the real PAM stack for a user that doesn't exist, so it can't
    /// count against anyone's failed logins. `cargo test -- --ignored`.
    #[test]
    #[ignore = "talks to the system's PAM"]
    fn unknown_user_is_refused() {
        let r = authenticate("nordlys-no-such-user", Secret::new("x".into()));
        assert!(r.is_err(), "{r:?}");
    }
}
