use serde::Serialize;

use crate::models::RelatedPort;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("{message}")]
    PortConflict {
        message: String,
        related_port: RelatedPort,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Invalid project manifest: {0}")]
    Manifest(#[from] serde_json::Error),
}

impl AppError {
    pub fn port_conflict(
        message: impl Into<String>,
        port: u16,
        protocol: impl Into<String>,
    ) -> Self {
        Self::PortConflict {
            message: message.into(),
            related_port: RelatedPort {
                port,
                protocol: protocol.into(),
            },
        }
    }

    pub fn related_port(&self) -> Option<&RelatedPort> {
        match self {
            Self::PortConflict { related_port, .. } => Some(related_port),
            _ => None,
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

pub fn invalid(message: impl Into<String>) -> AppError {
    AppError::Message(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_conflict_keeps_the_legacy_display_message_and_exposes_context() {
        let error = AppError::port_conflict(
            "Expected port 5173 is already occupied by node.exe",
            5173,
            "tcp",
        );

        assert_eq!(
            error.to_string(),
            "Expected port 5173 is already occupied by node.exe"
        );
        assert_eq!(
            error.related_port(),
            Some(&RelatedPort {
                port: 5173,
                protocol: "tcp".into(),
            })
        );
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            "\"Expected port 5173 is already occupied by node.exe\""
        );
    }
}
