//! A rendered terminal plan and its masked preview.

use super::doc::DocFormat;
use crate::domain::Client;
use std::path::PathBuf;

/// One planned file operation: write `after`, or delete the file when it is
/// `None`. `before` is the exact content the plan was rendered from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub format: DocFormat,
    pub before: Option<String>,
    pub after: Option<String>,
    pub secrets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPlan {
    pub client: Client,
    pub config_path: PathBuf,
    pub endpoint: String,
    pub changes: Vec<FileChange>,
    /// Every managed file after rendering, including files that already match disk.
    pub rendered: Vec<(String, String)>,
    pub secrets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TerminalPreview {
    pub config_path: String,
    pub endpoint: String,
    pub files: Vec<TerminalPreviewFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TerminalPreviewFile {
    pub path: String,
    pub content: String,
}

impl TerminalPlan {
    /// The same content `apply` would write, with every sensitive value masked.
    pub fn preview(&self) -> TerminalPreview {
        let mut secrets: Vec<&str> = self
            .secrets
            .iter()
            .map(String::as_str)
            .chain(
                self.changes
                    .iter()
                    .flat_map(|change| change.secrets.iter().map(String::as_str)),
            )
            .filter(|secret| !secret.is_empty())
            .collect();
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();

        let rendered = if self.rendered.is_empty() {
            self.changes
                .iter()
                .filter_map(|change| {
                    Some((
                        change.path.display().to_string(),
                        change.after.clone()?,
                    ))
                })
                .collect()
        } else {
            self.rendered.clone()
        };
        let mut files: Vec<TerminalPreviewFile> = rendered
            .into_iter()
            .map(|(path, mut content)| {
                for secret in &secrets {
                    content = content.replace(secret, "••••••");
                }
                TerminalPreviewFile { path, content }
            })
            .collect();
        let config_path = self.config_path.display().to_string();
        files.sort_by_key(|file| file.path != config_path);
        TerminalPreview {
            config_path,
            endpoint: self.endpoint.clone(),
            files,
        }
    }
}
