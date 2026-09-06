//! Topic latest offsets handler
//!
//! POST /v1/api/topics/latest-offsets - Resolve topic partition head offsets

use std::{collections::BTreeSet, sync::Arc};

use actix_web::{post, web, HttpResponse, Responder};
use kalamdb_auth::AuthSessionExtractor;
use kalamdb_commons::Role;
use kalamdb_core::app_context::AppContext;
use kalamdb_session::AuthSession;

use super::models::{
    LatestOffsetsRequest, LatestOffsetsResponse, TopicErrorResponse, TopicPartitionLatestOffset,
};

/// Check if role is allowed to resolve topic offsets.
/// Must be service, dba, or system role (NOT user)
fn is_topic_authorized(session: &AuthSession) -> bool {
    matches!(session.role(), Role::Service | Role::Dba | Role::System)
}

/// POST /v1/api/topics/latest-offsets - Resolve topic partition head offsets
///
/// # Authentication
/// Requires Bearer token authentication.
///
/// # Authorization
/// Role must be `service`, `dba`, or `system` (NOT `user`).
#[post("/latest-offsets")]
pub async fn latest_offsets_handler(
    extractor: AuthSessionExtractor,
    body: web::Json<LatestOffsetsRequest>,
    app_context: web::Data<Arc<AppContext>>,
) -> impl Responder {
    let session: AuthSession = extractor.into();

    if !is_topic_authorized(&session) {
        return HttpResponse::Forbidden().json(TopicErrorResponse::forbidden(
            "Topic offset inspection requires service, dba, or system role",
        ));
    }

    let topic_publisher = app_context.topic_publisher();
    let mut seen = BTreeSet::new();
    let mut offsets = Vec::with_capacity(body.partitions.len());

    for selector in &body.partitions {
        let dedupe_key = (selector.topic_id.to_string(), selector.partition_id);
        if !seen.insert(dedupe_key) {
            continue;
        }

        let last_offset =
            match topic_publisher.latest_offset(&selector.topic_id, selector.partition_id) {
                Ok(offset) => offset,
                Err(error) => {
                    return HttpResponse::InternalServerError().json(
                        TopicErrorResponse::internal_error(&format!(
                            "Failed to resolve latest offset: {}",
                            error
                        )),
                    );
                },
            };

        offsets.push(TopicPartitionLatestOffset {
            topic_id: selector.topic_id.clone(),
            partition_id: selector.partition_id,
            next_offset: last_offset.map(|offset| offset + 1).unwrap_or(0),
            last_offset,
        });
    }

    offsets.sort_by(|left, right| {
        let topic_compare = left.topic_id.to_string().cmp(&right.topic_id.to_string());
        if topic_compare == std::cmp::Ordering::Equal {
            left.partition_id.cmp(&right.partition_id)
        } else {
            topic_compare
        }
    });

    HttpResponse::Ok().json(LatestOffsetsResponse { offsets })
}
