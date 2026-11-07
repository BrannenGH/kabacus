// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

pub use kabacus_mcp::tools::*;

pub fn for_server(state: std::sync::Arc<crate::state::AppState>) -> KabacusMcp {
    KabacusMcp::new(
        state.service.clone(),
        None,
        Some(state.clone()),
        state.tool_lists.clone(),
    )
}
