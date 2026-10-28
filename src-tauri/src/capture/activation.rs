//! Bounded foreground activation attempts. Call from a worker so the UI can process input.
use std::time::Duration;

const ATTEMPTS: usize = 32;
const INTERVAL: Duration = Duration::from_millis(16);

pub(super) fn wait_for_activation(
    mut attempt: impl FnMut(bool) -> Result<bool, String>,
) -> Result<(), String> {
    for index in 0..ATTEMPTS {
        if attempt(index == 0)? {
            return Ok(());
        }
        if index + 1 < ATTEMPTS {
            // Never sleep inside the UI callback or hold the session lock here.
            std::thread::sleep(INTERVAL);
        }
    }
    Err("无法激活截图窗口，请重新触发截图".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_pending_foreground_transfer_can_complete_on_a_later_ui_turn() {
        let mut calls = Vec::new();
        wait_for_activation(|initial| {
            calls.push(initial);
            Ok(calls.len() == 3)
        })
        .unwrap();
        assert_eq!(calls, [true, false, false]);
    }

    #[test]
    fn an_already_active_surface_is_not_retried() {
        let mut calls = 0;
        wait_for_activation(|_| {
            calls += 1;
            Ok(true)
        })
        .unwrap();
        assert_eq!(calls, 1);
    }

    #[test]
    fn cancellation_between_attempts_prevents_any_later_reveal() {
        let mut calls = 0;
        let error = wait_for_activation(|_| {
            calls += 1;
            match calls {
                1 => Ok(false),
                2 => Err("截图会话已结束".into()),
                _ => panic!("cancelled capture was activated again"),
            }
        })
        .unwrap_err();
        assert_eq!(error, "截图会话已结束");
        assert_eq!(calls, 2);
    }

    #[test]
    fn native_window_errors_are_returned_without_retrying() {
        let mut calls = 0;
        let error = wait_for_activation(|_| {
            calls += 1;
            Err("窗口已销毁".into())
        })
        .unwrap_err();
        assert_eq!(error, "窗口已销毁");
        assert_eq!(calls, 1);
    }

    #[test]
    fn denied_activation_has_a_finite_retry_budget() {
        let mut calls = 0;
        let error = wait_for_activation(|_| {
            calls += 1;
            Ok(false)
        })
        .unwrap_err();
        assert_eq!(calls, ATTEMPTS);
        assert_eq!(error, "无法激活截图窗口，请重新触发截图");
    }
}
