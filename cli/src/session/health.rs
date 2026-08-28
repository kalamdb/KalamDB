use colored::Colorize;
use kalam_client::{ClusterHealthResponse, ClusterNodeHealth, KalamLinkClient, KalamLinkError};

use super::{CLISession, ClusterInfoDisplay, ClusterNodeDisplay};
use crate::Result;

impl CLISession {
    pub(in crate::session) fn normalize_server_field(value: String) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub(in crate::session) fn adopt_cluster_metadata(&mut self, info: &ClusterInfoDisplay) {
        self.cluster_name = Some(info.cluster_name.clone());

        if self.server_version.is_some() {
            return;
        }

        let version =
            info.current_node.as_ref().and_then(|node| node.version.clone()).or_else(|| {
                info.nodes
                    .iter()
                    .find(|node| node.is_self)
                    .and_then(|node| node.version.clone())
            });

        if let Some(version) = version.and_then(Self::normalize_server_field) {
            self.server_version = Some(version);
        }
    }

    pub(in crate::session) fn format_cluster_memory(memory_usage_mb: Option<u64>) -> String {
        memory_usage_mb.map_or_else(|| "memory n/a".to_string(), |mb| format!("{} MB", mb))
    }

    pub(in crate::session) fn format_cluster_cpu(cpu_usage_percent: Option<f32>) -> String {
        cpu_usage_percent
            .map(|cpu| format!("{:.1}% CPU", cpu))
            .unwrap_or_else(|| "CPU n/a".to_string())
    }

    pub(in crate::session) fn format_cluster_uptime(uptime_human: Option<&str>) -> String {
        uptime_human.unwrap_or("uptime n/a").to_string()
    }

    fn render_cluster_health_response(&self, health: &ClusterHealthResponse) {
        let active_nodes = health
            .nodes
            .iter()
            .filter(|node| node.status.eq_ignore_ascii_case("active"))
            .count();
        let offline_nodes = health
            .nodes
            .iter()
            .filter(|node| node.status.eq_ignore_ascii_case("offline"))
            .count();

        println!(
            "{} Cluster health: {}",
            if health.status.eq_ignore_ascii_case("healthy") {
                "✓".green()
            } else {
                "!".yellow()
            },
            health.status.green()
        );
        println!(
            "  Cluster: {} | Nodes: {} total, {} active, {} offline",
            health.cluster_id.as_str().cyan(),
            health.nodes.len(),
            active_nodes,
            offline_nodes
        );
        println!(
            "  Meta term: {} | Groups: {}/{}",
            health.current_term.to_string().cyan(),
            health.groups_leading.to_string().cyan(),
            health.total_groups.to_string().cyan()
        );
        println!();
        println!("{}", "Nodes:".yellow().bold());

        for node in &health.nodes {
            self.render_cluster_health_node(node);
        }
    }

    fn render_cluster_health_node(&self, node: &ClusterNodeHealth) {
        let self_marker = if node.is_self { " (connected)" } else { "" };
        let leader_marker = if node.is_leader { " [LEADER]" } else { "" };
        let hostname = node.hostname.as_deref().unwrap_or(node.api_addr.as_str());

        println!(
            "  Node {}: {} | {} | {}{}{}",
            node.node_id,
            node.role,
            node.status,
            node.api_addr,
            leader_marker.yellow(),
            self_marker.cyan()
        );
        println!(
            "           host={} | {} | {} | {}",
            hostname,
            Self::format_cluster_memory(node.memory_usage_mb),
            Self::format_cluster_cpu(node.cpu_usage_percent),
            Self::format_cluster_uptime(node.uptime_human.as_deref())
        );
    }

    pub(in crate::session) fn public_probe_client(&self) -> Result<KalamLinkClient> {
        Ok(KalamLinkClient::builder()
            .base_url(&self.server_url)
            .timeout(self.timeouts.receive_timeout)
            .max_retries(self.config.resolved_server().max_retries)
            .timeouts(self.timeouts.clone())
            .connection_options(self.config.to_connection_options())
            .build()?)
    }

