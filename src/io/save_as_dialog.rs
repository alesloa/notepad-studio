use std::path::PathBuf;

// Capture the request before dispatch: test overrides are scoped to the caller,
// while the native dialog and its reply run after that caller returns.
// Keep that capture outside the future even when the native build omits it.
#[cfg_attr(not(test), allow(clippy::manual_async_fn))]
pub(super) fn pick(starting: Option<PathBuf>, suggested: String) -> impl std::future::Future<Output = Option<PathBuf>> {
    #[cfg(test)]
    let reply = REPLY.with(|slot| slot.borrow_mut().take());
    async move {
        #[cfg(test)]
        if let Some(reply) = reply { return reply.await.ok().flatten(); }
        let mut dialog = rfd::AsyncFileDialog::new().set_title("Save As").set_file_name(&suggested);
        if let Some(directory) = starting { dialog = dialog.set_directory(directory); }
        dialog.save_file().await.map(|file| file.path().to_path_buf())
    }
}

#[cfg(test)]
thread_local! {
    static REPLY: std::cell::RefCell<Option<futures::channel::oneshot::Receiver<Option<PathBuf>>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct Override {
    previous: Option<futures::channel::oneshot::Receiver<Option<PathBuf>>>,
    _local: std::marker::PhantomData<std::rc::Rc<()>>,
}
#[cfg(test)]
impl Override {
    pub(crate) fn new(reply: futures::channel::oneshot::Receiver<Option<PathBuf>>) -> Self {
        Self { previous: REPLY.with(|slot| slot.replace(Some(reply))), _local: std::marker::PhantomData }
    }
}
#[cfg(test)]
impl Drop for Override {
    fn drop(&mut self) { REPLY.with(|slot| slot.replace(self.previous.take())); }
}
