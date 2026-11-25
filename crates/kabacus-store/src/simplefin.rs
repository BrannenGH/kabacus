// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Credentials and request budgets kept in the live store.

use crate::{Result, StoreError, UserId};

pub const REQUEST_LIMIT: usize = 20;
const WINDOW: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    Local,
    Member(UserId),
}

impl Owner {
    pub(crate) fn user(self) -> Result<Option<UserId>> {
        match self {
            Self::Local => Ok(None),
            Self::Member(id) if id > 0 => Ok(Some(id)),
            Self::Member(_) => Err(StoreError::Invalid("Invalid SimpleFIN membership".into())),
        }
    }
}

#[derive(Clone)]
pub struct Access {
    url: String,
    pub claimed_at: i64,
    pub(crate) requests: Vec<i64>,
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Access")
            .field("claimed_at", &self.claimed_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestBudget {
    Allowed { remaining: usize },
    Exhausted { next_at: i64 },
    Changed,
    Deferred,
}

impl Access {
    pub fn new(url: String, claimed_at: i64) -> Self {
        Self {
            url,
            claimed_at,
            requests: Vec::new(),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn same_claim(&self, other: &Self) -> bool {
        self.claimed_at == other.claimed_at && self.url == other.url
    }

    pub fn remaining(&self, now: i64) -> usize {
        REQUEST_LIMIT.saturating_sub(
            self.requests
                .iter()
                .filter(|&&at| at > now.saturating_sub(WINDOW))
                .count(),
        )
    }

    pub(crate) fn stored(url: String, claimed_at: i64, timestamps: &str) -> Result<Self> {
        let requests = serde_json::from_str(timestamps)
            .map_err(|_| StoreError::Invalid("Invalid SimpleFIN request budget".into()))?;
        Ok(Self {
            url,
            claimed_at,
            requests,
        })
    }

    pub(crate) fn reserve(
        &mut self,
        expected: &Self,
        now: i64,
        recent_since: Option<i64>,
    ) -> RequestBudget {
        if !self.same_claim(expected) {
            return RequestBudget::Changed;
        }
        if recent_since.is_some_and(|since| self.requests.iter().any(|&at| at > since)) {
            return RequestBudget::Deferred;
        }
        self.requests.retain(|&at| at > now.saturating_sub(WINDOW));
        if self.requests.len() >= REQUEST_LIMIT {
            let oldest = self.requests.iter().copied().min().unwrap_or(now);
            return RequestBudget::Exhausted {
                next_at: oldest.saturating_add(WINDOW),
            };
        }
        self.requests.push(now);
        RequestBudget::Allowed {
            remaining: REQUEST_LIMIT - self.requests.len(),
        }
    }

    pub(crate) fn timestamps(&self) -> Result<String> {
        serde_json::to_string(&self.requests)
            .map_err(|_| StoreError::Invalid("Invalid SimpleFIN request budget".into()))
    }
}
