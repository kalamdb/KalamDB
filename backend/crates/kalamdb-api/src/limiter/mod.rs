//! Rate limiting and connection guard module
//!
//! This module provides lightweight, zero-copy rate limiting for REST API and WebSocket
//! connections.
//!
//! ## Components
//!
//! - [`RateLimiter`]: User and connection-based rate limiting using atomic windows
//!
//! ## Design Principles
//!
//! - **Zero-copy**: Uses `Arc` for sharing without cloning data
//! - **Lock-free admission**: each request updates an atomic window and returns
//! - **Automatic cleanup**: TTL-based eviction eliminates manual cleanup overhead
//! - **Minimal allocations**: Keys are interned, values use interior mutability

mod rate_limiter;

pub use rate_limiter::RateLimiter;
