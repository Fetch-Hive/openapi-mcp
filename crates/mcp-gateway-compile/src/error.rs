use crate::loader::LoadError;
use crate::parse::ParseError;
use crate::safety::SafetyError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("failed to parse OpenAPI document: {0}")]
    Parse(String),
    #[error(transparent)]
    Load(#[from] LoadError),
    #[error(transparent)]
    Safety(#[from] SafetyError),
}

impl From<ParseError> for CompileError {
    fn from(value: ParseError) -> Self {
        match value {
            ParseError::Parse(msg) => Self::Parse(msg),
        }
    }
}

impl CompileError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Parse(_) => 1,
            Self::Load(e) => e.exit_code(),
            Self::Safety(_) => 3,
        }
    }

    /// Safety failures from a direct check or from the spec loader. The CLI
    /// maps these to its policy exit (2), not the compile-crate code 3.
    pub fn is_safety(&self) -> bool {
        matches!(self, Self::Safety(_) | Self::Load(LoadError::Safety(_)))
    }
}
