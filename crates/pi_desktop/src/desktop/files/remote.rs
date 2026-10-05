//! Remote file buffers have no local Zed File/Project. Only this SSH adapter can
//! read or save them. Failed saves keep edits and are never replayed automatically.
use super::*;
use anyhow::{Context as _, Result, ensure};
use pi_core::{
    remote_files::{Client, Document},
    ssh::SshTarget,
};
use std::{collections::HashMap, sync::Arc, time::Duration};

pub(super) struct Remote {
    target: SshTarget,
    client: Option<Arc<Client>>,
    revisions: HashMap<PathBuf, String>,
    polling: bool,
}
impl Remote {
    pub fn new(target: SshTarget) -> Self {
        Self {
            target,
            client: None,
            revisions: HashMap::new(),
            polling: false,
        }
    }
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }
    fn connect(&self) -> (SshTarget, Option<Arc<Client>>) {
        (self.target.clone(), self.client.clone())
    }
}
fn connected(target: SshTarget, client: Option<Arc<Client>>) -> Result<Arc<Client>> {
    client
        .map(Ok)
        .unwrap_or_else(|| Client::connect(&target).map(Arc::new))
}
impl FilesView {
    /// Read-only consumers (such as @ previews) use the file channel too. The
    /// caller must never substitute a desktop read when this is a remote view.
    pub fn read_remote(&self, path: String, cx: &App) -> Option<gpui::Task<Result<Document>>> {
        let (target, client) = self.remote.as_ref()?.connect();
        let name = self.remote_path(std::path::Path::new(&path));
        Some(cx.background_executor().spawn(async move {
            let name = name?;
            connected(target, client)?.read(name)
        }))
    }

    #[cfg(test)]
    pub(crate) fn use_remote_client(&mut self, client: Arc<Client>) {
        self.remote.as_mut().unwrap().client = Some(client);
    }

