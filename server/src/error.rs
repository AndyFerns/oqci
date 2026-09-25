//! The server's own error type — wraps compiler errors, never redefines
//! them.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    Compile(#[from] oqci::compile::CompileError),
    #[error(transparent)]
    Ir(#[from] oqci::ir::IrError),
}
