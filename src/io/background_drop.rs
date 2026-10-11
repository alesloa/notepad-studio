use gpui::BackgroundExecutor;

// A window task can end on the foreground executor. Large snapshots and owned
// staging files must still be released on the background executor in that case.
pub(crate) struct BackgroundDrop<T: Send + 'static> { value: Option<T>, executor: BackgroundExecutor }
impl<T: Send + 'static> BackgroundDrop<T> {
    pub(crate) fn new(value: T, executor: BackgroundExecutor) -> Self { Self { value: Some(value), executor } }
    pub(crate) fn as_ref(&self) -> Option<&T> { self.value.as_ref() }
    pub(crate) fn as_mut(&mut self) -> Option<&mut T> { self.value.as_mut() }
    pub(crate) fn take(&mut self) -> Option<T> { self.value.take() }
}
impl<T: Send + 'static> Drop for BackgroundDrop<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() { self.executor.spawn(async move { drop(value); }).detach(); }
    }
}