    fn remote_path(&self, path: &std::path::Path) -> Result<String> {
        let remote = self.remote.as_ref().context("Not a remote file view")?;
        if let Ok(relative) = path.strip_prefix(&self.root) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            ensure!(
                !relative.is_empty(),
                "Choose a file inside the remote project"
            );
            return Ok(relative);
        }
        let raw = path.to_string_lossy().into_owned();
        // Refuse a desktop absolute path before even opening an SSH connection.
        if path.is_absolute() {
            let root = remote.target.cwd.trim_end_matches('/');
            ensure!(
                raw.strip_prefix(root)
                    .is_some_and(|suffix| suffix.starts_with('/')),
                "File is outside the remote project"
            );
        }
        Ok(raw)
    }
    pub(super) fn load_remote_browser(&mut self, cx: &mut Context<Self>) {
        if self.browser_loading {
            return;
        }
        let (target, client) = self.remote.as_ref().unwrap().connect();
        self.browser_loading = true;
        let task = cx.background_executor().spawn(async move {
            let client = connected(target, client)?;
            let tree = client.list()?;
            Ok::<_, anyhow::Error>((client, tree))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.browser_loading = false;
                match result {
                    Ok((client, tree)) => {
                        this.remote.as_mut().unwrap().client = Some(client);
                        this.entries = tree.entries.into_iter().map(|entry| FileEntry {path:this.root.join(&entry.path), relative:entry.path, directory:entry.directory}).collect();
                        this.error = tree.truncated.then(|| "Remote tree truncated at its safety limit. Large generated folders are excluded.".into());
                        this.start_remote_poll(cx);
                        this.filter_rows(cx);
                    }
                    Err(error) => this.remote_error(error, cx),
                }
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }
    pub(super) fn open_remote(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.tabs.iter().any(|tab| tab.path == path) {
            self.select(path, cx);
            return;
        }
        let name = match self.remote_path(&path) {
            Ok(name) => name,
            Err(error) => {
                this_error(self, error, cx);
                return;
            }
        };
        if !self.opening.insert(path.clone()) {
            return;
        }
        let (target, client) = self.remote.as_ref().unwrap().connect();
        let task = cx.background_executor().spawn(async move {
            let client = connected(target, client)?;
            let document = client.read(name)?;
            Ok::<_, anyhow::Error>((client, document))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.opening.remove(&path);
                match result {
                    Ok((client, document)) => {
                        let path = this.root.join(&document.path);
                        match pi_editor::detached_buffer(
                            std::path::Path::new(&document.path),
                            &document.text,
                            cx,
                        ) {
                            Ok(buffer) => {
                                let remote = this.remote.as_mut().unwrap();
                                remote.client = Some(client);
                                remote.revisions.insert(path.clone(), document.revision);
                                this.error = None;
                                this.add_tab(path, buffer, cx);
                                this.start_remote_poll(cx);
                            }
                            Err(error) => this_error(this, error, cx),
                        }
                    }
                    Err(error) => this.remote_error(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    pub(super) fn save_remote(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.tab() else { return };
        let buffer = tab.buffer.clone();
        if buffer.read(cx).has_conflict() {
            return;
        }
        let path = tab.path.clone();
        let name = match self.remote_path(&path) {
            Ok(name) => name,
            Err(error) => {
                this_error(self, error, cx);
                return;
            }
        };
        let remote = self.remote.as_ref().unwrap();
        let Some(revision) = remote.revisions.get(&path).cloned() else {
            return;
        };
        let (target, client) = remote.connect();
        let version = buffer.read(cx).version();
        let mut text = buffer.read(cx).text();
        if buffer.read(cx).line_ending() == language::LineEnding::Windows {
            text = text.replace('\n', "\r\n");
        }
        self.saving = true;
        let task = cx.background_executor().spawn(async move {
            let client = connected(target, client)?;
            let document = client.save(name, text, revision)?;
            Ok::<_, anyhow::Error>((client, document))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this,cx| {
                this.saving = false;
                match result {
                    Ok((client, doc)) => {
                        let remote = this.remote.as_mut().unwrap();
                        remote.client = Some(client); remote.revisions.insert(path, doc.revision);
                        buffer.update(cx, |buffer,cx| buffer.did_save(version, None, cx));
                        this.error = None; this.start_remote_poll(cx);
                    }
                    Err(error) => {
                        // An acknowledgement may have been lost after writing. Keep dirty
                        // state and force an explicit reload/check before any retry.
                        buffer.update(cx, |buffer,cx| {buffer.set_conflict();cx.emit(BufferEvent::DirtyChanged);cx.notify();});
                        this.remote_error(error.context("Save failed; edits kept. Check/reload the remote file before retrying"),cx);
                    }
                }
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }
    pub(super) fn reload_remote(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(buffer) = self
            .tabs
            .iter()
            .find(|tab| tab.path == path)
            .map(|tab| tab.buffer.clone())
        else {
            return;
        };
        let name = match self.remote_path(&path) {
            Ok(name) => name,
            Err(error) => {
                this_error(self, error, cx);
                return;
            }
        };
        let version = buffer.read(cx).version();
        let (target, client) = self.remote.as_ref().unwrap().connect();
        self.opening.insert(path.clone());
        let task = cx.background_executor().spawn(async move {
            let client = connected(target, client)?;
            let doc = client.read(name)?;
            Ok::<_, anyhow::Error>((client, doc))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.opening.remove(&path);
                match result {
                    Ok((client, doc)) if buffer.read(cx).version() == version => {
                        this.remote.as_mut().unwrap().client = Some(client);
                        this.apply_remote_document(path, buffer, doc, cx);
                        this.error = None;
                        this.start_remote_poll(cx);
                    }
                    Ok(_) => this.error = Some(
                        "Buffer edited while reloading; edits kept. Reload again to discard them."
                            .into(),
                    ),
                    Err(error) => this.remote_error(error, cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn apply_remote_document(
        &mut self,
        path: PathBuf,
        buffer: Entity<Buffer>,
        doc: Document,
        cx: &mut Context<Self>,
    ) {
        self.remote
            .as_mut()
            .unwrap()
            .revisions
            .insert(path, doc.revision);
        buffer.update(cx, |buffer, cx| {
            buffer.set_line_ending(language::LineEnding::detect(&doc.text), cx);
            buffer.set_text(doc.text, cx);
            buffer.did_save(buffer.version(), None, cx);
        });
    }
    fn remote_error(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        self.remote.as_mut().unwrap().client = None;
        this_error(self, error, cx);
    }
    fn start_remote_poll(&mut self, cx: &mut Context<Self>) {
        if self.remote.as_ref().unwrap().polling {
            return;
        }
        self.remote.as_mut().unwrap().polling = true;
        cx.spawn(async move |this,cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(3)).await;
                let work = this.update(cx, |this,cx| {
                    let remote=this.remote.as_ref()?;
                    let client=remote.client.clone()?;
                    let paths=if this.saving || !this.opening.is_empty() {vec![]} else {
                        this.tabs.iter().filter_map(|tab| Some((tab.path.clone(),this.remote_path(&tab.path).ok()?,tab.buffer.clone(),tab.buffer.read(cx).version(),remote.revisions.get(&tab.path)?.clone()))).collect()
                    };
                    Some((client,paths))
                });
                let Ok(Some((client,paths)))=work else {
                    this.update(cx, |this,_| {if let Some(remote)=&mut this.remote {remote.polling=false}}).ok();
                    break;
                };
                if paths.is_empty() {continue}
                let task=cx.background_executor().spawn(async move {
                    paths.into_iter().map(|(path,name,buffer,version,revision)| {
                        let doc=client.read(name);
                        (path,buffer,version,revision,doc)
                    }).collect::<Vec<_>>()
                });
                let changes=task.await;
                if this.update(cx, |this,cx| {
                    if this.saving || !this.opening.is_empty() {return}
                    for (path,buffer,version,revision,result) in changes {
                        if !this.tabs.iter().any(|tab|tab.path==path&&tab.buffer==buffer) {continue}
                        match result {
                            Ok(doc) if doc.revision!=revision => {
                                if buffer.read(cx).is_dirty() || buffer.read(cx).version()!=version {
                                    buffer.update(cx, |buffer,cx| {buffer.set_conflict();cx.emit(BufferEvent::DirtyChanged);cx.notify();});
                                } else {this.apply_remote_document(path,buffer,doc,cx);}
                            }
                            Ok(_)=>{},
                            Err(error)=>{this.remote_error(error.context("Remote file polling stopped; buffers kept. Refresh or reload to reconnect"),cx);break;}
                        }
                    }
                    cx.notify();
                }).is_err() {break}
            }
        }).detach();
    }
}
fn this_error(this: &mut FilesView, error: anyhow::Error, cx: &mut Context<FilesView>) {
    this.error = Some(format!("{error:#}"));
    cx.notify();
}

#[cfg(test)]
mod tests {
    include!("remote_tests.rs");
}
