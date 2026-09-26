//! Who the process runs as, according to the operating system.
//!
//! On Windows this asks the domain-join status and can produce a Kerberos
//! `Negotiate` token through SSPI for the marketplace SPN. Everywhere else it
//! reports the login name so a development server can trust it.

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Each platform constructs a different subset of variants.
pub(crate) enum JoinState {
    /// Joined to an Active Directory domain with the given name.
    Domain(String),
    /// A workgroup or standalone machine.
    Workgroup,
    /// Not a Windows host, so the question does not apply.
    NotApplicable,
}

#[derive(Clone, Debug)]
pub(crate) struct HostIdentity {
    /// `DOMAIN\user` on Windows, the login name elsewhere.
    pub(crate) account: String,
    pub(crate) join: JoinState,
}

pub(crate) fn current() -> HostIdentity {
    HostIdentity {
        account: account_name(),
        join: join_state(),
    }
}

fn account_name() -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string());
    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.trim().is_empty() => format!("{}\\{user}", domain.trim()),
        _ => user,
    }
}

/// The sentence for an SSPI sign-in that could not reach a domain controller:
/// the network is down, so sync treats it like any unreachable server.
pub(crate) const NO_DOMAIN_CONTROLLER: &str = "Could not reach a domain controller to sign in. Connect to the corporate network or VPN, then try again.";

/// A plain sentence for the SSPI status codes people actually hit when
/// Windows cannot get a Kerberos ticket for the marketplace.
#[cfg_attr(not(windows), allow(dead_code))]
fn sspi_sentence(status: i32) -> Option<&'static str> {
    // SSPI statuses are HRESULTs; compare their bit patterns.
    Some(match status as u32 {
        0x8009_030C => "The domain refused this Windows account's sign-in. Sign out of Windows and back in, then try again.",
        0x8009_030E => "Windows has no Kerberos sign-in for this account. Lock and unlock the PC, or connect to the corporate network or VPN, then try again.",
        0x8009_0303 | 0x8009_0322 => "Windows does not recognize the marketplace server's name (HTTP service principal). Ask the marketplace administrator to check its registration.",
        0x8009_0311 => NO_DOMAIN_CONTROLLER,
        0x8009_0324 => "This PC's clock is too far from the domain's. Turn on \"Set time automatically\" in Windows Settings, then try again.",
        _ => return None,
    })
}

#[cfg(not(windows))]
fn join_state() -> JoinState {
    JoinState::NotApplicable
}

/// A Kerberos `Negotiate` token for `HTTP/<host>`, base64 encoded, when the
/// platform can produce one. `None` on non-Windows hosts.
#[cfg(not(windows))]
pub(crate) fn negotiate_token(_host: &str) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(not(windows))]
pub(crate) fn free_disk_bytes(_path: &std::path::Path) -> Option<u64> {
    None
}

#[cfg(not(windows))]
pub(crate) fn long_paths_enabled() -> Option<bool> {
    None
}

#[cfg(not(windows))]
pub(crate) fn attach_parent_console() {}

#[cfg(windows)]
fn join_state() -> JoinState {
    windows::join_state()
}

#[cfg(windows)]
pub(crate) fn negotiate_token(host: &str) -> Result<Option<String>, String> {
    windows::negotiate_token(host).map(Some)
}

#[cfg(windows)]
pub(crate) fn free_disk_bytes(path: &std::path::Path) -> Option<u64> {
    windows::free_disk_bytes(path)
}

#[cfg(windows)]
pub(crate) fn long_paths_enabled() -> Option<bool> {
    winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\FileSystem")
        .ok()
        .and_then(|key| key.get_value::<u32, _>("LongPathsEnabled").ok())
        .map(|value| value != 0)
}

/// A release build hides its console window; a CLI invocation reattaches to
/// the parent console so its output is visible.
#[cfg(windows)]
pub(crate) fn attach_parent_console() {
    windows::attach_parent_console();
}

