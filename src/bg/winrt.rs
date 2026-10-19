//! Вспомогательные функции для синхронного ожидания WinRT-операций.

use std::sync::mpsc::channel;

use crate::log;

/// Ждёт асинхронную операцию WinRT не дольше `WAIT_MS`: зависший драйвер/плеер не должен
/// вешать командный поток (иначе встают и переключатели, и громкость).
pub(super) fn wait<T: windows::core::RuntimeType + 'static>(
    op: windows::core::Result<windows_future::IAsyncOperation<T>>,
) -> windows::core::Result<T> {
    use windows_future::{AsyncOperationCompletedHandler, IAsyncOperation};
    const WAIT_MS: u64 = 5000;
    let op = op?;
    let (tx, rx) = channel::<Handoff<windows::core::Result<T>>>();
    op.SetCompleted(&AsyncOperationCompletedHandler::new(move |op: windows::core::Ref<IAsyncOperation<T>>, _| {
        if let Some(op) = op.as_ref() {
            let _ = tx.send(Handoff(op.GetResults()));
        }
        Ok(())
    }))?;
    match rx.recv_timeout(std::time::Duration::from_millis(WAIT_MS)) {
        Ok(r) => r.0,
        Err(_) => {
            log::error("WinRT: операция не завершилась за 5 с");
            Err(windows::core::Error::from(windows::core::HRESULT::from_win32(1460)))
        }
    }
}

/// Результат WinRT-операции переходит из потока обратного вызова в командный поток. Все такие
/// объекты — free-threaded (агильные), но windows-rs не помечает их `Send`.
struct Handoff<T>(T);
unsafe impl<T> Send for Handoff<T> {}
