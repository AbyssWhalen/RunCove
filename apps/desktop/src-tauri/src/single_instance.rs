use crate::error::{invalid, AppResult};

#[cfg(windows)]
pub struct SingleInstanceGuard {
    mutex: usize,
    wake_event: usize,
    listener_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    listener: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(windows))]
pub struct SingleInstanceGuard;

impl SingleInstanceGuard {
    pub fn acquire(name: &str) -> AppResult<Self> {
        #[cfg(windows)]
        {
            Self::acquire_windows(name, true)
        }

        #[cfg(not(windows))]
        {
            let _ = name;
            Ok(Self)
        }
    }

    pub fn acquire_after_previous(name: &str, timeout: std::time::Duration) -> AppResult<Self> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            #[cfg(windows)]
            let attempt = Self::acquire_windows(name, false);
            #[cfg(not(windows))]
            let attempt = Self::acquire(name);

            match attempt {
                Ok(guard) => return Ok(guard),
                Err(error)
                    if error.to_string() == "RunCove is already running"
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(error) => return Err(error),
            }
        }
    }

    #[cfg(windows)]
    fn acquire_windows(name: &str, notify_existing: bool) -> AppResult<Self> {
        use std::sync::atomic::AtomicBool;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent};

        let wide_mutex_name = wide_name(name);
        let mutex = unsafe { CreateMutexW(None, false, PCWSTR(wide_mutex_name.as_ptr())) }
            .map_err(|error| invalid(format!("Could not create instance guard: {error}")))?;
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;

        let wake_name = format!("{name}.Wake");
        let wide_wake_name = wide_name(&wake_name);
        let wake_event =
            match unsafe { CreateEventW(None, false, false, PCWSTR(wide_wake_name.as_ptr())) } {
                Ok(handle) => handle,
                Err(error) => {
                    let _ = unsafe { CloseHandle(mutex) };
                    return Err(invalid(format!(
                        "Could not create instance wake event: {error}"
                    )));
                }
            };

        if already_exists {
            let notify_result = if notify_existing {
                unsafe { SetEvent(wake_event) }.map_err(|error| {
                    invalid(format!(
                        "Could not wake the running RunCove window: {error}"
                    ))
                })
            } else {
                Ok(())
            };
            let _ = unsafe { CloseHandle(wake_event) };
            let _ = unsafe { CloseHandle(mutex) };
            notify_result?;
            return Err(invalid("RunCove is already running"));
        }

        Ok(Self {
            mutex: mutex.0 as usize,
            wake_event: wake_event.0 as usize,
            listener_stop: std::sync::Arc::new(AtomicBool::new(false)),
            listener: None,
        })
    }

    #[cfg(windows)]
    pub fn start_wake_listener(&mut self, wake: impl Fn() + Send + 'static) -> AppResult<()> {
        use std::sync::atomic::Ordering;
        use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};

        if self.listener.is_some() {
            return Err(invalid("RunCove instance wake listener is already running"));
        }

        let event = self.wake_event;
        let stop = self.listener_stop.clone();
        self.listener = Some(
            std::thread::Builder::new()
                .name("runcove-instance-wake".into())
                .spawn(move || loop {
                    let result = unsafe { WaitForSingleObject(HANDLE(event as *mut _), INFINITE) };
                    if result != WAIT_OBJECT_0 || stop.load(Ordering::Acquire) {
                        break;
                    }
                    wake();
                })
                .map_err(|error| {
                    invalid(format!("Could not start instance wake listener: {error}"))
                })?,
        );
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn start_wake_listener(&mut self, _wake: impl Fn() + Send + 'static) -> AppResult<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn wide_name(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::Threading::SetEvent;

        self.listener_stop.store(true, Ordering::Release);
        let _ = unsafe { SetEvent(HANDLE(self.wake_event as *mut _)) };
        if let Some(listener) = self.listener.take() {
            let _ = listener.join();
        }
        let _ = unsafe { CloseHandle(HANDLE(self.wake_event as *mut _)) };
        let _ = unsafe { CloseHandle(HANDLE(self.mutex as *mut _)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_second_guard_with_the_same_name_is_rejected_until_release() {
        let name = format!(
            r"Local\RunCove.Test.{}.{}",
            std::process::id(),
            crate::storage::now_ms()
        );
        let mut first = SingleInstanceGuard::acquire(&name).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        first
            .start_wake_listener(move || sender.send(()).unwrap())
            .unwrap();

        assert!(SingleInstanceGuard::acquire(&name).is_err());
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("the existing instance should receive a wake request");

        drop(first);
        assert!(SingleInstanceGuard::acquire(&name).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn elevated_relaunch_waits_for_the_previous_guard() {
        let name = format!(
            r"Local\RunCove.Test.Wait.{}.{}",
            std::process::id(),
            crate::storage::now_ms()
        );
        let first = SingleInstanceGuard::acquire(&name).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(first);
        });

        let next =
            SingleInstanceGuard::acquire_after_previous(&name, std::time::Duration::from_secs(2))
                .unwrap();

        releaser.join().unwrap();
        drop(next);
    }

    #[cfg(windows)]
    #[test]
    fn elevated_relaunch_wait_does_not_wake_the_previous_window() {
        let name = format!(
            r"Local\RunCove.Test.NoWake.{}.{}",
            std::process::id(),
            crate::storage::now_ms()
        );
        let mut first = SingleInstanceGuard::acquire(&name).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        first
            .start_wake_listener(move || sender.send(()).unwrap())
            .unwrap();

        assert!(SingleInstanceGuard::acquire_after_previous(
            &name,
            std::time::Duration::from_millis(150)
        )
        .is_err());
        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
    }
}
