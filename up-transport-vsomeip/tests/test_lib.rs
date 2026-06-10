/********************************************************************************
 * Copyright (c) 2023 Contributors to the Eclipse Foundation
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 ********************************************************************************/

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;
use tempfile::TempDir;
use up_rust::{UListener, UMessage};

static INIT: Once = Once::new();
static NEXT_NETWORK_ID: AtomicUsize = AtomicUsize::new(1);

pub fn before_test() {
    INIT.call_once(env_logger::init);
}

pub struct VsomeipTestNetwork {
    name: String,
}

pub struct IsolatedVsomeipConfig {
    _dir: TempDir,
    path: PathBuf,
}

impl VsomeipTestNetwork {
    pub fn new(test_name: &str) -> Self {
        let id = NEXT_NETWORK_ID.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let test_name = test_name
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect::<String>();
        Self {
            name: format!("up_vs_{pid}_{id}_{test_name}"),
        }
    }

    pub fn config(&self, app_name: &str, app_id: u16) -> IsolatedVsomeipConfig {
        self.config_with_services(app_name, app_id, &[])
    }

    pub fn config_with_services(
        &self,
        app_name: &str,
        app_id: u16,
        services: &[(u16, u16)],
    ) -> IsolatedVsomeipConfig {
        let dir = TempDir::new().expect("failed to create temporary vSomeIP config directory");
        let path = dir.path().join(format!("{app_name}.json"));
        let services_json = if services.is_empty() {
            String::new()
        } else {
            let entries = services
                .iter()
                .map(|(service, instance)| {
                    format!(r#"{{ "service": "0x{service:04x}", "instance": "0x{instance:04x}" }}"#)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(r#", "services": [{entries}]"#)
        };
        let config = format!(
            r#"{{
  "unicast": "127.0.0.1",
  "network": "{}",
  "applications": [{{ "name": "{}", "id": "0x{:04x}" }}]{}
}}
"#,
            self.name, app_name, app_id, services_json
        );
        fs::write(&path, config).expect("failed to write temporary vSomeIP config");
        IsolatedVsomeipConfig { _dir: dir, path }
    }
}

impl IsolatedVsomeipConfig {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub struct PrintingListener;
#[async_trait::async_trait]
impl UListener for PrintingListener {
    async fn on_receive(&self, msg: UMessage) {
        println!("{:?}", msg);
    }
}
