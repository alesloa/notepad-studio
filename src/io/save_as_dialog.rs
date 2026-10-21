use std::path::{Path, PathBuf};

pub(crate) fn pick_rename(path: &Path) -> impl std::future::Future<Output = Option<PathBuf>> {
    pick_with_title(path.parent().map(Path::to_path_buf),
        path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(), "Rename")
}

// Capture the request before dispatch: test overrides are scoped to the caller,
// while the native dialog and its reply run after that caller returns.
// Keep that capture outside the future even when the native build omits it.
#[cfg_attr(not(test), allow(clippy::manual_async_fn))]
pub(super) fn pick(starting: Option<PathBuf>, suggested: String) -> impl std::future::Future<Output = Option<PathBuf>> {
    pick_with_title(starting, suggested, "Save As")
}

#[cfg_attr(not(test), allow(clippy::manual_async_fn))]
pub(super) fn pick_with_title(starting: Option<PathBuf>, suggested: String, title: &'static str) -> impl std::future::Future<Output = Option<PathBuf>> {
    pick_filtered(starting, suggested, title, None)
}

pub(crate) fn pick_docx(starting: Option<PathBuf>, suggested: String) -> impl std::future::Future<Output = Option<PathBuf>> {
    pick_filtered(starting, suggested, "Save As .docx", Some("docx"))
}

#[cfg_attr(not(test), allow(clippy::manual_async_fn))]
fn pick_filtered(starting: Option<PathBuf>, suggested: String, title: &'static str, extension: Option<&'static str>) -> impl std::future::Future<Output = Option<PathBuf>> {
    #[cfg(test)]
    let reply = REPLY.with(|slot| slot.borrow_mut().take());
    #[cfg(test)]
    REQUESTS.with(|slot| { if let Some(requests) = slot.borrow_mut().as_mut() { requests.push((title, suggested.clone())); } });
    async move {
        #[cfg(test)]
        if let Some(reply) = reply { return reply.await.ok().flatten(); }
        let mut dialog = rfd::AsyncFileDialog::new().set_title(title).set_file_name(&suggested);
        if let Some(directory) = starting { dialog = dialog.set_directory(directory); }
        if let Some(extension) = extension { dialog = dialog.add_filter("Word Document (.docx)", &[extension]); }
        dialog.save_file().await.map(|file| file.path().to_path_buf())
    }
}

#[cfg(test)]
thread_local! {
    static REPLY: std::cell::RefCell<Option<futures::channel::oneshot::Receiver<Option<PathBuf>>>> = const { std::cell::RefCell::new(None) };
    static REQUESTS: std::cell::RefCell<Option<Vec<(&'static str, String)>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct RequestCapture(Option<Vec<(&'static str, String)>>);
#[cfg(test)]
impl RequestCapture {
    pub(crate) fn new() -> Self { Self(REQUESTS.with(|slot| slot.replace(Some(Vec::new())))) }
    pub(crate) fn requests(&self) -> Vec<(&'static str, String)> { REQUESTS.with(|slot| slot.borrow().clone().unwrap_or_default()) }
}
#[cfg(test)]
impl Drop for RequestCapture { fn drop(&mut self) { REQUESTS.with(|slot| slot.replace(self.0.take())); } }

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