#[cfg(windows)]
mod windows {
    use super::JoinState;
    use base64::Engine as _;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;
    use std::ptr;
    use windows_sys::Win32::Foundation::{SEC_E_OK, SEC_I_CONTINUE_NEEDED};
    use windows_sys::Win32::NetworkManagement::NetManagement::{
        NetApiBufferFree, NetGetJoinInformation, NetSetupDomainName,
    };
    use windows_sys::Win32::Security::Authentication::Identity::{
        AcquireCredentialsHandleW, DeleteSecurityContext, FreeContextBuffer, FreeCredentialsHandle,
        InitializeSecurityContextW, SecBuffer, SecBufferDesc, ISC_REQ_ALLOCATE_MEMORY,
        ISC_REQ_MUTUAL_AUTH, SECBUFFER_TOKEN, SECBUFFER_VERSION, SECPKG_CRED_OUTBOUND,
        SECURITY_NATIVE_DREP,
    };
    use windows_sys::Win32::Security::Credentials::SecHandle;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn join_state() -> JoinState {
        let mut name: *mut u16 = ptr::null_mut();
        let mut status: i32 = 0;
        // SAFETY: NetGetJoinInformation writes an allocated buffer to `name`
        // and a status to `status`; the buffer is released with
        // NetApiBufferFree before returning.
        let result = unsafe { NetGetJoinInformation(ptr::null(), &mut name, &mut status) };
        if result != 0 {
            return match std::env::var("USERDNSDOMAIN") {
                Ok(domain) if !domain.trim().is_empty() => JoinState::Domain(domain),
                _ => JoinState::Workgroup,
            };
        }
        let domain = if name.is_null() {
            String::new()
        } else {
            // SAFETY: `name` is a NUL-terminated wide string from the API.
            let mut length = 0usize;
            while unsafe { *name.add(length) } != 0 {
                length += 1;
            }
            let slice = unsafe { std::slice::from_raw_parts(name, length) };
            let text = String::from_utf16_lossy(slice);
            unsafe { NetApiBufferFree(name.cast()) };
            text
        };
        if status == NetSetupDomainName {
            JoinState::Domain(domain)
        } else {
            JoinState::Workgroup
        }
    }

    pub(super) fn negotiate_token(host: &str) -> Result<String, String> {
        let package = wide("Negotiate");
        let target = wide(&format!("HTTP/{host}"));
        let mut credentials = SecHandle {
            dwLower: 0,
            dwUpper: 0,
        };
        let mut expiry = 0i64;
        // SAFETY: every pointer refers to a live local; the credential handle
        // is freed before returning on every path.
        let acquired = unsafe {
            AcquireCredentialsHandleW(
                ptr::null(),
                package.as_ptr(),
                SECPKG_CRED_OUTBOUND,
                ptr::null(),
                ptr::null(),
                None,
                ptr::null(),
                &mut credentials,
                &mut expiry,
            )
        };
        if acquired != SEC_E_OK {
            return Err(super::sspi_sentence(acquired).map_or_else(
                || format!("AcquireCredentialsHandle(Negotiate) failed with 0x{acquired:08x}."),
                |sentence| format!("{sentence} (Windows code 0x{acquired:08x})"),
            ));
        }
        let mut buffer = SecBuffer {
            cbBuffer: 0,
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: ptr::null_mut(),
        };
        let mut output = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &mut buffer,
        };
        let mut context = SecHandle {
            dwLower: 0,
            dwUpper: 0,
        };
        let mut attributes = 0u32;
        // SAFETY: as above; SSPI allocates the output token because of
        // ISC_REQ_ALLOCATE_MEMORY and it is released with FreeContextBuffer.
        let status = unsafe {
            InitializeSecurityContextW(
                &credentials,
                ptr::null(),
                target.as_ptr(),
                ISC_REQ_MUTUAL_AUTH | ISC_REQ_ALLOCATE_MEMORY,
                0,
                SECURITY_NATIVE_DREP,
                ptr::null(),
                0,
                &mut context,
                &mut output,
                &mut attributes,
                &mut expiry,
            )
        };
        let result = if status == SEC_E_OK || status == SEC_I_CONTINUE_NEEDED {
            if buffer.pvBuffer.is_null() || buffer.cbBuffer == 0 {
                Err("SSPI produced an empty Negotiate token.".to_string())
            } else {
                // SAFETY: SSPI wrote cbBuffer bytes at pvBuffer.
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        buffer.pvBuffer.cast::<u8>(),
                        buffer.cbBuffer as usize,
                    )
                };
                Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
            }
        } else {
            Err(super::sspi_sentence(status).map_or_else(
                || format!("InitializeSecurityContext for HTTP/{host} failed with 0x{status:08x}."),
                |sentence| format!("{sentence} (Windows code 0x{status:08x} for HTTP/{host})"),
            ))
        };
        // SAFETY: releases what the calls above allocated.
        unsafe {
            if !buffer.pvBuffer.is_null() {
                FreeContextBuffer(buffer.pvBuffer);
            }
            if status == SEC_E_OK || status == SEC_I_CONTINUE_NEEDED {
                DeleteSecurityContext(&context);
            }
            FreeCredentialsHandle(&credentials);
        }
        result
    }

    pub(super) fn free_disk_bytes(path: &Path) -> Option<u64> {
        let wide_path = wide(&path.display().to_string());
        let mut free = 0u64;
        // SAFETY: the path is NUL-terminated and `free` outlives the call.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide_path.as_ptr(),
                &mut free,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free)
    }

    pub(super) fn attach_parent_console() {
        // SAFETY: attaching to the parent console has no preconditions; failure
        // (no parent console) is harmless.
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn common_sspi_failures_read_as_sentences() {
        let skew = super::sspi_sentence(0x8009_0324_u32 as i32).expect("time skew");
        assert!(skew.contains("clock"));
        assert!(super::sspi_sentence(0x8009_0311_u32 as i32)
            .is_some_and(|sentence| sentence.contains("VPN")));
        assert_eq!(super::sspi_sentence(0x1234), None);
    }
}
