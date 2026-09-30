//! Which turn last wrote each line of a file, from jj's line annotation (`jj file
//! annotate`), for the editor's turn bars.
use std::collections::{HashMap, HashSet};

use jj_lib::annotate::FileAnnotator;
use jj_lib::repo_path::RepoPathBuf;

use super::*;

/// Operations searched for turns pi recorded.
const RECORDED_TURN_OPS: usize = 5000;

/// Which turn wrote each line of a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineTurns {
    /// Per line: the turn, or `None` for lines written by you, by other jj
    /// changes, or before the immutable history.
    pub lines: Vec<Option<ChangeId>>,
    /// Each turn's description: the prompt it ran.
    pub descriptions: HashMap<ChangeId, String>,
}

impl Project {
    /// For each line of `text`, the file as the editor has it now, the turn that
    /// wrote it: a change pi recorded (`pi: record turn …` in the operation log,
    /// any session).
    pub fn annotate(&mut self, path: &str, text: &str) -> Result<LineTurns> {
        self.repo = self.repo.reload_at_head().block_on()?;
        let wc = self
            .wc_commit()?
            .context("no working-copy commit in this workspace")?;
        let path = RepoPathBuf::from_internal_string(path)?;
        let repo = self.repo.as_ref();
        let domain = RevsetExpression::commit(wc.id().clone())
            .ancestors()
            .minus(&immutable_heads().ancestors());
        let no_extensions: [Box<dyn SymbolResolverExtension>; 0] = [];
        let domain =
            domain.resolve_user_expression(repo, &SymbolResolver::new(repo, &no_extensions))?;
        let mut annotator =
            FileAnnotator::with_file_content(wc.id(), &path, text.as_bytes().to_vec());
        annotator.compute(repo, &domain).block_on()?;
        let recorded = self.recorded_turns()?;
        let mut changes: HashMap<CommitId, Option<ChangeId>> = HashMap::new();
        let mut turns = LineTurns::default();
        for (origin, _) in annotator.to_annotation().lines() {
            let change = match origin {
                Ok(id) if id != wc.id() => match changes.get(id) {
                    Some(change) => change.clone(),
                    None => {
                        let commit = self.repo.store().get_commit(id)?;
                        let change = commit.change_id().clone();
                        let turn = recorded
                            .contains(&short_change_id(&change))
                            .then_some(change);
                        if let Some(turn) = &turn {
                            turns
                                .descriptions
                                .insert(turn.clone(), commit.description().trim_end().to_owned());
                        }
                        changes.insert(id.clone(), turn.clone());
                        turn
                    }
                },
                _ => None,
            };
            turns.lines.push(change);
        }
        Ok(turns)
    }

    /// Short IDs of the changes pi recorded as turns, from the operation log.
    fn recorded_turns(&self) -> Result<HashSet<String>> {
        let mut turns = HashSet::new();
        let mut operation = self.repo.operation().clone();
        for _ in 0..RECORDED_TURN_OPS {
            if let Some(short) = operation
                .metadata()
                .description
                .strip_prefix("pi: record turn ")
            {
                turns.insert(short.to_owned());
            }
            let Some(parent) = operation.parents().block_on()?.into_iter().next() else {
                break;
            };
            operation = parent;
        }
        Ok(turns)
    }
}
