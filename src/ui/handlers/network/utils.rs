// SPDX-FileCopyrightText: Copyright (c) 2026 owu <wqh@live.com>
// SPDX-License-Identifier: GPL-3.0-only

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use crate::{AppState, AppWindow};
use crate::network;
use slint::{ModelRc, SharedString, VecModel};
use crate::PortProxyRuleUI;

// Refreshes all network view data (distros, interfaces, rules, etc.)
pub async fn refresh_network_view_data(app: slint::Weak<AppWindow>, app_state: Arc<Mutex<AppState>>) {
    let (distros, interfaces, net_config) = {
        let state = app_state.lock().await;
        (
            state.wsl_dashboard.get_distros().await,
            network::scanner::get_physical_interfaces(),
            state.config_manager.get_network_config(),
        )
    };
    
    // Get active portproxy rules from Windows
    let active_ports = network::port_proxy::get_active_listen_ports().unwrap_or_default();

    let mut distro_names = Vec::new();
    for d in distros {
        distro_names.push(SharedString::from(d.name));
    }
    
    let mut local_ips = vec![SharedString::from("0.0.0.0")];
    let mut other_ips = Vec::new();
    let mut wsl_internal_ips = Vec::new();

    for iface in interfaces {
        if !iface.is_loopback && !iface.is_virtual {
            if iface.ip_address.starts_with("172.") && iface.ip_address.ends_with(".1") {
                continue;
            }
            if iface.ip_address.starts_with("172.") {
                wsl_internal_ips.push(SharedString::from(iface.ip_address.clone()));
            } else {
                other_ips.push(SharedString::from(iface.ip_address.clone()));
            }
        }
    }
    
    local_ips.extend(other_ips);
    local_ips.extend(wsl_internal_ips);
    
    let mut rule_uis = Vec::new();
    for r in &net_config.port_proxies {
        let is_active = active_ports.contains(&(r.listen_address.clone(), r.listen_port));
        rule_uis.push(PortProxyRuleUI {
            id: SharedString::from(&r.id),
            distro_name: SharedString::from(&r.distro_name),
            listen_address: SharedString::from(&r.listen_address),
            listen_port: SharedString::from(r.listen_port.to_string()),
            target_port: SharedString::from(r.target_port.to_string()),
            enable_firewall: r.enable_firewall,
            is_active,
        });
    }

    let proxy_config = net_config.proxy.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app) = app.upgrade() {
            let distro_model = ModelRc::new(VecModel::from(distro_names));
            app.set_network_distro_names(distro_model);
            
            let ip_model = ModelRc::new(VecModel::from(local_ips));
            app.set_network_local_ips(ip_model);
            
            let rule_model = ModelRc::new(VecModel::from(rule_uis));
            app.set_network_rules(rule_model);

            // Update networking mode from .wslconfig
            let mode = crate::utils::wsl_config::get_wsl_networking_mode().to_uppercase();
            app.set_network_networking_mode(mode.into());

            // Initialize proxy settings from configuration
            app.set_network_proxy_is_enabled(proxy_config.is_enabled);
            app.set_network_proxy_host(proxy_config.host.into());
            app.set_network_proxy_port(proxy_config.port.into());
            app.set_network_proxy_auth_enabled(proxy_config.auth_enabled);
            app.set_network_proxy_username(proxy_config.username.into());
            app.set_network_proxy_password(proxy_config.password.into());
            let no_proxy = if proxy_config.no_proxy.is_empty() { network::models::default_no_proxy() } else { proxy_config.no_proxy.clone() };
            app.set_network_proxy_no_proxy(no_proxy.into());
        }
    });
}

// Helper to show a task status toast with a 5-second auto-hide timer
pub fn show_toast(app: slint::Weak<AppWindow>, text: String) {
    info!("show_toast: '{}'", text);
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(app_instance) = app.upgrade() {
            app_instance.set_task_status_text(text.into());
            app_instance.set_task_status_visible(true);
            
            let ah_hide = app.clone();
            slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
                if let Some(app_h) = ah_hide.upgrade() {
                    app_h.set_task_status_visible(false);
                }
            });
        }
    });
}
