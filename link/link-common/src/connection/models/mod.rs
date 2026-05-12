//! Connection-level models: options, protocol messages, HTTP utilities.

pub mod connection_options;
pub mod server_message;

pub use connection_options::{ConnectionOptions, HttpVersion};
pub use kalamdb_commons::{
    ClientMessage, ClusterHealthResponse, ClusterNodeHealth, CompressionType, HealthCheckResponse,
    ProtocolOptions, SerializationType,
};
pub use server_message::ServerMessage;
