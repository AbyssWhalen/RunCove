use crate::model::PortEntry;
use crate::render::Renderer;

pub struct JsonRenderer;

impl Renderer for JsonRenderer {
    fn render(&self, entries: &[PortEntry], _no_color: bool) -> String {
        serde_json::to_string_pretty(entries)
            .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize: {}\"}}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionState, Protocol};

    #[test]
    fn preserves_cli_json_schema() {
        let entry = PortEntry {
            port: 5173,
            protocol: Protocol::TCP,
            state: ConnectionState::Listen,
            pid: Some(42),
            process_name: Some("node.exe".into()),
            bind_address: "127.0.0.1".parse().unwrap(),
            is_public: false,
        };

        let value: serde_json::Value =
            serde_json::from_str(&JsonRenderer.render(&[entry], true)).unwrap();
        let object = value[0].as_object().unwrap();
        let mut keys: Vec<_> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "bind_address",
                "is_public",
                "pid",
                "port",
                "process_name",
                "protocol",
                "state",
            ]
        );
    }
}
