//! Bounds native allocations before walking guest-controlled values.

use super::conversion_limit::ConversionLimit;
use crate::{FunctionsError, Result};

pub(crate) struct ConversionBudget {
    remaining: usize,
}

impl ConversionBudget {
    pub(crate) fn new(scope: &v8::PinScope) -> Self {
        Self {
            remaining: scope.get_slot::<ConversionLimit>().map_or(8 * 1024 * 1024, |limit| limit.0),
        }
    }

    pub(crate) fn enter(&mut self, scope: &v8::PinScope, depth: usize) -> Result<()> {
        if scope.is_execution_terminating() {
            return Err(FunctionsError::Cancelled);
        }
        // Cyclic graphs also terminate at this bound, without risking native stack exhaustion.
        if depth >= 64 {
            return Err(FunctionsError::ResourceLimit("value nesting depth".into()));
        }
        self.charge(64)
    }

    pub(crate) fn items(&mut self, len: usize) -> Result<()> {
        self.charge(len.saturating_mul(256))
    }

    pub(crate) fn string(&mut self, value: v8::Local<v8::String>) -> Result<()> {
        self.charge(value.length().saturating_mul(3))
    }

    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or_else(|| FunctionsError::ResourceLimit("native value conversion bytes".into()))?;
        Ok(())
    }
}
