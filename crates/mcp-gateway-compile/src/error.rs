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
}
