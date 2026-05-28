/// No-op span guard used when KalamDB traceability is compiled out.
#[derive(Debug, Default)]
pub struct NoopSpanGuard;

#[macro_export]
macro_rules! kdb_trace_span_entered {
    ($($span:tt)*) => {{
        #[cfg(feature = "traceability")]
        {
            tracing::trace_span!($($span)*).entered()
        }
        #[cfg(not(feature = "traceability"))]
        {
            let _ = stringify!($($span)*);
            $crate::NoopSpanGuard
        }
    }};
}

#[macro_export]
macro_rules! kdb_debug_span_entered {
    ($($span:tt)*) => {{
        #[cfg(feature = "traceability")]
        {
            tracing::debug_span!($($span)*).entered()
        }
        #[cfg(not(feature = "traceability"))]
        {
            let _ = stringify!($($span)*);
            $crate::NoopSpanGuard
        }
    }};
}

#[macro_export]
macro_rules! kdb_info_span_entered {
    ($($span:tt)*) => {{
        #[cfg(feature = "traceability")]
        {
            tracing::info_span!($($span)*).entered()
        }
        #[cfg(not(feature = "traceability"))]
        {
            let _ = stringify!($($span)*);
            $crate::NoopSpanGuard
        }
    }};
}

#[macro_export]
macro_rules! kdb_await_in_info_span {
    ($future:expr, $($span:tt)*) => {{
        #[cfg(feature = "traceability")]
        {
            use tracing::Instrument as _;
            ($future).instrument(tracing::info_span!($($span)*)).await
        }
        #[cfg(not(feature = "traceability"))]
        {
            let _ = stringify!($($span)*);
            ($future).await
        }
    }};
}

#[macro_export]
macro_rules! kdb_record_current_span {
    ($field:literal, $value:expr) => {{
        #[cfg(feature = "traceability")]
        {
            tracing::Span::current().record($field, $value);
        }
        #[cfg(not(feature = "traceability"))]
        {
            let _ = ($field, stringify!($value));
        }
    }};
}

#[macro_export]
macro_rules! kdb_trace {
    ($($event:tt)*) => {{
        #[cfg(feature = "traceability")]
        {
            tracing::trace!($($event)*);
        }
        #[cfg(not(feature = "traceability"))]
        {
            let _ = stringify!($($event)*);
        }
    }};
}

#[macro_export]
macro_rules! kdb_debug {
    ($($event:tt)*) => {{
        #[cfg(feature = "traceability")]
        {
            tracing::debug!($($event)*);
        }
        #[cfg(not(feature = "traceability"))]
        {
            let _ = stringify!($($event)*);
        }
    }};
}