    pub(in crate::session) async fn fetch_cluster_info(&self) -> Option<ClusterInfoDisplay> {
        let result = self
            .client
            .execute_query(
                "SELECT cluster_id, node_id, role, status, api_addr, is_self, is_leader, version \
                 FROM system.cluster ORDER BY is_leader DESC, node_id ASC",
                None,
                None,
                None,
            )
            .await;

        match result {
            Ok(response) => {
                let mut nodes = Vec::new();
                let mut current_node = None;
                let mut is_cluster_mode = false;
                let mut cluster_name = String::new();

                if let Some(query_result) = response.results.first() {
                    if let Some(rows) = &query_result.rows {
                        for row in rows {
                            if row.len() >= 8 {
                                if cluster_name.is_empty() {
                                    cluster_name =
                                        row[0].as_str().unwrap_or("standalone").to_string();
                                }
                                let node_id = row[1].as_u64().unwrap_or(0);
                                let role = row[2].as_str().unwrap_or("unknown").to_string();
                                let status = row[3].as_str().unwrap_or("unknown").to_string();
                                let api_addr = row[4].as_str().unwrap_or("").to_string();
                                let is_self = row[5].as_bool().unwrap_or(false);
                                let is_leader = row[6].as_bool().unwrap_or(false);
                                let version = row[7].as_str().map(ToString::to_string);

                                if matches!(
                                    role.as_str(),
                                    "leader" | "follower" | "learner" | "candidate"
                                ) {
                                    is_cluster_mode = true;
                                }

                                let node = ClusterNodeDisplay {
                                    node_id,
                                    role,
                                    status,
                                    api_addr,
                                    is_self,
                                    is_leader,
                                    version,
                                };

                                if is_self {
                                    current_node = Some(node.clone());
                                }
                                nodes.push(node);
                            }
                        }
                    }
                }

                if nodes.len() <= 1 && nodes.iter().any(|n| n.role == "standalone") {
                    is_cluster_mode = false;
                }

                Some(ClusterInfoDisplay {
                    is_cluster_mode,
                    cluster_name,
                    current_node,
                    nodes,
                })
            },
            Err(_) => None,
        }
    }

    /// Check server health and refresh cached server metadata.
    pub async fn health_check(&mut self) -> Result<()> {
        let probe_client = self.public_probe_client()?;
        let basic_health = probe_client.health_check().await;

        match &basic_health {
            Ok(health) => {
                self.connected = true;
                self.server_version = Self::normalize_server_field(health.version.clone());
                self.server_api_version = Self::normalize_server_field(health.api_version.clone());
                self.server_build_date =
                    health.build_date.clone().and_then(Self::normalize_server_field);
            },
            Err(KalamLinkError::ServerError {
                status_code: 403, ..
            }) => {
                self.connected = true;
            },
            Err(_) => {
                self.connected = false;
                self.server_version = None;
                self.server_api_version = None;
                self.server_build_date = None;
            },
        }

        match probe_client.cluster_health_check().await {
            Ok(cluster_health) => {
                self.connected = true;
                self.server_version = Self::normalize_server_field(cluster_health.version.clone());
                self.server_build_date =
                    Self::normalize_server_field(cluster_health.build_date.clone());
                self.render_cluster_health_response(&cluster_health);
                return Ok(());
            },
            Err(KalamLinkError::ServerError {
                status_code: 403, ..
            }) => match basic_health {
                Ok(_) => {
                    println!("✓ Server is healthy");
                    println!("  {}", "Cluster health endpoint is restricted to localhost".yellow());
                    return Ok(());
                },
                Err(KalamLinkError::ServerError {
                    status_code: 403, ..
                }) => {
                    println!(
                        "{}",
                        "Health endpoints are localhost-only for this connection".yellow()
                    );
                    println!("  {}", "No authenticated SQL fallback was used.".dimmed());
                    println!(
                        "  {}",
                        "Run SELECT * FROM system.cluster manually if you want authenticated \
                         cluster state."
                            .dimmed()
                    );
                    return Ok(());
                },
                Err(e) => return Err(e.into()),
            },
            Err(_) => {
                if basic_health.is_ok() {
                    println!("✓ Server is healthy");
                    return Ok(());
                }
            },
        }

        match basic_health {
            Ok(_) => {
                println!("✓ Server is healthy");
                Ok(())
            },
            Err(e) => {
                self.connected = false;
                self.server_version = None;
                self.server_api_version = None;
                self.server_build_date = None;
                Err(e.into())
            },
        }
    }
}
