//! Project choice happens after SSH connection. Browsing is read-only and
//! independent of session creation; stale responses cannot select a folder.

use crate::{
    app::{PhoneApp, Route},
    remote,
};
use gpui::Context;

#[derive(Default)]
pub(crate) struct ProjectBrowser {
    pub directory: Option<remote::Directory>,
    pub loading: bool,
    pub error: Option<String>,
    pub manual: bool,
    pub show_hidden: bool,
    pub requested: Option<String>,
    generation: u64,
}

impl ProjectBrowser {
    pub fn clear(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            ..Self::default()
        };
    }

    fn begin(&mut self, path: String) -> u64 {
        self.generation += 1;
        self.loading = true;
        self.error = None;
        self.requested = Some(path);
        self.generation
    }

    fn complete(&mut self, generation: u64, result: Result<remote::Directory, String>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.loading = false;
        match result {
            Ok(directory) => self.directory = Some(directory),
            Err(error) => self.error = Some(error),
        }
        true
    }
}

impl PhoneApp {
    pub(crate) fn browse_projects(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let Some(store) = &self.store else { return };
        let Some(live) = &store.live else {
            let path = path.unwrap_or_else(|| "/Users/nick/repos".into());
            let entries = if path == "/Users/nick/repos" {
                store
                    .projects
                    .iter()
                    .map(|project| remote::Folder {
                        name: project.name.clone(),
                        path: project.path.replacen('~', "/Users/nick", 1),
                        project: true,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            self.project_browser.directory = Some(remote::Directory {
                version: 1,
                parent: std::path::Path::new(&path)
                    .parent()
                    .map(|p| p.display().to_string()),
                path,
                entries,
                truncated: false,
            });
            self.project_browser.loading = false;
            self.project_browser.error = None;
            cx.notify();
            return;
        };
        let connection = live.connection.clone();
        let helper = live.helper.clone();
        let path = path.unwrap_or_else(|| helper.home.clone());
        let show_hidden = self.project_browser.show_hidden;
        let generation = self.project_browser.begin(path.clone());
        cx.spawn(async move |this, cx| {
            let result = remote::directories(&connection, &helper, &path, show_hidden)
                .await
                .map_err(|error| format!("{error:#}"));
            this.update(cx, |this, cx| {
                if this.project_browser.complete(generation, result) {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn select_project(&mut self, index: usize, cx: &mut Context<Self>) {
        if self
            .store
            .as_ref()
            .is_none_or(|store| index >= store.projects.len())
        {
            return;
        }
        self.project = index;
        if let Some(store) = &self.store {
            let address = store.computer.address.clone();
            let path = store.projects[index].path.clone();
            if !store.is_sample() {
                self.update_prefs(cx, |prefs| {
                    prefs.projects.insert(address, path);
                });
            }
        }
        if self.route() == Route::Projects {
            self.routes = vec![Route::Sessions, Route::Start];
        }
        self.close_sheet(cx);
        cx.notify();
    }

    pub(crate) fn use_browsed_project(&mut self, cx: &mut Context<Self>) {
        if self.project_browser.loading || self.project_browser.error.is_some() {
            return;
        }
        let Some(path) = self
            .project_browser
            .directory
            .as_ref()
            .map(|d| d.path.clone())
        else {
            return;
        };
        if let Some(store) = &mut self.store {
            let index = store.add_project(&path);
            self.select_project(index, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(path: &str) -> remote::Directory {
        remote::Directory {
            version: 1,
            path: path.into(),
            parent: None,
            entries: Vec::new(),
            truncated: false,
        }
    }

    #[test]
    fn later_navigation_and_computer_changes_invalidate_old_results() {
        let mut browser = ProjectBrowser::default();
        let old = browser.begin("/first".into());
        let latest = browser.begin("/second".into());
        assert!(!browser.complete(old, Ok(listing("/first"))));
        assert!(browser.loading);
        assert!(browser.complete(latest, Ok(listing("/second"))));
        assert_eq!(browser.directory.as_ref().unwrap().path, "/second");
        let pending = browser.begin("/third".into());
        browser.clear();
        assert!(!browser.complete(pending, Ok(listing("/third"))));
        assert!(browser.directory.is_none());
    }

    #[test]
    fn failed_navigation_keeps_location_and_exposes_the_error() {
        let mut browser = ProjectBrowser::default();
        let request = browser.begin("/project".into());
        browser.complete(request, Ok(listing("/project")));
        let request = browser.begin("/missing".into());
        browser.complete(request, Err("Folder does not exist".into()));
        assert!(!browser.loading);
        assert_eq!(browser.directory.unwrap().path, "/project");
        assert!(browser.error.is_some());
    }
}
