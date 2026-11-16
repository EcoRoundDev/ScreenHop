use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use screenhop::config::AppConfig;
use screenhop_platform::macos::hook::MacMouseHook;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookState {
    NeedsPermission,
    Paused,
    Starting,
    Running,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookStatus {
    pub state: HookState,
    pub error: Option<String>,
}

trait Backend {
    fn trusted(&mut self) -> bool;
    fn installed(&self) -> bool;
    fn enabled(&self) -> bool;
    fn disabled_reason(&mut self) -> Option<u32>;
    fn install(&mut self) -> Result<(), String>;
    fn stop(&mut self);
    fn reenable(&mut self) -> Result<(), String>;
}

struct SystemBackend(MacMouseHook);

impl Backend for SystemBackend {
    fn trusted(&mut self) -> bool {
        unsafe {
            extern "C" {
                fn AXIsProcessTrusted() -> bool;
            }
            AXIsProcessTrusted()
        }
    }
    fn installed(&self) -> bool {
        self.0.is_installed()
    }
    fn enabled(&self) -> bool {
        self.0.is_enabled()
    }
    fn disabled_reason(&mut self) -> Option<u32> {
        self.0.take_disabled_reason()
    }
    fn install(&mut self) -> Result<(), String> {
        crate::app::install_macos_hook(&mut self.0).map_err(|error| format!("{error:#}"))
    }
    fn stop(&mut self) {
        self.0.stop();
    }
    fn reenable(&mut self) -> Result<(), String> {
        self.0.reenable().map_err(|error| format!("{error:#}"))
    }
}

struct Controller<B> {
    backend: B,
    status: HookStatus,
    trusted: Option<bool>,
    wanted: Option<bool>,
    next_check: Duration,
    waiting_since: Duration,
    retry_at: Option<Duration>,
    retry_count: usize,
    recoveries: VecDeque<Duration>,
    recovery_blocked: bool,
}

impl<B: Backend> Controller<B> {
    fn new(backend: B) -> Self {
        Self {
            backend,
            status: HookStatus {
                state: HookState::NeedsPermission,
                error: None,
            },
            trusted: None,
            wanted: None,
            next_check: Duration::ZERO,
            waiting_since: Duration::ZERO,
            retry_at: None,
            retry_count: 0,
            recoveries: VecDeque::new(),
            recovery_blocked: false,
        }
    }

    fn set_state(&mut self, state: HookState, error: Option<String>) {
        let status = HookStatus { state, error };
        if self.status != status {
            log::info!("macOS 中键运行状态: {:?}, {:?}", status.state, status.error);
            self.status = status;
        }
    }

    fn reset_retry(&mut self) {
        self.retry_count = 0;
        self.retry_at = None;
        self.recovery_blocked = false;
    }

    fn fail_install(&mut self, now: Duration, error: String) {
        self.backend.stop();
        self.retry_at = [1, 2, 5]
            .get(self.retry_count)
            .map(|delay| now + Duration::from_secs(*delay));
        self.retry_count += 1;
        self.set_state(HookState::Failed, Some(error));
    }

    fn reconcile(&mut self, wanted: bool, force: bool, now: Duration) -> HookStatus {
        let changed = self.wanted != Some(wanted);
        self.wanted = Some(wanted);
        if force || changed {
            self.reset_retry();
            self.recoveries.clear();
        }
        let disabled = self.backend.disabled_reason();
        let retry_due = self.retry_at.is_some_and(|deadline| now >= deadline);
        let due = force || changed || disabled.is_some() || retry_due || now >= self.next_check;
        if due {
            let trusted = self.backend.trusted();
            if self.trusted != Some(trusted) {
                self.reset_retry();
                self.recoveries.clear();
                if !trusted {
                    self.waiting_since = now;
                }
            }
            self.trusted = Some(trusted);
            let delay =
                if !trusted && now.saturating_sub(self.waiting_since) < Duration::from_secs(120) {
                    1
                } else {
                    5
                };
            self.next_check = now + Duration::from_secs(delay);
        }
        if self.trusted != Some(true) {
            self.backend.stop();
            self.set_state(HookState::NeedsPermission, None);
            return self.status.clone();
        }
        if !wanted {
            self.backend.stop();
            self.set_state(HookState::Paused, None);
            return self.status.clone();
        }
        if self.recovery_blocked {
            return self.status.clone();
        }
        if !due && (self.backend.installed() || self.status.state == HookState::Running) {
            return self.status.clone();
        }
        if (self.backend.installed() || self.status.state == HookState::Running)
            && due
            && (disabled.is_some() || !self.backend.enabled())
        {
            while self
                .recoveries
                .front()
                .is_some_and(|time| now.saturating_sub(*time) >= Duration::from_secs(60))
            {
                self.recoveries.pop_front();
            }
            if self.recoveries.len() >= 3 {
                self.backend.stop();
                self.retry_at = None;
                self.recovery_blocked = true;
                self.set_state(
                    HookState::Failed,
                    Some("鼠标钩子在一分钟内频繁停用，请手动重试".into()),
                );
                return self.status.clone();
            }
            self.recoveries.push_back(now);
            if self.backend.reenable().is_err() {
                self.backend.stop();
                self.retry_at = Some(now);
            }
        }
        if self.backend.installed() && self.backend.enabled() {
            self.set_state(HookState::Running, None);
            return self.status.clone();
        }
        let retry_due = self.retry_at.is_some_and(|deadline| now >= deadline);
        if self.status.state == HookState::Failed && !retry_due && !force && !changed {
            return self.status.clone();
        }
        self.retry_at = None;
        self.backend.stop();
        self.set_state(HookState::Starting, None);
        match self.backend.install() {
            Ok(()) if self.backend.installed() && self.backend.enabled() => {
                self.retry_count = 0;
                self.set_state(HookState::Running, None);
            }
            Ok(()) => self.fail_install(now, "鼠标钩子安装后未处于运行状态".into()),
            Err(error) => self.fail_install(now, error),
        }
        self.status.clone()
    }
}

struct Runtime {
    started: Instant,
    controller: Controller<SystemBackend>,
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

pub fn reconcile(config: &AppConfig, force: bool) -> HookStatus {
    crate::app::update_hook_config(config);
    RUNTIME.with(|slot| {
        let mut slot = slot.borrow_mut();
        let runtime = slot.get_or_insert_with(|| Runtime {
            started: Instant::now(),
            controller: Controller::new(SystemBackend(MacMouseHook::new())),
        });
        runtime
            .controller
            .reconcile(!config.disable_hook, force, runtime.started.elapsed())
    })
}

pub fn status() -> HookStatus {
    RUNTIME.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|runtime| runtime.controller.status.clone())
            .unwrap_or(HookStatus {
                state: HookState::NeedsPermission,
                error: None,
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        trusted: bool,
        installed: bool,
        enabled: bool,
        reason: Option<u32>,
        install_failures: usize,
        reenable_fails: bool,
        installs: usize,
        trust_checks: usize,
        recoveries: usize,
        stops: usize,
    }

    impl Backend for FakeBackend {
        fn trusted(&mut self) -> bool {
            self.trust_checks += 1;
            self.trusted
        }
        fn installed(&self) -> bool {
            self.installed
        }
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn disabled_reason(&mut self) -> Option<u32> {
            self.reason.take()
        }
        fn install(&mut self) -> Result<(), String> {
            assert!(!self.installed);
            self.installs += 1;
            if self.install_failures > 0 {
                self.install_failures -= 1;
                return Err("installation failed".into());
            }
            self.installed = true;
            self.enabled = true;
            Ok(())
        }
        fn stop(&mut self) {
            if self.installed {
                self.stops += 1;
            }
            self.installed = false;
            self.enabled = false;
        }
        fn reenable(&mut self) -> Result<(), String> {
            self.recoveries += 1;
            if self.reenable_fails {
                return Err("callback panicked".into());
            }
            self.enabled = true;
            Ok(())
        }
    }

    fn second(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn permission_grant_installs_once_without_restarting() {
        let mut c = Controller::new(FakeBackend::default());
        assert_eq!(
            c.reconcile(true, false, second(0)).state,
            HookState::NeedsPermission
        );
        c.backend.trusted = true;
        c.reconcile(true, false, Duration::from_millis(250));
        assert_eq!(c.backend.trust_checks, 1);
        assert_eq!(
            c.reconcile(true, false, second(1)).state,
            HookState::Running
        );
        c.reconcile(true, false, second(2));
        assert_eq!(c.backend.installs, 1);
    }

    #[test]
    fn permission_grant_preserves_pause_and_enable_is_immediate() {
        let mut c = Controller::new(FakeBackend::default());
        c.reconcile(false, false, second(0));
        c.backend.trusted = true;
        assert_eq!(
            c.reconcile(false, false, second(1)).state,
            HookState::Paused
        );
        assert_eq!(c.backend.installs, 0);
        assert_eq!(
            c.reconcile(true, false, Duration::from_millis(1250)).state,
            HookState::Running
        );
    }

    #[test]
    fn retry_deadlines_are_bounded_and_force_restarts_attempts() {
        let mut c = Controller::new(FakeBackend {
            trusted: true,
            install_failures: 10,
            ..Default::default()
        });
        c.reconcile(true, false, second(0));
        c.reconcile(true, false, Duration::from_millis(750));
        assert_eq!(c.backend.installs, 1);
        c.reconcile(true, false, second(1));
        c.reconcile(true, false, second(2));
        assert_eq!(c.backend.installs, 2);
        c.reconcile(true, false, second(3));
        c.reconcile(true, false, second(7));
        assert_eq!(c.backend.installs, 3);
        c.reconcile(true, false, second(8));
        c.reconcile(true, false, second(100));
        assert_eq!(c.backend.installs, 4);
        c.reconcile(true, true, second(101));
        assert_eq!(c.backend.installs, 5);
    }

    #[test]
    fn revoked_permission_stops_hook_then_regrant_recovers() {
        let mut c = Controller::new(FakeBackend {
            trusted: true,
            ..Default::default()
        });
        c.reconcile(true, false, second(0));
        c.backend.trusted = false;
        assert_eq!(
            c.reconcile(true, false, second(5)).state,
            HookState::NeedsPermission
        );
        assert_eq!(c.backend.stops, 1);
        c.backend.trusted = true;
        assert_eq!(
            c.reconcile(true, false, second(6)).state,
            HookState::Running
        );
        assert_eq!(c.backend.installs, 2);
    }

    #[test]
    fn repeated_disable_is_latched_until_manual_action() {
        let mut c = Controller::new(FakeBackend {
            trusted: true,
            ..Default::default()
        });
        c.reconcile(true, false, second(0));
        for time in 1..=3 {
            c.backend.reason = Some(0xFFFFFFFE);
            c.backend.enabled = false;
            assert_eq!(
                c.reconcile(true, false, second(time)).state,
                HookState::Running
            );
        }
        c.backend.reason = Some(0xFFFFFFFF);
        c.backend.enabled = false;
        assert_eq!(c.reconcile(true, false, second(4)).state, HookState::Failed);
        c.reconcile(true, false, second(100));
        assert_eq!(c.backend.installs, 1);
        assert_eq!(c.backend.recoveries, 3);
        assert_eq!(
            c.reconcile(true, true, second(101)).state,
            HookState::Running
        );
    }

    #[test]
    fn broken_callback_stops_before_reinstallation() {
        let mut c = Controller::new(FakeBackend {
            trusted: true,
            reenable_fails: true,
            ..Default::default()
        });
        c.reconcile(true, false, second(0));
        c.backend.reason = Some(0xFFFFFFFE);
        c.backend.enabled = false;
        assert_eq!(
            c.reconcile(true, false, second(1)).state,
            HookState::Running
        );
        assert_eq!(c.backend.stops, 1);
        assert_eq!(c.backend.installs, 2);
    }

    #[test]
    fn force_checks_permission_without_reinstalling_healthy_hook() {
        let mut c = Controller::new(FakeBackend {
            trusted: true,
            ..Default::default()
        });
        c.reconcile(true, false, second(0));
        c.reconcile(true, true, Duration::from_millis(250));
        assert_eq!(c.backend.installs, 1);
        assert_eq!(c.backend.stops, 0);
        assert_eq!(c.backend.trust_checks, 2);
    }

    #[test]
    fn unhealthy_hook_waits_for_health_deadline_without_unbounded_installs() {
        let mut c = Controller::new(FakeBackend {
            trusted: true,
            ..Default::default()
        });
        c.reconcile(true, false, second(0));
        c.backend.enabled = false;
        c.reconcile(true, false, Duration::from_millis(250));
        assert_eq!(c.backend.installs, 1);
        assert_eq!(c.backend.recoveries, 0);
        c.reconcile(true, false, second(5));
        assert_eq!(c.backend.recoveries, 1);
        assert_eq!(c.backend.installs, 1);
    }

    #[test]
    fn retry_checks_permission_before_attempting_install() {
        let mut c = Controller::new(FakeBackend {
            trusted: true,
            install_failures: 1,
            ..Default::default()
        });
        c.reconcile(true, false, second(0));
        c.backend.trusted = false;
        assert_eq!(
            c.reconcile(true, false, second(1)).state,
            HookState::NeedsPermission
        );
        assert_eq!(c.backend.installs, 1);
    }

    #[test]
    fn permission_polling_slows_after_two_minutes() {
        let mut c = Controller::new(FakeBackend::default());
        c.reconcile(true, false, second(0));
        c.reconcile(true, false, second(120));
        assert_eq!(c.next_check, second(125));
        c.backend.trusted = true;
        assert_eq!(
            c.reconcile(true, true, second(121)).state,
            HookState::Running
        );
    }
}

pub fn shutdown() {
    RUNTIME.with(|slot| {
        if let Some(mut runtime) = slot.borrow_mut().take() {
            runtime.controller.backend.stop();
        }
    });
}
