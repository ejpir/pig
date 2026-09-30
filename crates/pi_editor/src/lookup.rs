//! Project lookups for `@` mentions: workspace symbols and language-server problems.
//! Both need running language servers, so callers pass the project from
//! [`crate::language_project`].
use gpui::{App, Entity, Task};
use language::Point;
use lsp::DiagnosticSeverity;
use project::{Project, ProjectPath, lsp_store::SymbolLocation};
use std::{collections::HashMap, fmt::Write as _};

#[derive(Clone, Debug, PartialEq)]
pub struct SymbolHit {
    pub name: String,
    /// `const`, `function`, …
    pub kind: &'static str,
    /// Project-relative, with `/`.
    pub path: String,
    /// One-based.
    pub line: u32,
}

/// The language servers' workspace symbols matching `query`.
pub fn workspace_symbols(
    project: &Entity<Project>,
    query: &str,
    cx: &mut App,
) -> Task<Vec<SymbolHit>> {
    let task = project.update(cx, |p, cx| p.symbols(query, cx));
    cx.background_executor().spawn(async move {
        task.await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|symbol| {
                let SymbolLocation::InProject(path) = &symbol.path else {
                    return None;
                };
                Some(SymbolHit {
                    kind: kind_word(language::symbol_kind_to_lsp(symbol.kind)),
                    path: path.path.as_unix_str().to_owned(),
                    line: symbol.range.start.0.row + 1,
                    name: symbol.name,
                })
            })
            .collect()
    })
}

fn kind_word(kind: lsp::SymbolKind) -> &'static str {
    match kind {
        lsp::SymbolKind::FUNCTION => "function",
        lsp::SymbolKind::METHOD => "method",
        lsp::SymbolKind::CLASS => "class",
        lsp::SymbolKind::INTERFACE => "interface",
        lsp::SymbolKind::STRUCT => "struct",
        lsp::SymbolKind::ENUM => "enum",
        lsp::SymbolKind::CONSTANT => "const",
        lsp::SymbolKind::VARIABLE => "variable",
        lsp::SymbolKind::FIELD | lsp::SymbolKind::PROPERTY => "field",
        lsp::SymbolKind::MODULE | lsp::SymbolKind::NAMESPACE => "module",
        lsp::SymbolKind::TYPE_PARAMETER => "type",
        _ => "symbol",
    }
}

/// A file with language-server problems, from the servers' summaries.
#[derive(Clone, Debug, PartialEq)]
pub struct ProblemFile {
    pub path: ProjectPath,
    /// Project-relative, with `/`.
    pub relative: String,
    pub errors: usize,
    pub warnings: usize,
    /// The servers that reported them.
    pub servers: Vec<String>,
}

pub fn problem_files(project: &Entity<Project>, cx: &App) -> Vec<ProblemFile> {
    let project = project.read(cx);
    let names: HashMap<_, _> = project
        .language_server_statuses(cx)
        .map(|(id, status)| (id, status.name.to_string()))
        .collect();
    let mut files: Vec<ProblemFile> = Vec::new();
    for (path, server, summary) in project.diagnostic_summaries(false, cx) {
        if summary.error_count + summary.warning_count == 0 {
            continue;
        }
        let name = names.get(&server).cloned().unwrap_or_default();
        match files.iter_mut().find(|file| file.path == path) {
            Some(file) => {
                file.errors += summary.error_count;
                file.warnings += summary.warning_count;
                if !file.servers.contains(&name) {
                    file.servers.push(name);
                }
            }
            None => files.push(ProblemFile {
                relative: path.path.as_unix_str().to_owned(),
                path,
                errors: summary.error_count,
                warnings: summary.warning_count,
                servers: vec![name],
            }),
        }
    }
    files.sort_by(|a, b| b.errors.cmp(&a.errors).then(a.relative.cmp(&b.relative)));
    files
}

/// A file's errors and warnings as text for pi, one per line.
pub fn problems_text(project: &Entity<Project>, file: &ProblemFile, cx: &mut App) -> Task<String> {
    let open = project.update(cx, |p, cx| p.open_buffer(file.path.clone(), cx));
    let relative = file.relative.clone();
    cx.spawn(async move |cx| {
        let Ok(buffer) = open.await else {
            return format!("{relative}: could not be read");
        };
        buffer.read_with(cx, |buffer, _| {
            let snapshot = buffer.snapshot();
            let mut out = String::new();
            for entry in snapshot.diagnostics_in_range::<_, Point>(0..snapshot.len(), false) {
                let diagnostic = entry.diagnostic;
                let severity = match diagnostic.severity {
                    DiagnosticSeverity::ERROR => "error",
                    DiagnosticSeverity::WARNING => "warning",
                    _ => continue,
                };
                if !diagnostic.is_primary {
                    continue;
                }
                let message = diagnostic
                    .message
                    .as_ref()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim();
                let _ = write!(
                    out,
                    "{relative}:{}:{} {severity}: {message}",
                    entry.range.start.row + 1,
                    entry.range.start.column + 1,
                );
                if let Some(source) = &diagnostic.source {
                    let _ = write!(out, " ({source})");
                }
                out.push('\n');
            }
            out
        })
    })
}
