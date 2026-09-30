//! Zed matching over an owned snapshot of this composer's reported candidates.
//! No Workspace, filesystem reads or Pi process is created here.
use super::{Item, Kind};
use fuzzy::{CharBag, PathMatchCandidate, PathMatchCandidateSet, StringMatchCandidate};
use gpui::BackgroundExecutor;
use std::sync::{Arc, atomic::AtomicBool};
use zed_path::{PathStyle, rel_path::RelPath};

struct PathCandidate {
    path: Arc<RelPath>,
    bag: CharBag,
    directory: bool,
}
struct PathSet(Vec<PathCandidate>);
fn candidate(entry: &PathCandidate) -> PathMatchCandidate<'_> {
    PathMatchCandidate {
        path: &entry.path,
        char_bag: entry.bag,
        is_dir: entry.directory,
    }
}
impl<'a> PathMatchCandidateSet<'a> for PathSet {
    type Candidates = std::iter::Map<
        std::slice::Iter<'a, PathCandidate>,
        fn(&'a PathCandidate) -> PathMatchCandidate<'a>,
    >;
    // A local collection key only, never a repository/history identity.
    fn id(&self) -> usize {
        0
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn root_is_file(&self) -> bool {
        false
    }
    fn prefix(&self) -> Arc<RelPath> {
        RelPath::empty_arc()
    }
    fn path_style(&self) -> PathStyle {
        if cfg!(target_os = "windows") {
            PathStyle::Windows
        } else {
            PathStyle::Unix
        }
    }
    fn candidates(&'a self, start: usize) -> Self::Candidates {
        self.0[start..].iter().map(candidate)
    }
}

pub(super) async fn filter(
    groups: Vec<Vec<Item>>,
    query: &str,
    cancel: &AtomicBool,
    executor: BackgroundExecutor,
) -> Vec<Item> {
    let mut output = Vec::new();
    for items in groups {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Vec::new();
        }
        let Some(first) = items.first() else {
            continue;
        };
        let paths = matches!(first.kind, Kind::File | Kind::Directory);
        let limit = if paths { super::FILES } else { super::OTHERS };
        // Empty query keeps the owner's preferred changed/recent ordering.
        if query.is_empty() {
            output.extend(items.into_iter().take(limit));
            continue;
        }
        if paths {
            let set = PathSet(
                items
                    .iter()
                    .filter_map(|item| {
                        let super::Body::Path { path, .. } = &item.choice.mention().body else {
                            return None;
                        };
                        let path = RelPath::from_unix_str(path.trim_end_matches('/')).ok()?;
                        Some(PathCandidate {
                            path: path.into(),
                            bag: path.as_unix_str().into(),
                            directory: item.kind == Kind::Directory,
                        })
                    })
                    .collect(),
            );
            let query = query
                .strip_prefix("./")
                .unwrap_or(query)
                .trim_end_matches('/');
            let matches = fuzzy::match_path_sets(
                &[set],
                query,
                &None,
                false,
                limit,
                cancel,
                executor.clone(),
            )
            .await;
            for matched in matches {
                let path = matched.path.as_unix_str();
                if let Some(item) = items.iter().find(|item| matches!(&item.choice.mention().body, super::Body::Path { path: candidate, .. } if candidate.trim_end_matches('/') == path)) {
                    let mut item = item.clone();
                    let basename = path.rfind('/').map_or(0, |at| at + 1);
                    item.matched = matched.positions.into_iter().filter_map(|at| at.checked_sub(basename)).collect();
                    output.push(item);
                }
            }
        } else {
            let candidates: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(id, item)| {
                    StringMatchCandidate::new(id, item.filter.as_deref().unwrap_or(&item.name))
                })
                .collect();
            let matches = fuzzy::match_strings(
                &candidates,
                query,
                false,
                true,
                limit,
                cancel,
                executor.clone(),
            )
            .await;
            for matched in matches.into_iter().take(limit) {
                let mut item = items[matched.candidate_id].clone();
                item.matched = matched
                    .positions
                    .into_iter()
                    .filter(|at| *at < item.name.len() && item.name.is_char_boundary(*at))
                    .collect();
                output.push(item);
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::super::{Body, Choice, Mention};
    use super::*;
    fn item(kind: Kind, label: &str, path: Option<&str>) -> Item {
        Item {
            kind,
            name: label.into(),
            matched: Vec::new(),
            detail: String::new(),
            filter: None,
            choice: Choice::Ready(Mention {
                kind,
                label: label.into(),
                body: path
                    .map(|path| Body::Path {
                        path: path.into(),
                        line: None,
                    })
                    .unwrap_or_else(|| Body::Text("original text".into())),
            }),
        }
    }
    #[gpui::test]
    fn zed_paths_include_directories_unicode_and_scattered_matches(cx: &mut gpui::TestAppContext) {
        let executor = cx.executor();
        let cancel = AtomicBool::new(false);
        let dirs = vec![item(Kind::Directory, "设计/", Some("docs/设计/"))];
        let matched = cx.foreground_executor().block_on(filter(
            vec![dirs],
            "./docs/设计/",
            &cancel,
            executor.clone(),
        ));
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].name, "设计/");
        assert!(
            matched[0]
                .matched
                .iter()
                .all(|at| matched[0].name.is_char_boundary(*at))
        );
        assert_eq!(matched[0].choice.mention().inline(), "@docs/设计/");
        let files = vec![item(
            Kind::File,
            "openai-completions.ts",
            Some("packages/ai/openai-completions.ts"),
        )];
        let matched = cx.foreground_executor().block_on(filter(
            vec![files],
            "oac",
            &cancel,
            executor.clone(),
        ));
        assert_eq!(matched.len(), 1);
        assert!(matched[0].matched.len() >= 3);
    }
    #[gpui::test]
    fn zed_strings_filter_all_metadata_groups_and_bound_empty_results(
        cx: &mut gpui::TestAppContext,
    ) {
        let executor = cx.executor();
        let cancel = AtomicBool::new(false);
        for kind in [
            Kind::Symbol,
            Kind::Turn,
            Kind::Session,
            Kind::Terminal,
            Kind::Problems,
        ] {
            let items = vec![
                item(kind, "parseSession", None),
                item(kind, "unrelated", None),
            ];
            let matched = cx.foreground_executor().block_on(filter(
                vec![items],
                "ps",
                &cancel,
                executor.clone(),
            ));
            assert_eq!(matched.len(), 1);
            assert_eq!(matched[0].name, "parseSession");
            assert_eq!(
                matched[0].choice.mention().body,
                Body::Text("original text".into())
            );
        }
        let items = (0..20)
            .map(|n| item(Kind::Session, &format!("Session {n}"), None))
            .collect();
        assert_eq!(
            cx.foreground_executor()
                .block_on(filter(vec![items], "", &cancel, executor.clone()))
                .len(),
            super::super::OTHERS
        );
    }
    #[gpui::test]
    fn cancelled_matching_never_returns_even_empty_query_candidates(cx: &mut gpui::TestAppContext) {
        let executor = cx.executor();
        let cancel = AtomicBool::new(true);
        assert!(
            cx.foreground_executor()
                .block_on(filter(
                    vec![vec![item(Kind::Session, "session", None)]],
                    "",
                    &cancel,
                    executor.clone()
                ))
                .is_empty()
        );
    }
}
