pub mod config;
pub mod guidance;
pub mod identifiers;
pub mod init;
pub mod link;
pub mod prompts;
pub mod resolve;
pub mod scaffold;
pub mod status;
pub mod templates;
pub mod ts;

pub use config::KalamProjectConfig;
pub use identifiers::{
    normalize_namespace_name, parse_namespace_id, parse_table_id, parse_table_name,
    parse_table_ref, parse_user_id, preferred_user_label,
};
pub use link::{link_environment, LinkOptions};
pub use status::{collect_status, show_status};
