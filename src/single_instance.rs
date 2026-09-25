use anyhow::{Context, Result};

#[cfg(target_os = "windows")]
pub(super) fn try_lock(app_id: &str) -> Result<Option<std::os::windows::io::OwnedHandle>> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = format!(r"Local\{app_id}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }
        .context("创建 Windows 单实例互斥体失败")?;
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let guard = unsafe { OwnedHandle::from_raw_handle(handle.0) };

    if already_exists {
        return Ok(None);
    }

    Ok(Some(guard))
}

#[cfg(not(target_os = "windows"))]
pub(super) fn try_lock(_app_id: &str) -> Result<Option<std::net::TcpListener>> {
    match std::net::TcpListener::bind("127.0.0.1:57832") {
        Ok(listener) => Ok(Some(listener)),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Ok(None),
        Err(error) => Err(error).context("绑定单实例端口失败"),
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::try_lock;
    use std::process::Command;

    fn test_app_id(name: &str) -> String {
        format!(
            "com.dongdong.screenhop.test.{}.{}",
            std::process::id(),
            name
        )
    }

    #[test]
    fn single_instance_lock_does_not_depend_on_tcp_port() {
        let _listener = std::net::TcpListener::bind("127.0.0.1:57832").ok();
        let lock = try_lock(&test_app_id("port_independence")).unwrap();

        assert!(lock.is_some());
    }

    #[test]
    fn lock_rejects_duplicates_and_releases_on_drop() {
        let app_id = test_app_id("lifetime");
        let first = try_lock(&app_id).unwrap().unwrap();

        assert!(try_lock(&app_id).unwrap().is_none());
        drop(first);
        assert!(try_lock(&app_id).unwrap().is_some());
    }

    #[test]
    fn invalid_mutex_name_is_an_error() {
        assert!(try_lock(r"invalid\name").is_err());
    }

    #[test]
    fn lock_excludes_other_processes_until_released() {
        let app_id = test_app_id("cross_process");
        let first = try_lock(&app_id).unwrap().unwrap();

        run_lock_probe(&app_id, false);
        drop(first);
        run_lock_probe(&app_id, true);
    }

    fn run_lock_probe(app_id: &str, expected_acquired: bool) {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "single_instance::tests::lock_probe",
                "--nocapture",
            ])
            .env("SCREENHOP_TEST_MUTEX_ID", app_id)
            .env(
                "SCREENHOP_TEST_MUTEX_EXPECTED",
                expected_acquired.to_string(),
            )
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "lock probe failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn lock_probe() {
        let Ok(app_id) = std::env::var("SCREENHOP_TEST_MUTEX_ID") else {
            return;
        };
        let expected = std::env::var("SCREENHOP_TEST_MUTEX_EXPECTED")
            .unwrap()
            .parse::<bool>()
            .unwrap();

        assert_eq!(try_lock(&app_id).unwrap().is_some(), expected);
    }
}
