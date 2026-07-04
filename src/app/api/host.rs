use std::time::Instant;

use crate::api::schema::{
    HostCloseAction, HostCloseResult, HostNavigateParams, HostNavigateResult,
    HostPrepareEntryParams, HostPrepareEntryResult, PaneDirection, PaneTarget, ResponseResult,
    TabTarget,
};
use crate::app::{App, Mode};
use crate::layout::{find_in_direction, PaneId};

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_host_navigate(
        &mut self,
        id: String,
        params: HostNavigateParams,
    ) -> String {
        let Some((ws_idx, tab_idx, source_pane_id)) = self.host_active_pane() else {
            return encode_error(id, "pane_not_found", "pane not found");
        };
        let target =
            self.host_directional_pane_target(ws_idx, tab_idx, source_pane_id, params.direction);
        let changed = if let Some(target_pane_id) = target {
            self.state.focus_pane_in_workspace(ws_idx, target_pane_id)
        } else {
            false
        };
        if target.is_some() {
            self.state.mode = Mode::Terminal;
        }
        let focused_pane_id = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.tabs.get(tab_idx))
            .map(|tab| tab.layout.focused())
            .and_then(|pane_id| self.public_pane_id(ws_idx, pane_id));

        encode_success(
            id,
            ResponseResult::HostNavigate {
                navigate: HostNavigateResult {
                    changed,
                    at_edge: target.is_none(),
                    focused_pane_id,
                },
            },
        )
    }

    pub(super) fn handle_host_prepare_entry(
        &mut self,
        id: String,
        params: HostPrepareEntryParams,
    ) -> String {
        self.state
            .prepare_host_entry(params.direction.into(), Instant::now());
        encode_success(
            id,
            ResponseResult::HostPrepareEntry {
                entry: HostPrepareEntryResult { armed: true },
            },
        )
    }

    pub(super) fn handle_host_close(&mut self, id: String) -> String {
        let Some(ws_idx) = self.state.active else {
            return self.encode_host_close(id, HostCloseAction::CloseHost);
        };
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return self.encode_host_close(id, HostCloseAction::CloseHost);
        };
        let pane_count = ws
            .active_tab()
            .map(|tab| tab.layout.pane_count())
            .unwrap_or_default();
        if pane_count > 1 {
            let Some(pane_id) = ws.focused_pane_id() else {
                return self.encode_host_close(id, HostCloseAction::Noop);
            };
            let Some(public_pane_id) = self.public_pane_id(ws_idx, pane_id) else {
                return encode_error(id, "pane_not_found", "pane not found");
            };
            let target = PaneTarget {
                pane_id: public_pane_id,
            };
            if let Err(response) = self.close_pane(id.clone(), &target) {
                return response;
            }
            return self.encode_host_close(id, HostCloseAction::ClosePane);
        }
        if ws.tabs.len() > 1 {
            let Some(tab_id) = self.public_tab_id(ws_idx, ws.active_tab_index()) else {
                return encode_error(id, "tab_not_found", "tab not found");
            };
            let target = TabTarget { tab_id };
            if let Err(response) = self.close_tab(id.clone(), &target) {
                return response;
            }
            return self.encode_host_close(id, HostCloseAction::CloseTab);
        }

        self.encode_host_close(id, HostCloseAction::CloseHost)
    }

    fn encode_host_close(&self, id: String, action: HostCloseAction) -> String {
        encode_success(
            id,
            ResponseResult::HostClose {
                close: HostCloseResult { action },
            },
        )
    }

    fn host_active_pane(&self) -> Option<(usize, usize, PaneId)> {
        let ws_idx = self.state.active?;
        let ws = self.state.workspaces.get(ws_idx)?;
        let tab_idx = ws.active_tab_index();
        let pane_id = ws.tabs.get(tab_idx)?.layout.focused();
        Some((ws_idx, tab_idx, pane_id))
    }

    fn host_directional_pane_target(
        &self,
        ws_idx: usize,
        tab_idx: usize,
        source_pane_id: PaneId,
        direction: PaneDirection,
    ) -> Option<PaneId> {
        let tab = self.state.workspaces.get(ws_idx)?.tabs.get(tab_idx)?;
        let panes = tab.layout.panes(self.state.view.terminal_area);
        let source = panes.iter().find(|pane| pane.id == source_pane_id)?;
        find_in_direction(source, direction.into(), &panes)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ratatui::layout::{Direction, Rect};

    use super::*;
    use crate::api::schema::{EmptyParams, Method, Request, SuccessResponse};
    use crate::app::App;
    use crate::config::Config;
    use crate::layout::NavDirection;
    use crate::workspace::Workspace;

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("host")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app
    }

    #[test]
    fn host_navigate_focuses_internal_neighbor() {
        let mut app = test_app();
        let root = app.state.workspaces[0].tabs[0].root_pane;
        let right = app.state.workspaces[0].test_split(Direction::Horizontal);
        app.state.workspaces[0].tabs[0].layout.focus_pane(root);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 100, 20));
        let right_public = app.public_pane_id(0, right).unwrap();

        let response = app.handle_host_navigate(
            "req".into(),
            HostNavigateParams {
                direction: PaneDirection::Right,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::HostNavigate { navigate } = success.result else {
            panic!("expected host navigate response");
        };
        assert!(navigate.changed);
        assert!(!navigate.at_edge);
        assert_eq!(navigate.focused_pane_id, Some(right_public));
        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(right));
    }

    #[test]
    fn host_navigate_reports_edge_without_changing_focus() {
        let mut app = test_app();
        let root = app.state.workspaces[0].tabs[0].root_pane;
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 100, 20));
        let root_public = app.public_pane_id(0, root).unwrap();

        let response = app.handle_host_navigate(
            "req".into(),
            HostNavigateParams {
                direction: PaneDirection::Right,
            },
        );

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::HostNavigate { navigate } = success.result else {
            panic!("expected host navigate response");
        };
        assert!(!navigate.changed);
        assert!(navigate.at_edge);
        assert_eq!(navigate.focused_pane_id, Some(root_public));
        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(root));
    }

    #[test]
    fn host_entry_intent_focuses_entry_edge_pane() {
        let mut app = test_app();
        let root = app.state.workspaces[0].tabs[0].root_pane;
        let right = app.state.workspaces[0].test_split(Direction::Horizontal);
        app.state.workspaces[0].tabs[0].layout.focus_pane(root);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 100, 20));
        let now = Instant::now();

        app.state.prepare_host_entry(NavDirection::Left, now);
        assert!(app
            .state
            .consume_host_entry_intent(now + Duration::from_millis(50)));

        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(right));
        assert!(app.state.suppress_next_host_entry_mouse_focus);
    }

    #[test]
    fn stale_host_entry_intent_is_ignored() {
        let mut app = test_app();
        let root = app.state.workspaces[0].tabs[0].root_pane;
        app.state.workspaces[0].test_split(Direction::Horizontal);
        app.state.workspaces[0].tabs[0].layout.focus_pane(root);
        crate::ui::compute_view(&mut app.state, Rect::new(0, 0, 100, 20));
        let now = Instant::now();

        app.state.prepare_host_entry(NavDirection::Left, now);
        assert!(!app
            .state
            .consume_host_entry_intent(now + Duration::from_secs(2)));

        assert_eq!(app.state.workspaces[0].focused_pane_id(), Some(root));
        assert!(!app.state.suppress_next_host_entry_mouse_focus);
    }

    #[test]
    fn host_close_closes_pane_tab_or_host_by_scope() {
        let mut app = test_app();
        app.state.workspaces[0].test_split(Direction::Horizontal);

        let response = app.handle_host_close("close_pane".into());
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::HostClose { close } = success.result else {
            panic!("expected host close response");
        };
        assert_eq!(close.action, HostCloseAction::ClosePane);
        assert_eq!(app.state.workspaces[0].tabs[0].layout.pane_count(), 1);

        app.state.workspaces[0].test_add_tab(Some("logs"));
        let response = app.handle_host_close("close_tab".into());
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::HostClose { close } = success.result else {
            panic!("expected host close response");
        };
        assert_eq!(close.action, HostCloseAction::CloseTab);
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);

        let response = app.handle_host_close("close_host".into());
        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::HostClose { close } = success.result else {
            panic!("expected host close response");
        };
        assert_eq!(close.action, HostCloseAction::CloseHost);
        assert_eq!(app.state.workspaces.len(), 1);
    }

    #[test]
    fn host_requests_round_trip() {
        let request = Request {
            id: "req".into(),
            method: Method::HostPrepareEntry(HostPrepareEntryParams {
                direction: PaneDirection::Left,
            }),
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(encoded.contains("\"method\":\"host.prepare_entry\""));
        let decoded: Request = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, request);

        let request = Request {
            id: "req".into(),
            method: Method::HostClose(EmptyParams::default()),
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(encoded.contains("\"method\":\"host.close\""));
    }
}
