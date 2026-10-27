//! In-memory idempotency receipts for `POST /v1/functions/{schema}/{name}`.
//!
//! A repeated call with the same user, routine, and `idempotency-key` returns the stored
//! response when the body digest matches. A different body for the same key is a conflict.
//! Receipts expire after ten minutes and the map is bounded.

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use actix_web::HttpResponse;
use kalamdb_serialization::sha256_digest;
use parking_lot::Mutex;

const RECEIPT_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_RECEIPTS: usize = 1_024;

static CLOCK: AtomicU64 = AtomicU64::new(1);
static RECEIPTS: Mutex<Option<HashMap<String, Receipt>>> = Mutex::new(None);

struct Receipt {
    digest: [u8; 32],
    stored: StoredResponse,
    expires: Instant,
    seq: u64,
}

/// Response bytes replayed for a matching idempotency key.
#[derive(Clone)]
pub(super) struct StoredResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl StoredResponse {
    pub(super) fn into_response(self) -> HttpResponse {
        let status = actix_web::http::StatusCode::from_u16(self.status)
            .unwrap_or(actix_web::http::StatusCode::OK);
        HttpResponse::build(status)
            .insert_header((actix_web::http::header::CONTENT_TYPE, self.content_type))
            .body(self.body)
    }
}

pub(super) enum IdempotencyLookup {
    Fresh,
    Replay(StoredResponse),
    Conflict,
}

pub(super) fn lookup(user_id: &str, routine_id: &str, key: &str, body: &[u8]) -> IdempotencyLookup {
    let map_key = receipt_key(user_id, routine_id, key);
    let digest = sha256_digest(body);
    let mut guard = RECEIPTS.lock();
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some(existing) = map.get(&map_key) {
        if existing.expires <= Instant::now() {
            map.remove(&map_key);
        } else if existing.digest != digest {
            return IdempotencyLookup::Conflict;
        } else {
            return IdempotencyLookup::Replay(existing.stored.clone());
        }
    }
    IdempotencyLookup::Fresh
}

pub(super) fn store(
    user_id: &str,
    routine_id: &str,
    key: &str,
    body: &[u8],
    stored: StoredResponse,
) {
    let map_key = receipt_key(user_id, routine_id, key);
    let mut guard = RECEIPTS.lock();
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() >= MAX_RECEIPTS {
        if let Some(oldest) =
            map.iter().min_by_key(|(_, receipt)| receipt.seq).map(|(k, _)| k.clone())
        {
            map.remove(&oldest);
        }
    }
    map.insert(
        map_key,
        Receipt {
            digest: sha256_digest(body),
            stored,
            expires: Instant::now() + RECEIPT_TTL,
            seq: CLOCK.fetch_add(1, Ordering::Relaxed),
        },
    );
}

fn receipt_key(user_id: &str, routine_id: &str, key: &str) -> String {
    let mut out = String::with_capacity(user_id.len() + routine_id.len() + key.len() + 2);
    out.push_str(user_id);
    out.push('\u{0}');
    out.push_str(routine_id);
    out.push('\u{0}');
    out.push_str(key);
    out
}
