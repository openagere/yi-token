//! Terminal client support: documents, plans, atomic commits and client specs.

pub mod clients;
pub mod commit;
pub mod doc;
pub mod paths;
pub mod plan;
pub mod spec;

pub use commit::{AppliedPlan, apply};
pub use doc::{Doc, DocFormat};
pub use plan::{FileChange, TerminalPlan, TerminalPreview, TerminalPreviewFile};
pub use spec::{ClientCtx, ClientSpec, TerminalMode, build_plan};
